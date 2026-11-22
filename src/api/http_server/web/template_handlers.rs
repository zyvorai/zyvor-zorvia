// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real, curated OS template catalog, backed by `crate::templates::TEMPLATES`
//! -- ~30 pre-built `VMConfig`s (Ubuntu/Fedora/RHEL/Windows/Talos/etc, each
//! with real firmware/feature/clock presets, e.g. Secure Boot UEFI + HyperV
//! enlightenments for Windows) that had zero HTTP route before this.
//!
//! This is a different, real feature from the deleted `Templates.tsx`
//! (which wanted user-authored templates saved from existing VMs, with no
//! backing at all) and from `ContentLibrary.tsx` (vCenter-style versioned
//! libraries, also no backing).
//!
//! `deploy_template_handler` deploys one for real: each template is
//! already a complete, bootable `VMConfig` (real container-disk image +
//! cloud-init, not just a name) built by `VMConfigBuilder`, the same
//! builder `fabric_create_vm` uses -- so deploying is just overriding
//! `name`/`namespace` and calling the same real `client.create_vm()`.

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

pub async fn list_templates_handler() -> impl IntoResponse {
    Json(json!(crate::templates::TEMPLATES.list_by_family())).into_response()
}

pub async fn get_template_handler(Path(name): Path<String>) -> impl IntoResponse {
    match crate::templates::TEMPLATES.get(&name) {
        Some(config) => Json(config).into_response(),
        None => {
            let (st, j) = err_json(404, "NOT_FOUND", &format!("No template named '{name}'"));
            (st, j).into_response()
        }
    }
}

/// Core of "create a VM from a template" -- shared by the manual deploy
/// handler and `WarmPool` reconciliation (Phase 7 follow-up), so a
/// pool-provisioned VM is built exactly the same way a manually deployed
/// one is.
pub(crate) async fn create_vm_from_template(
    client: &crate::kube::KubeClient,
    namespace: &str,
    template_name: &str,
    vm_name: &str,
) -> anyhow::Result<()> {
    let mut config = crate::templates::TEMPLATES
        .get(template_name)
        .ok_or_else(|| anyhow::anyhow!("No template named '{template_name}'"))?;
    config.name = vm_name.to_string();
    config.namespace = namespace.to_string();
    client.create_vm(&config).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct DeployTemplateBody {
    pub vm_name: String,
    #[serde(default)]
    pub start: Option<bool>,
}

pub async fn deploy_template_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<DeployTemplateBody>,
) -> impl IntoResponse {
    if crate::templates::TEMPLATES.get(&name).is_none() {
        let (st, j) = err_json(404, "NOT_FOUND", &format!("No template named '{name}'"));
        return (st, j).into_response();
    }
    if body.vm_name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "vm_name is required");
        return (st, j).into_response();
    }

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let result = create_vm_from_template(&client, &namespace, &name, &body.vm_name).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Create,
        "vm",
        &body.vm_name,
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()),
    )
    .await;
    super::webhook_handlers::dispatch_webhook_event(
        WebhookEvent::VMCreated,
        &body.vm_name,
        result.is_ok(),
    )
    .await;

    match result {
        Ok(_) => {
            if body.start.unwrap_or(true) {
                let _ = client.start_vm(&namespace, &body.vm_name).await;
            }
            (
                StatusCode::CREATED,
                Json(json!({ "vm_name": body.vm_name, "template": name })),
            )
                .into_response()
        }
        Err(e) => {
            let (st, j) = err_json(500, "DEPLOY_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}
