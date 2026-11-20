// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Fabric-compat live migration API, backed by real KubeVirt
//! `VirtualMachineInstanceMigration` CRDs. KubeVirt's scheduler selects the
//! target node itself; there is no user-supplied host/URI, unlike a raw
//! libvirt/QEMU migration.

use super::*;
use serde_json::json;

fn migration_json(m: &crate::kube::types::VirtualMachineInstanceMigration) -> serde_json::Value {
    let status = m.status.as_ref();
    let state = status.and_then(|s| s.migration_state.as_ref());
    json!({
        "id": m.metadata.name,
        "vm_name": m.spec.vmi_name,
        "phase": status.and_then(|s| s.phase.clone()).unwrap_or_else(|| "Pending".into()),
        "source_node": state.and_then(|s| s.source_node.clone()),
        "target_node": state.and_then(|s| s.target_node.clone()),
        "start_timestamp": state.and_then(|s| s.start_timestamp.clone()),
        "end_timestamp": state.and_then(|s| s.end_timestamp.clone()),
        "completed": state.and_then(|s| s.completed).unwrap_or(false),
        "failed": state.and_then(|s| s.failed).unwrap_or(false),
        "created": m.metadata.creation_timestamp.as_ref().map(|t| t.0.to_string()),
    })
}

pub async fn fabric_migrate_vm(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    let result = client.migrate_vm(&namespace, &name).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Migrate,
        "vm",
        &name,
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()),
    )
    .await;
    super::webhook_handlers::dispatch_webhook_event(
        WebhookEvent::MigrationStarted,
        &name,
        result.is_ok(),
    )
    .await;
    match result {
        Ok(m) => (StatusCode::CREATED, Json(migration_json(&m))).into_response(),
        Err(e) => {
            // classify_migration_error is the sanitizer here; it needs the
            // raw error text (running sanitize_error() first would collapse
            // it before the classifier ever sees it — see hotplug_handlers).
            let raw = e.to_string();
            let (code, kind, msg) =
                crate::kube::lifecycle::classify_migration_error("start migration", &raw);
            let (st, j) = err_json(code, kind, &msg);
            (st, j).into_response()
        }
    }
}

pub async fn fabric_list_vm_migrations(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.list_migrations(&namespace, Some(&name)).await {
        Ok(list) => {
            Json(json!(list.iter().map(migration_json).collect::<Vec<_>>())).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(500, "MIGRATION_LIST_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_get_migration(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_migration(&namespace, &id).await {
        Ok(m) => Json(migration_json(&m)).into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_cancel_migration(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.cancel_migration(&namespace, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

/// Pre-flight readiness checks before starting a migration -- runs the real
/// checks in `crate::migration::assistant::MigrationAssistant` (target node
/// schedulability, VM running state, shared-vs-local storage), which existed
/// with no caller anywhere in the codebase until now.
#[derive(Debug, Deserialize)]
pub struct ReadinessQuery {
    /// Check a single VM; omit to check every VM in the server's namespace.
    #[serde(default)]
    pub vm: Option<String>,
    #[serde(default)]
    pub target_node: Option<String>,
}

pub async fn migration_readiness_handler(
    State(state): State<SharedState>,
    Query(params): Query<ReadinessQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm_names = match params.vm {
        Some(name) => vec![name],
        None => match client.list_vms(&namespace).await {
            Ok(vms) => vms
                .into_iter()
                .filter_map(|vm| vm.metadata.name)
                .collect::<Vec<_>>(),
            Err(e) => {
                let (st, j) = err_json(500, "READINESS_FAILED", &sanitize_error(&e));
                return (st, j).into_response();
            }
        },
    };

    if vm_names.is_empty() {
        return Json(json!({ "checks": [] })).into_response();
    }

    let mut assistant = crate::migration::assistant::MigrationAssistant::new();
    assistant.select_vms(vm_names);
    if let Some(node) = params.target_node {
        assistant.select_target(&node);
    }
    assistant.run_pre_checks(&namespace).await;

    Json(json!({ "checks": assistant.pre_check_results })).into_response()
}
