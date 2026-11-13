//! VMware -> KubeVirt import API (see `crate::migration_import`). Every route
//! is cluster.admin: it creates privileged Jobs and consumes vCenter
//! credentials. Progress and cancellation use `/operations/{id}`.

use super::*;
use crate::migration_import::{self, job::h2kvm_image, ImportRequest};
use crate::operations::OperationsDb;
use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::api::storage::v1::StorageClass;
use kube::api::Api;
use serde_json::json;

fn caller(auth: &Option<axum::Extension<crate::api::auth::AuthIdentity>>) -> String {
    auth.as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into())
}

async fn audit_import(
    audit: &SharedAuditTrail,
    user: &str,
    vcenter: &str,
    namespace: &str,
    details: serde_json::Value,
) {
    let entry = crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", vcenter),
        timestamp: chrono::Utc::now(),
        user: user.to_string(),
        // Starts privileged Jobs and reads vCenter credentials.
        severity: crate::audit_trail::AuditSeverity::High,
        action: crate::audit_trail::AuditAction::Exec,
        resource_type: "vm-import".into(),
        resource_name: vcenter.to_string(),
        namespace: namespace.to_string(),
        details,
        ip_address: String::new(),
        success: true,
    };
    audit.write().await.record(entry);
}

/// What would stop this import from working, and what Zorvia cannot check.
async fn preflight(client: &kube::Client, req: &ImportRequest) -> (Vec<String>, Vec<String>) {
    let mut blockers = req.problems();
    let mut warnings = Vec::new();

    if !crate::features::allow_non_ga("vm-import-h2kvm") {
        blockers.push(
            "vm-import-h2kvm is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable".into(),
        );
    }
    if !req.mode.is_lab_stub() && h2kvm_image().is_none() {
        blockers.push(
            "ZORVIA_H2KVM_IMAGE is not set (h2kvm is licensed separately; point it at your h2kvm CLI image)"
                .into(),
        );
    }
    if !blockers.is_empty() {
        return (blockers, warnings); // nothing further can be checked safely
    }

    // KubeVirt and CDI installed?
    match client.list_api_groups().await {
        Ok(groups) => {
            for needed in ["kubevirt.io", "cdi.kubevirt.io"] {
                if !groups.groups.iter().any(|g| g.name == needed) {
                    blockers.push(format!(
                        "API group {needed} is not installed on this cluster"
                    ));
                }
            }
        }
        Err(e) => blockers.push(format!("could not check installed API groups: {e}")),
    }

    // Namespace, and whether it will admit a privileged pod (vsphere only).
    let namespaces: Api<Namespace> = Api::all(client.clone());
    match namespaces.get(&req.namespace).await {
        Ok(ns) => {
            if !req.mode.is_lab_stub() {
                let enforce = ns
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|l| l.get("pod-security.kubernetes.io/enforce"))
                    .cloned();
                match enforce.as_deref() {
                    Some("privileged") => {}
                    Some(level) => blockers.push(format!(
                        "namespace '{}' enforces Pod Security '{level}', which rejects the privileged h2kvm Job; set pod-security.kubernetes.io/enforce=privileged",
                        req.namespace
                    )),
                    None => warnings.push(format!(
                        "namespace '{}' has no Pod Security label; the privileged h2kvm Job needs pod-security.kubernetes.io/enforce=privileged if your cluster's default is stricter",
                        req.namespace
                    )),
                }
            }
        }
        Err(e) => blockers.push(format!("cannot inspect namespace '{}': {e}", req.namespace)),
    }

    // Storage classes and target VM names.
    let classes: Api<StorageClass> = Api::all(client.clone());
    let mut wanted: Vec<&String> = req
        .vms
        .iter()
        .filter_map(|v| v.storage_class.as_ref())
        .collect();
    if let Some(sc) = &req.scratch_storage_class {
        wanted.push(sc);
    }
    wanted.sort();
    wanted.dedup();
    for sc in wanted {
        if let Err(e) = classes.get(sc).await {
            blockers.push(format!("cannot inspect StorageClass '{sc}': {e}"));
        }
    }
    if migration_import::dns_label(&req.namespace) {
        for vm in &req.vms {
            let target = vm
                .target_vm_name
                .clone()
                .unwrap_or_else(|| migration_import::default_target_name(&vm.source_vm));
            if let Err(e) =
                migration_import::ensure_target_available(client, &req.namespace, &target).await
            {
                blockers.push(e.to_string());
            }
        }
    }

    let attachments = migration_import::networks::attachment_api(client, &req.namespace);
    let mut names: Vec<_> = req
        .vms
        .iter()
        .flat_map(|vm| vm.networks.iter())
        .filter_map(|n| n.attachment.as_deref())
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        match attachments.get(name).await {
            Ok(attachment) => {
                match serde_json::to_value(&attachment)
                    .map_err(anyhow::Error::from)
                    .and_then(|v| migration_import::networks::validate_attachment(&v))
                {
                    Ok(()) => {}
                    Err(e) => blockers.push(format!(
                        "NetworkAttachmentDefinition '{name}' is invalid: {e}"
                    )),
                }
            }
            Err(e) => blockers.push(format!(
                "cannot inspect NetworkAttachmentDefinition '{name}': {e}"
            )),
        }
    }

    if req.mode.is_lab_stub() {
        warnings.push(
            if matches!(req.mode, crate::migration_import::ImportMode::Local) {
                "mode=local: Job emits stage logs, then creates a stopped emptyDisk VirtualMachine (lab)"
                    .into()
            } else {
                "mode=stub: Job emits stage logs only; no VirtualMachine is created (lab orchestration proof)"
                    .into()
            },
        );
        warnings.push(format!(
            "ServiceAccount '{}' must exist in namespace '{}' if you override the default",
            req.service_account.as_deref().unwrap_or("default"),
            req.namespace
        ));
    } else {
        // Zorvia has no read access to Secrets or ServiceAccounts by design.
        warnings.push(format!(
            "Secret '{}' (vCenter credentials) and ServiceAccount '{}' must already exist in namespace '{}'; Zorvia cannot verify them",
            req.source.secret_name,
            req.service_account.as_deref().unwrap_or("zorvia-h2kvm"),
            req.namespace
        ));
        warnings.push(
            "vCenter reachability and the scratch size (must hold the largest disk twice over) are not checked until the Job runs"
                .into(),
        );
    }
    (blockers, warnings)
}

pub async fn preflight_import_handler(
    State(state): State<SharedState>,
    Json(req): Json<ImportRequest>,
) -> impl IntoResponse {
    if !crate::features::allow_non_ga("vm-import-h2kvm") {
        let (st, j) = err_json(
            403,
            "EXPERIMENTAL_DISABLED",
            "vm-import-h2kvm is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
        );
        return (st, j).into_response();
    }
    let client = state.read().await.kube_client.client();
    let (blockers, warnings) = preflight(&client, &req).await;
    Json(json!({ "ready": blockers.is_empty(), "blockers": blockers, "warnings": warnings }))
        .into_response()
}

pub async fn create_import_handler(
    State(state): State<SharedState>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Json(req): Json<ImportRequest>,
) -> impl IntoResponse {
    if !crate::features::allow_non_ga("vm-import-h2kvm") {
        let (st, j) = err_json(
            403,
            "EXPERIMENTAL_DISABLED",
            "vm-import-h2kvm is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
        );
        return (st, j).into_response();
    }
    let (client, audit) = {
        let s = state.read().await;
        (s.kube_client.client(), s.audit.clone())
    };
    let (blockers, warnings) = preflight(&client, &req).await;
    if !blockers.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({ "error": "PREFLIGHT_FAILED", "blockers": blockers, "warnings": warnings }),
            ),
        )
            .into_response();
    }
    match migration_import::enqueue_wave_in(OperationsDb::global(), &req) {
        Ok((wave_id, ops)) => {
            audit_import(
                &audit,
                &caller(&auth),
                &req.source.vcenter,
                &req.namespace,
                json!({
                    "wave_id": wave_id,
                    "vms": req.vms.iter().map(|v| v.source_vm.clone()).collect::<Vec<_>>(),
                }),
            )
            .await;
            (
                StatusCode::ACCEPTED,
                Json(json!({
                    "wave_id": wave_id,
                    "warnings": warnings,
                    "operations": ops.iter().map(|o| json!({
                        "operation_id": o.id,
                        "source_vm": o.resource,
                        "target_vm_name": o.params.get("target_vm_name"),
                    })).collect::<Vec<_>>(),
                })),
            )
                .into_response()
        }
        Err(e) => {
            let (st, j) = err_json(400, "INVALID", &e.to_string());
            (st, j).into_response()
        }
    }
}

pub async fn list_imports_handler() -> impl IntoResponse {
    match OperationsDb::global().list(Some(migration_import::OP_KIND), 200) {
        Ok(ops) => Json(json!({ "imports": ops.iter().map(|o| json!({
            "operation_id": o.id,
            "wave_id": o.params.get("wave_id"),
            "source_vm": o.resource,
            "target_vm_name": o.params.get("target_vm_name"),
            "namespace": o.namespace,
            "state": o.state,
            "phase": o.phase,
            "progress": o.progress,
            "error": o.error,
            "created": o.created,
            "completed": o.completed,
        })).collect::<Vec<_>>() }))
        .into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "OPERATIONS_UNAVAILABLE", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn get_wave_handler(Path(wave_id): Path<String>) -> impl IntoResponse {
    match migration_import::wave_summary(OperationsDb::global(), &wave_id) {
        Ok(s) if s["total"] == 0 => {
            let (st, j) = err_json(404, "NOT_FOUND", "wave not found");
            (st, j).into_response()
        }
        Ok(s) => Json(s).into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "OPERATIONS_UNAVAILABLE", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}
