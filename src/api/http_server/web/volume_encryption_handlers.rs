// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Volume envelope-encryption API (see `crate::volume_encryption`). Every route is
//! cluster.admin and Experimental. Responses never carry key material, wrapped or not.

use super::*;
use crate::volume_encryption::{self as ve, DataKeyWrapper, EncryptRequest};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::api::storage::v1::StorageClass;
use kube::api::{Api, PostParams};
use serde_json::json;

fn caller(auth: &Option<axum::Extension<crate::api::auth::AuthIdentity>>) -> String {
    auth.as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into())
}

async fn audit_ve(
    audit: &SharedAuditTrail,
    user: &str,
    verb: &str,
    req_ns: &str,
    name: &str,
    ok: bool,
    details: serde_json::Value,
) {
    let entry = crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", name),
        timestamp: chrono::Utc::now(),
        user: user.to_string(),
        severity: crate::audit_trail::AuditSeverity::High,
        action: crate::audit_trail::AuditAction::ConfigChange,
        resource_type: "volume-encryption".into(),
        resource_name: format!("{verb}:{name}"),
        namespace: req_ns.to_string(),
        details,
        ip_address: String::new(),
        success: ok,
    };
    audit.write().await.record(entry);
}

fn gate() -> Option<axum::response::Response> {
    if crate::features::allow_non_ga(ve::FEATURE_ID) {
        return None;
    }
    let (st, j) = err_json(
        403,
        "EXPERIMENTAL_DISABLED",
        "volume-encryption is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
    );
    Some((st, j).into_response())
}

/// Blockers and warnings for a request; checks the key provider and StorageClass.
async fn preflight(client: &kube::Client, req: &EncryptRequest) -> (Vec<String>, Vec<String>) {
    let mut blockers = req.problems();
    let mut warnings = Vec::new();
    if crate::keys::provider().is_none() {
        blockers.push(
            "no key provider is configured (ZORVIA_KEY_FILE/ZORVIA_KEY_HEX or ZORVIA_VAULT_*); data keys are never stored unwrapped"
                .into(),
        );
    }
    if !blockers.is_empty() {
        return (blockers, warnings);
    }
    let classes: Api<StorageClass> = Api::all(client.clone());
    match classes.get(&req.storage_class).await {
        Ok(sc) => {
            let params = sc.parameters.unwrap_or_default();
            let (verified, w) = ve::check_storage_class(&sc.provisioner, &params);
            if !verified {
                let msg = format!("StorageClass '{}': {}", req.storage_class, w.join("; "));
                if req.acknowledge_unverified_encryption {
                    warnings.push(format!("{msg} (acknowledged)"));
                } else {
                    blockers.push(format!(
                        "{msg}; set acknowledge_unverified_encryption=true to proceed anyway"
                    ));
                }
            }
        }
        Err(e) => blockers.push(format!(
            "cannot inspect StorageClass '{}': {e}",
            req.storage_class
        )),
    }
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &req.namespace);
    if let Ok(Some(_)) = secrets
        .get_opt(&ve::dek_secret_name(&req.vm, &req.volume))
        .await
    {
        blockers.push("a data-key Secret for this volume already exists".into());
    }
    warnings.push(
        "the StorageClass's own KMS encrypts the volume; the Zorvia-wrapped data key is recorded for key management but is not consumed by the CSI driver (docs/VOLUME_ENCRYPTION.md)"
            .into(),
    );
    (blockers, warnings)
}

pub async fn status_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let p = crate::keys::provider();
    Json(json!({
        "feature": ve::FEATURE_ID,
        "maturity": "experimental",
        "provider": p.map(|p| p.provider_name()),
        "ready": p.is_some(),
    }))
    .into_response()
}

pub async fn plan_handler(
    State(state): State<SharedState>,
    Json(req): Json<EncryptRequest>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let client = state.read().await.kube_client.client();
    let (blockers, warnings) = preflight(&client, &req).await;
    Json(json!({
        "ready": blockers.is_empty(),
        "blockers": blockers,
        "warnings": warnings,
        "steps": [
            "generate a 256-bit data key and wrap it with the key provider",
            format!("create Secret {} holding only the wrapped key", ve::dek_secret_name(&req.vm, &req.volume)),
            format!("create encrypted DataVolume {} on StorageClass {}", req.dv_name(), req.storage_class),
        ],
    }))
    .into_response()
}

pub async fn apply_handler(
    State(state): State<SharedState>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Json(req): Json<EncryptRequest>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let (kc, audit) = {
        let s = state.read().await;
        (s.kube_client.clone(), s.audit.clone())
    };
    let client = kc.client();
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
    let user = caller(&auth);
    let name = req.dv_name();
    let result: anyhow::Result<()> = async {
        let provider = crate::keys::provider().ok_or_else(|| anyhow::anyhow!("no key provider"))?;
        let wrapped = ve::new_wrapped(provider, &req.ctx())?;
        let manifest = ve::secret_manifest(&req, &wrapped, 1, &provider.provider_name());
        let secret: Secret = serde_json::from_value(manifest)?;
        let secrets: Api<Secret> = Api::namespaced(client.clone(), &req.namespace);
        secrets.create(&PostParams::default(), &secret).await?;
        if let Err(e) = kc
            .apply_data_volume(&req.namespace, &ve::data_volume_manifest(&req))
            .await
        {
            // Do not leave an orphaned key record behind a volume that was not created.
            let _ = secrets
                .delete(
                    &ve::dek_secret_name(&req.vm, &req.volume),
                    &Default::default(),
                )
                .await;
            return Err(e);
        }
        Ok(())
    }
    .await;
    let details = json!({ "volume": req.volume, "vm": req.vm, "storage_class": req.storage_class,
        "secret": ve::dek_secret_name(&req.vm, &req.volume), "error": result.as_ref().err().map(|e| e.to_string()) });
    audit_ve(
        &audit,
        &user,
        "encrypt",
        &req.namespace,
        &name,
        result.is_ok(),
        details,
    )
    .await;
    match result {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "data_volume": name,
                "key_secret": ve::dek_secret_name(&req.vm, &req.volume),
                "warnings": warnings,
            })),
        )
            .into_response(),
        Err(e) => err_json(500, "APPLY_FAILED", &e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateRequest {
    pub namespace: String,
    pub vm: String,
    pub volume: String,
}

pub async fn rotate_handler(
    State(state): State<SharedState>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Json(req): Json<RotateRequest>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    for (k, v) in [
        ("namespace", &req.namespace),
        ("vm", &req.vm),
        ("volume", &req.volume),
    ] {
        if let Err(e) = crate::kube::lifecycle::validate_k8s_name(k, v) {
            return err_json(400, "INVALID_INPUT", &e.to_string()).into_response();
        }
    }
    let Some(provider) = crate::keys::provider() else {
        return err_json(409, "NO_KEY_PROVIDER", "no key provider is configured").into_response();
    };
    let (client, audit) = {
        let s = state.read().await;
        (s.kube_client.client(), s.audit.clone())
    };
    let name = ve::dek_secret_name(&req.vm, &req.volume);
    let ctx = ve::context(&req.namespace, &req.vm, &req.volume);
    let secrets: Api<Secret> = Api::namespaced(client, &req.namespace);
    let result: anyhow::Result<(u32, bool)> = async {
        let mut secret = secrets.get(&name).await?;
        let ann = secret.metadata.annotations.clone().unwrap_or_default();
        let version: u32 = ann
            .get(ve::ANN_VERSION)
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let old = secret
            .data
            .as_ref()
            .and_then(|d| d.get("wrapped-dek"))
            .and_then(|b| String::from_utf8(b.0.clone()).ok())
            .ok_or_else(|| anyhow::anyhow!("Secret has no wrapped-dek"))?;
        if !provider.needs_rewrap(&old) {
            return Ok((version, false));
        }
        let fresh = ve::rewrap(provider, &ctx, &old)?;
        let next = version + 1;
        secret.data = None;
        secret.string_data = Some([("wrapped-dek".to_string(), fresh)].into());
        let a = secret
            .metadata
            .annotations
            .get_or_insert_with(Default::default);
        a.insert(ve::ANN_VERSION.into(), next.to_string());
        a.insert(ve::ANN_PROVIDER.into(), provider.provider_name());
        // metadata.resourceVersion is kept, so a concurrent change makes this fail.
        secrets
            .replace(&name, &PostParams::default(), &secret)
            .await?;
        Ok((next, true))
    }
    .await;
    let user = caller(&auth);
    audit_ve(
        &audit,
        &user,
        "rotate",
        &req.namespace,
        &name,
        result.is_ok(),
        json!({ "result": result.as_ref().map(|(v, r)| json!({"version": v, "rewrapped": r})).ok(),
                "error": result.as_ref().err().map(|e| e.to_string()) }),
    )
    .await;
    match result {
        Ok((version, rewrapped)) => Json(json!({ "key_secret": name, "version": version,
            "rewrapped": rewrapped, "provider": provider.provider_name() }))
        .into_response(),
        Err(e) => err_json(500, "ROTATE_FAILED", &e.to_string()).into_response(),
    }
}
