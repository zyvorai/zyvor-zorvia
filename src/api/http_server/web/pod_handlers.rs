// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Cluster-wide pod inventory, streaming logs and interactive exec.
//!
//! Every endpoint here is `cluster.admin`: the REST routes via
//! `required_permission` (`/v1/pods`, `/v1/namespaces`), and the `/ws/pods/*`
//! sockets via `authorize_permission` because `/ws/*` bypasses the `/api`
//! RBAC layer.

use super::ws_proxy_handlers::authorize_permission;
use super::*;
use crate::api::auth::permissions::ApiPermission;
use axum::extract::{
    ws::{Message, WebSocket},
    WebSocketUpgrade,
};
use futures_util::{SinkExt, StreamExt};
use k8s_openapi::api::core::v1::{ContainerStatus, Namespace, Pod};
use kube::api::{Api, AttachParams, ListParams, LogParams, TerminalSize};
use std::time::{Duration, Instant};

const MAX_TAIL_LINES: i64 = 10_000;

/// Kubernetes object names (DNS-1123 subdomain) — rejects path tricks before
/// the value reaches an API URL.
fn valid_k8s_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !s.starts_with(['-', '.'])
        && !s.ends_with(['-', '.'])
}

fn flag(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "true" | "yes"))
}

#[derive(Debug, Serialize)]
pub struct ContainerSummary {
    pub name: String,
    pub image: String,
    pub init: bool,
    pub ready: bool,
    pub restarts: i32,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub status: String,
    pub ready: String,
    pub restarts: i32,
    pub created: Option<String>,
    pub age_seconds: Option<i64>,
    pub node: Option<String>,
    pub pod_ip: Option<String>,
    pub owner_kind: Option<String>,
    pub containers: Vec<ContainerSummary>,
}

fn container_state(cs: &ContainerStatus) -> String {
    let Some(state) = cs.state.as_ref() else {
        return "Unknown".into();
    };
    if let Some(w) = &state.waiting {
        return w.reason.clone().unwrap_or_else(|| "Waiting".into());
    }
    if let Some(t) = &state.terminated {
        return t.reason.clone().unwrap_or_else(|| "Terminated".into());
    }
    if state.running.is_some() {
        return "Running".into();
    }
    "Unknown".into()
}

fn summarize(pod: Pod) -> PodSummary {
    let meta = pod.metadata;
    let spec = pod.spec.unwrap_or_default();
    let status = pod.status.unwrap_or_default();
    let phase = status.phase.clone().unwrap_or_else(|| "Unknown".into());

    let statuses = status.container_statuses.clone().unwrap_or_default();
    let init_statuses = status.init_container_statuses.clone().unwrap_or_default();

    let mut containers = Vec::new();
    for (init, specs, sts) in [
        (
            true,
            spec.init_containers.clone().unwrap_or_default(),
            &init_statuses,
        ),
        (false, spec.containers.clone(), &statuses),
    ] {
        for c in specs {
            let st = sts.iter().find(|s| s.name == c.name);
            containers.push(ContainerSummary {
                image: c.image.clone().unwrap_or_default(),
                init,
                ready: st.is_some_and(|s| s.ready),
                restarts: st.map_or(0, |s| s.restart_count),
                state: st.map_or_else(|| "Pending".into(), container_state),
                name: c.name,
            });
        }
    }

    let ready_count = statuses.iter().filter(|s| s.ready).count();
    let restarts = statuses.iter().map(|s| s.restart_count).sum();

    // kubectl-style display status: a waiting/terminated reason on any main
    // container (CrashLoopBackOff, ImagePullBackOff, Error…) beats the phase.
    let mut display = phase.clone();
    for cs in &statuses {
        let s = container_state(cs);
        if s != "Running" && s != "Completed" && s != "Unknown" {
            display = s;
            break;
        }
    }
    if meta.deletion_timestamp.is_some() {
        display = "Terminating".into();
    }

    let created = meta.creation_timestamp.as_ref().map(|t| t.0);
    PodSummary {
        name: meta.name.unwrap_or_default(),
        namespace: meta.namespace.unwrap_or_default(),
        phase,
        status: display,
        ready: format!("{ready_count}/{}", spec.containers.len()),
        restarts,
        created: created.map(|t| t.to_string()),
        age_seconds: created
            .map(|t| (k8s_openapi::jiff::Timestamp::now().as_second() - t.as_second()).max(0)),
        node: spec.node_name,
        pod_ip: status.pod_ip,
        owner_kind: meta
            .owner_references
            .and_then(|o| o.into_iter().next().map(|r| r.kind)),
        containers,
    }
}

#[derive(Debug, Deserialize)]
pub struct PodListQuery {
    pub namespace: Option<String>,
}

pub async fn list_pods_handler(
    State(state): State<SharedState>,
    Query(q): Query<PodListQuery>,
) -> impl IntoResponse {
    let client = state.read().await.kube_client.client();
    let ns = q.namespace.filter(|n| !n.is_empty() && n != "all");
    if let Some(ns) = ns.as_deref() {
        if !valid_k8s_name(ns) {
            let (st, j) = err_json(400, "INVALID", "invalid namespace");
            return (st, j).into_response();
        }
    }
    let api: Api<Pod> = match ns.as_deref() {
        Some(ns) => Api::namespaced(client, ns),
        None => Api::all(client),
    };
    match api.list(&ListParams::default()).await {
        Ok(list) => {
            let mut pods: Vec<PodSummary> = list.items.into_iter().map(summarize).collect();
            pods.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
            let count = pods.len();
            Json(serde_json::json!({ "pods": pods, "count": count })).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn list_namespaces_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let client = state.read().await.kube_client.client();
    let api: Api<Namespace> = Api::all(client);
    match api.list(&ListParams::default()).await {
        Ok(list) => {
            let mut names: Vec<String> = list
                .items
                .into_iter()
                .filter_map(|n| n.metadata.name)
                .collect();
            names.sort();
            Json(serde_json::json!({ "namespaces": names })).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

/// What the Zorvia service account may do to pods cluster-wide. Lets the UI
/// disable actions whose RBAC rule is missing (e.g. after an upgrade that
/// didn't re-apply the ClusterRole) instead of failing on click.
#[derive(Debug, Serialize)]
pub struct PodCapabilities {
    pub logs: bool,
    pub exec: bool,
    pub delete: bool,
}

async fn can_i(client: &kube::Client, verb: &str, subresource: Option<&str>) -> bool {
    use k8s_openapi::api::authorization::v1::{
        ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    };
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                verb: Some(verb.into()),
                resource: Some("pods".into()),
                subresource: subresource.map(Into::into),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    match api.create(&Default::default(), &review).await {
        Ok(r) => r.status.is_some_and(|s| s.allowed),
        // Unknown ≠ denied: keep the action enabled; the kube call itself still enforces RBAC.
        Err(e) => {
            log::warn!("pods capability check ({verb} {subresource:?}) failed: {e}");
            true
        }
    }
}

pub async fn pod_capabilities_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let client = state.read().await.kube_client.client();
    let (logs, exec, delete) = tokio::join!(
        can_i(&client, "get", Some("log")),
        can_i(&client, "create", Some("exec")),
        can_i(&client, "delete", None),
    );
    Json(PodCapabilities { logs, exec, delete })
}

#[allow(clippy::too_many_arguments)]
async fn audit_pod(
    audit: &SharedAuditTrail,
    user: &str,
    action: crate::audit_trail::AuditAction,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
    success: bool,
    details: serde_json::Value,
) {
    let entry = crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", pod),
        timestamp: chrono::Utc::now(),
        user: user.to_string(),
        severity: if matches!(action, crate::audit_trail::AuditAction::Exec) {
            crate::audit_trail::AuditSeverity::High
        } else {
            crate::audit_trail::AuditSeverity::Info
        },
        action,
        resource_type: "pod".into(),
        resource_name: match container {
            Some(c) => format!("{pod}/{c}"),
            None => pod.to_string(),
        },
        namespace: namespace.to_string(),
        details,
        ip_address: String::new(),
        success,
    };
    audit.write().await.record(entry);
}

// ── Pod actions, events, YAML ───────────────────────────────────────

fn invalid_names(namespace: &str, name: &str) -> Option<axum::response::Response> {
    if valid_k8s_name(namespace) && valid_k8s_name(name) {
        return None;
    }
    let (st, j) = err_json(400, "INVALID", "invalid namespace or pod name");
    Some((st, j).into_response())
}

fn kube_err(e: &kube::Error) -> axum::response::Response {
    let (status, code) = match e {
        kube::Error::Api(s) if s.code == 404 => (404, "NOT_FOUND"),
        kube::Error::Api(s) if s.code == 403 => (403, "FORBIDDEN"),
        _ => (500, "KUBE_ERROR"),
    };
    let (st, j) = err_json(status, code, &sanitize_error(e));
    (st, j).into_response()
}

fn caller(auth: &Option<axum::Extension<crate::api::auth::AuthIdentity>>) -> String {
    auth.as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into())
}

#[derive(Debug, Deserialize)]
pub struct DeletePodQuery {
    /// Grace period in seconds; `0` force-deletes.
    pub grace: Option<u32>,
}

pub async fn delete_pod_handler(
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
    Query(q): Query<DeletePodQuery>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
) -> axum::response::Response {
    if let Some(r) = invalid_names(&namespace, &name) {
        return r;
    }
    let s = state.read().await;
    let client = s.kube_client.client();
    let audit = s.audit.clone();
    drop(s);

    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let dp = kube::api::DeleteParams {
        grace_period_seconds: q.grace,
        ..Default::default()
    };
    let result = api.delete(&name, &dp).await;
    audit_pod(
        &audit,
        &caller(&auth),
        crate::audit_trail::AuditAction::Delete,
        &namespace,
        &name,
        None,
        result.is_ok(),
        serde_json::json!({ "grace": q.grace }),
    )
    .await;
    match result {
        Ok(_) => Json(serde_json::json!({ "deleted": true, "namespace": namespace, "name": name }))
            .into_response(),
        Err(e) => kube_err(&e),
    }
}

/// Kinds whose controller recreates a deleted pod.
fn restartable_owner(kind: &str) -> bool {
    matches!(
        kind,
        "ReplicaSet" | "StatefulSet" | "DaemonSet" | "ReplicationController"
    )
}

pub async fn restart_pod_handler(
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
) -> axum::response::Response {
    if let Some(r) = invalid_names(&namespace, &name) {
        return r;
    }
    let s = state.read().await;
    let client = s.kube_client.client();
    let audit = s.audit.clone();
    drop(s);

    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let pod = match api.get(&name).await {
        Ok(p) => p,
        Err(e) => return kube_err(&e),
    };
    let owner = pod
        .metadata
        .owner_references
        .unwrap_or_default()
        .into_iter()
        .find(|o| o.controller == Some(true));
    let Some(owner) = owner.filter(|o| restartable_owner(&o.kind)) else {
        let (st, j) = err_json(
            409,
            "NOT_RESTARTABLE",
            "pod is not managed by a ReplicaSet, StatefulSet, DaemonSet or ReplicationController; \
             deleting it would not bring it back",
        );
        return (st, j).into_response();
    };

    let result = api.delete(&name, &kube::api::DeleteParams::default()).await;
    audit_pod(
        &audit,
        &caller(&auth),
        crate::audit_trail::AuditAction::Restart,
        &namespace,
        &name,
        None,
        result.is_ok(),
        serde_json::json!({ "owner_kind": owner.kind, "owner": owner.name }),
    )
    .await;
    match result {
        Ok(_) => Json(serde_json::json!({
            "restarted": true,
            "owner_kind": owner.kind,
            "owner": owner.name,
        }))
        .into_response(),
        Err(e) => kube_err(&e),
    }
}

#[derive(Debug, Serialize)]
pub struct PodEvent {
    pub r#type: String,
    pub reason: String,
    pub message: String,
    pub count: i32,
    pub source: String,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

pub async fn pod_events_handler(
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
) -> axum::response::Response {
    use k8s_openapi::api::core::v1::Event;
    if let Some(r) = invalid_names(&namespace, &name) {
        return r;
    }
    let client = state.read().await.kube_client.client();
    let api: Api<Event> = Api::namespaced(client, &namespace);
    let lp = ListParams::default().fields(&format!(
        "involvedObject.kind=Pod,involvedObject.name={name}"
    ));
    match api.list(&lp).await {
        Ok(list) => {
            let mut events: Vec<PodEvent> = list
                .items
                .into_iter()
                .map(|e| {
                    let last = e
                        .last_timestamp
                        .map(|t| t.0.to_string())
                        .or_else(|| e.event_time.map(|t| t.0.to_string()));
                    PodEvent {
                        r#type: e.type_.unwrap_or_else(|| "Normal".into()),
                        reason: e.reason.unwrap_or_default(),
                        message: e.message.unwrap_or_default(),
                        count: e.count.unwrap_or(1),
                        source: e
                            .source
                            .and_then(|s| s.component)
                            .or(e.reporting_component)
                            .unwrap_or_default(),
                        first_seen: e.first_timestamp.map(|t| t.0.to_string()),
                        last_seen: last,
                    }
                })
                .collect();
            events.sort_by(|a, b| a.last_seen.cmp(&b.last_seen));
            Json(serde_json::json!({ "events": events })).into_response()
        }
        Err(e) => kube_err(&e),
    }
}

pub async fn pod_yaml_handler(
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
) -> axum::response::Response {
    if let Some(r) = invalid_names(&namespace, &name) {
        return r;
    }
    let client = state.read().await.kube_client.client();
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    match api.get(&name).await {
        Ok(mut pod) => {
            pod.metadata.managed_fields = None;
            let mut value = match serde_json::to_value(&pod) {
                Ok(v) => v,
                Err(e) => {
                    let (st, j) = err_json(500, "SERIALIZE", &e.to_string());
                    return (st, j).into_response();
                }
            };
            if let Some(obj) = value.as_object_mut() {
                obj.insert("apiVersion".into(), "v1".into());
                obj.insert("kind".into(), "Pod".into());
            }
            match serde_yaml::to_string(&value) {
                Ok(yaml) => Json(serde_json::json!({ "yaml": yaml })).into_response(),
                Err(e) => {
                    let (st, j) = err_json(500, "SERIALIZE", &e.to_string());
                    (st, j).into_response()
                }
            }
        }
        Err(e) => kube_err(&e),
    }
}

// ── Logs ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PodLogQuery {
    pub token: Option<String>,
    pub container: Option<String>,
    pub tail: Option<i64>,
    pub timestamps: Option<String>,
    pub previous: Option<String>,
}

pub async fn ws_pod_logs(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
    Query(q): Query<PodLogQuery>,
) -> axum::response::Response {
    let s = state.read().await;
    let identity = match authorize_permission(&s, q.token.as_deref(), ApiPermission::ClusterAdmin) {
        Ok(i) => i,
        Err(resp) => return *resp,
    };
    let client = s.kube_client.client();
    let audit = s.audit.clone();
    drop(s);

    let container = q.container.filter(|c| !c.is_empty());
    if !valid_k8s_name(&namespace)
        || !valid_k8s_name(&name)
        || container.as_deref().is_some_and(|c| !valid_k8s_name(c))
    {
        let (st, j) = err_json(400, "INVALID", "invalid namespace, pod or container name");
        return (st, j).into_response();
    }

    let lp = LogParams {
        follow: true,
        tail_lines: Some(q.tail.unwrap_or(500).clamp(1, MAX_TAIL_LINES)),
        timestamps: flag(q.timestamps.as_deref()),
        previous: flag(q.previous.as_deref()),
        container: container.clone(),
        ..Default::default()
    };
    let user = identity.username.unwrap_or_else(|| "api-token".into());
    audit_pod(
        &audit,
        &user,
        crate::audit_trail::AuditAction::ViewLogs,
        &namespace,
        &name,
        container.as_deref(),
        true,
        serde_json::Value::Null,
    )
    .await;

    ws.on_upgrade(move |socket| stream_logs(socket, client, namespace, name, lp))
        .into_response()
}

async fn stream_logs(
    socket: WebSocket,
    client: kube::Client,
    namespace: String,
    name: String,
    lp: LogParams,
) {
    use futures_util::io::AsyncBufReadExt;

    let (mut tx, mut rx) = socket.split();
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let reader = match api.log_stream(&name, &lp).await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx
                .send(Message::text(format!(
                    "\x1b[31merror: cannot stream logs for {namespace}/{name}: {}\x1b[0m",
                    sanitize_error(&e)
                )))
                .await;
            let _ = tx.close().await;
            return;
        }
    };

    let mut lines = Box::pin(reader).lines();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            line = lines.next() => match line {
                Some(Ok(l)) => {
                    if tx.send(Message::text(l)).await.is_err() {
                        break;
                    }
                }
                Some(Err(e)) => {
                    let _ = tx
                        .send(Message::text(format!("\x1b[31merror: {e}\x1b[0m")))
                        .await;
                    break;
                }
                None => {
                    let _ = tx
                        .send(Message::text("\x1b[90m[log stream ended]\x1b[0m"))
                        .await;
                    break;
                }
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
            _ = heartbeat.tick() => {
                if tx.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = tx.close().await;
}

// ── Exec ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PodExecQuery {
    pub token: Option<String>,
    pub container: Option<String>,
    pub shell: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ExecControl {
    Resize { cols: u16, rows: u16 },
}

/// Fixed shell launchers — never user-supplied argv.
fn shell_command(shell: &str) -> Vec<String> {
    let script = match shell {
        "bash" => "export TERM=xterm-256color; exec bash",
        "sh" => "export TERM=xterm-256color; exec sh",
        _ => "export TERM=xterm-256color; if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi",
    };
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

pub async fn ws_pod_exec(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    Path((namespace, name)): Path<(String, String)>,
    Query(q): Query<PodExecQuery>,
) -> axum::response::Response {
    let s = state.read().await;
    let identity = match authorize_permission(&s, q.token.as_deref(), ApiPermission::ClusterAdmin) {
        Ok(i) => i,
        Err(resp) => return *resp,
    };
    let client = s.kube_client.client();
    let audit = s.audit.clone();
    drop(s);

    let container = q.container.filter(|c| !c.is_empty());
    if !valid_k8s_name(&namespace)
        || !valid_k8s_name(&name)
        || container.as_deref().is_some_and(|c| !valid_k8s_name(c))
    {
        let (st, j) = err_json(400, "INVALID", "invalid namespace, pod or container name");
        return (st, j).into_response();
    }
    let shell = q.shell.unwrap_or_else(|| "auto".into());
    let user = identity.username.unwrap_or_else(|| "api-token".into());

    ws.on_upgrade(move |socket| async move {
        run_exec(
            socket, client, audit, user, namespace, name, container, shell,
        )
        .await
    })
    .into_response()
}

fn exit_code(
    status: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Status>,
) -> Option<i32> {
    let status = status?;
    if status.status.as_deref() == Some("Success") {
        return Some(0);
    }
    status
        .details
        .and_then(|d| d.causes)
        .and_then(|causes| {
            causes
                .into_iter()
                .find(|c| c.reason.as_deref() == Some("ExitCode"))
                .and_then(|c| c.message)
        })
        .and_then(|m| m.trim().parse().ok())
}

#[allow(clippy::too_many_arguments)]
async fn run_exec(
    socket: WebSocket,
    client: kube::Client,
    audit: SharedAuditTrail,
    user: String,
    namespace: String,
    name: String,
    container: Option<String>,
    shell: String,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let started = Instant::now();
    let (mut tx, mut rx) = socket.split();
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let mut ap = AttachParams::interactive_tty();
    if let Some(c) = &container {
        ap = ap.container(c.clone());
    }

    let mut attached = match api.exec(&name, shell_command(&shell), &ap).await {
        Ok(a) => a,
        Err(e) => {
            let msg = sanitize_error(&e);
            audit_pod(
                &audit,
                &user,
                crate::audit_trail::AuditAction::Exec,
                &namespace,
                &name,
                container.as_deref(),
                false,
                serde_json::json!({ "shell": shell, "error": msg }),
            )
            .await;
            let _ = tx
                .send(Message::text(
                    serde_json::json!({ "type": "error", "message": msg }).to_string(),
                ))
                .await;
            let _ = tx.close().await;
            return;
        }
    };
    audit_pod(
        &audit,
        &user,
        crate::audit_trail::AuditAction::Exec,
        &namespace,
        &name,
        container.as_deref(),
        true,
        serde_json::json!({ "shell": shell, "event": "start" }),
    )
    .await;

    let (Some(mut stdin), Some(mut stdout)) = (attached.stdin(), attached.stdout()) else {
        let _ = tx.close().await;
        return;
    };
    let mut resize = attached.terminal_size();
    let status = attached.take_status();

    let mut buf = vec![0u8; 8192];
    let mut client_gone = false;
    loop {
        tokio::select! {
            n = stdout.read(&mut buf) => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Message::binary(buf[..n].to_vec())).await.is_err() {
                        client_gone = true;
                        break;
                    }
                }
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Binary(b))) => {
                    if stdin.write_all(&b).await.is_err() {
                        break;
                    }
                }
                // Text frames are control messages only; keystrokes arrive as binary.
                Some(Ok(Message::Text(t))) => {
                    if let Ok(ExecControl::Resize { cols, rows }) = serde_json::from_str(&t) {
                        if let Some(r) = resize.as_mut() {
                            let _ = r.try_send(TerminalSize {
                                width: cols.clamp(2, 1000),
                                height: rows.clamp(2, 1000),
                            });
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                    client_gone = true;
                    break;
                }
                _ => {}
            },
        }
    }

    let code = if client_gone {
        attached.abort();
        None
    } else {
        match status {
            Some(fut) => tokio::time::timeout(Duration::from_secs(3), fut)
                .await
                .ok()
                .and_then(exit_code),
            None => None,
        }
    };
    audit_pod(
        &audit,
        &user,
        crate::audit_trail::AuditAction::Exec,
        &namespace,
        &name,
        container.as_deref(),
        true,
        serde_json::json!({
            "shell": shell,
            "event": "end",
            "exit_code": code,
            "duration_secs": started.elapsed().as_secs(),
        }),
    )
    .await;
    if !client_gone {
        let _ = tx
            .send(Message::text(
                serde_json::json!({ "type": "exit", "code": code }).to_string(),
            ))
            .await;
        let _ = tx.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_names() {
        assert!(valid_k8s_name("zorvia-system"));
        assert!(valid_k8s_name("virt-launcher-web-abc12"));
        assert!(!valid_k8s_name(""));
        assert!(!valid_k8s_name("../etc"));
        assert!(!valid_k8s_name("a/b"));
        assert!(!valid_k8s_name("-lead"));
        assert!(!valid_k8s_name("Upper"));
    }

    #[test]
    fn only_recreating_controllers_are_restartable() {
        for k in [
            "ReplicaSet",
            "StatefulSet",
            "DaemonSet",
            "ReplicationController",
        ] {
            assert!(restartable_owner(k), "{k}");
        }
        for k in ["VirtualMachineInstance", "Job", "Node", ""] {
            assert!(!restartable_owner(k), "{k}");
        }
    }

    #[test]
    fn shell_command_is_fixed() {
        assert_eq!(shell_command("rm -rf /")[0], "/bin/sh");
        assert!(shell_command("anything")[2].contains("command -v bash"));
    }

    #[test]
    fn parses_resize_control() {
        let c: ExecControl =
            serde_json::from_str(r#"{"type":"resize","cols":120,"rows":40}"#).expect("resize");
        let ExecControl::Resize { cols, rows } = c;
        assert_eq!((cols, rows), (120, 40));
    }
}
