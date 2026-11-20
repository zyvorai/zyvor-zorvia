// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Rescue mode: offline guest-disk operations (set hostname, inject SSH
//! key, enable SSH service) against a *stopped* VM's own disk, by running a
//! privileged Kubernetes Job that mounts its PVC and calls GuestKit
//! directly. See docs/RESCUE.md for the security model and what's
//! deferred (password reset, package install, disk inspect).

use super::*;
use crate::kube::rescue::{build_rescue_job, resolve_vm_disk_pvc, RescueOperation};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};

/// Kubernetes object names (DNS-1123 subdomain) -- same validator as
/// pod-ops (`pod_handlers.rs`), reused here rather than duplicated because
/// this handler builds Job/Pod API paths from the VM name the same way.
fn valid_k8s_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !s.starts_with(['-', '.'])
        && !s.ends_with(['-', '.'])
}

fn caller(auth: &Option<axum::Extension<crate::api::auth::AuthIdentity>>) -> String {
    auth.as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into())
}

/// Mirrors `pod_handlers.rs`'s `audit_pod` -- this is exec-equivalent in
/// sensitivity (arbitrary offline mutation of a VM's guest filesystem via a
/// privileged Job), so it gets the same `High` severity and the same
/// `AuditAction::Exec` action, recorded unconditionally on success and
/// failure.
async fn audit_rescue(
    audit: &SharedAuditTrail,
    user: &str,
    vm_name: &str,
    success: bool,
    details: serde_json::Value,
) {
    let entry = crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", vm_name),
        timestamp: chrono::Utc::now(),
        user: user.to_string(),
        severity: crate::audit_trail::AuditSeverity::High,
        action: crate::audit_trail::AuditAction::Exec,
        resource_type: "vm".into(),
        resource_name: vm_name.to_string(),
        namespace: String::new(),
        details,
        ip_address: String::new(),
        success,
    };
    audit.write().await.record(entry);
}

fn kube_err(e: &kube::Error) -> axum::response::Response {
    let (status, code) = match e {
        kube::Error::Api(s) if s.code == 404 => (404, "NOT_FOUND"),
        kube::Error::Api(s) if s.code == 403 => (403, "FORBIDDEN"),
        _ => (500, "KUBE_ERROR"),
    };
    let (st, j) = err_json(status, code, &e.to_string());
    (st, j).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum RescueRequestBody {
    SetHostname { hostname: String },
    InjectSshKey { user: String, key: String },
    EnableSsh {},
}

impl RescueRequestBody {
    fn validate(self) -> Result<RescueOperation, &'static str> {
        match self {
            Self::SetHostname { hostname } => {
                let hostname = hostname.trim();
                if hostname.is_empty() || hostname.len() > 253 {
                    return Err("hostname must be 1-253 characters");
                }
                Ok(RescueOperation::SetHostname {
                    hostname: hostname.to_string(),
                })
            }
            Self::InjectSshKey { user, key } => {
                let user = user.trim();
                let key = key.trim();
                if user.is_empty()
                    || !user
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err("user must be a plain username");
                }
                if key.is_empty() || !(key.starts_with("ssh-") || key.starts_with("ecdsa-")) {
                    return Err("key does not look like an SSH public key");
                }
                Ok(RescueOperation::InjectSshKey {
                    user: user.to_string(),
                    key: key.to_string(),
                })
            }
            Self::EnableSsh {} => Ok(RescueOperation::EnableSsh),
        }
    }
}

/// `POST /v1/vms/:name/rescue` -- validates the VM is stopped, resolves its
/// PVC, creates the rescue Job, returns `202 {job_name, state: "queued"}`.
/// `cluster.admin`-gated (see `permissions.rs`) -- this creates a
/// privileged Job, not a routine per-VM action.
pub async fn rescue_vm_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Json(body): Json<RescueRequestBody>,
) -> axum::response::Response {
    if !valid_k8s_name(&name) {
        let (st, j) = err_json(400, "INVALID", "invalid VM name");
        return (st, j).into_response();
    }
    let operation = match body.validate() {
        Ok(op) => op,
        Err(msg) => {
            let (st, j) = err_json(400, "INVALID", msg);
            return (st, j).into_response();
        }
    };

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    let kube_client = s.kube_client.clone();
    let audit = s.audit.clone();
    drop(s);

    let vm = match kube_client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &format!("VM '{name}' not found: {e}"));
            return (st, j).into_response();
        }
    };

    // Defense in depth: RescueTab.tsx already disables every action unless
    // the VM is stopped, since a running VM's PVC is attached to
    // virt-launcher -- a mount conflict, not just a policy preference.
    let status = kube_client
        .get_status(&namespace, &name)
        .await
        .unwrap_or_else(|_| "Unknown".to_string());
    if !status.eq_ignore_ascii_case("stopped") {
        let (st, j) = err_json(
            409,
            "VM_NOT_STOPPED",
            &format!("VM must be stopped before Rescue mode can run (current state: {status})"),
        );
        return (st, j).into_response();
    }

    let pvc_name = match resolve_vm_disk_pvc(&vm, None) {
        Ok(p) => p,
        Err(e) => {
            let (st, j) = err_json(409, "NO_RESCUABLE_DISK", &e.to_string());
            return (st, j).into_response();
        }
    };

    // "job-name" becomes a Pod label value (63-char limit) once the Job
    // controller creates the pod -- generate_id's own name-truncation +
    // short hex suffix keeps this comfortably inside that limit.
    let job_name = crate::utils::generate_id("zorvia-rescue", &name);
    let job = build_rescue_job(&job_name, &namespace, &pvc_name, &operation);

    let jobs: Api<Job> = Api::namespaced(client, &namespace);
    let result = jobs.create(&PostParams::default(), &job).await;

    audit_rescue(
        &audit,
        &caller(&auth),
        &name,
        result.is_ok(),
        serde_json::json!({ "operation": operation.name(), "job_name": job_name, "pvc": pvc_name }),
    )
    .await;

    match result {
        Ok(_) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job_name": job_name, "state": "queued" })),
        )
            .into_response(),
        Err(e) => kube_err(&e),
    }
}

#[derive(Debug, Serialize)]
struct RescueJobStatus {
    state: &'static str,
    result: Option<serde_json::Value>,
}

/// `GET /v1/vms/:name/rescue/:job_name` -- reads the Job's status and (once
/// its pod has run) the single JSON result line `rescue-agent` printed to
/// stdout, via the same `Api<Pod>::logs()` kube-rs already exposes for
/// pod-ops. No separate result-passing infrastructure.
pub async fn get_rescue_job_handler(
    State(state): State<SharedState>,
    Path((name, job_name)): Path<(String, String)>,
) -> axum::response::Response {
    if !valid_k8s_name(&name) || !valid_k8s_name(&job_name) {
        let (st, j) = err_json(400, "INVALID", "invalid VM or job name");
        return (st, j).into_response();
    }

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let job = match jobs.get(&job_name).await {
        Ok(j) => j,
        Err(e) => return kube_err(&e),
    };

    let status = job.status.unwrap_or_default();
    let (state_str, done) = if status.succeeded.unwrap_or(0) > 0 {
        ("succeeded", true)
    } else if status.failed.unwrap_or(0) > 0 {
        ("failed", true)
    } else if status.active.unwrap_or(0) > 0 {
        ("running", false)
    } else {
        ("pending", false)
    };

    let mut result = None;
    if done {
        let pods: Api<Pod> = Api::namespaced(client, &namespace);
        if let Ok(list) = pods
            .list(&ListParams::default().labels(&format!("job-name={job_name}")))
            .await
        {
            if let Some(pod) = list.items.into_iter().next() {
                if let Some(pod_name) = pod.metadata.name {
                    if let Ok(logs) = pods.logs(&pod_name, &LogParams::default()).await {
                        result = logs
                            .lines()
                            .last()
                            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok());
                    }
                }
            }
        }
    }

    Json(RescueJobStatus {
        state: state_str,
        result,
    })
    .into_response()
}

/// `DELETE /v1/vms/:name/rescue/:job_name` -- best-effort cleanup once a
/// caller is done polling (Jobs also self-expire via
/// `ttl_seconds_after_finished`, this just lets the UI clean up sooner).
pub async fn delete_rescue_job_handler(
    State(state): State<SharedState>,
    Path((name, job_name)): Path<(String, String)>,
) -> axum::response::Response {
    if !valid_k8s_name(&name) || !valid_k8s_name(&job_name) {
        let (st, j) = err_json(400, "INVALID", "invalid VM or job name");
        return (st, j).into_response();
    }
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let jobs: Api<Job> = Api::namespaced(client, &namespace);
    let dp = DeleteParams {
        propagation_policy: Some(kube::api::PropagationPolicy::Background),
        ..Default::default()
    };
    match jobs.delete(&job_name, &dp).await {
        Ok(_) => Json(serde_json::json!({ "deleted": true })).into_response(),
        Err(e) => kube_err(&e),
    }
}
