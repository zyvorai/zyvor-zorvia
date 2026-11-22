// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Certificate lifecycle API (Experimental, `cert-lifecycle`): inventory, expiry status and
//! alerts, cert-manager state and the renew action. Every route is cluster.admin (see
//! `required_permission`). Responses carry certificate metadata only, never PEM or keys.
//! Logic lives in `crate::certs`; this file gathers cluster access, audits and serves.

use super::*;
use crate::certs::{self, certmanager, expiry, kube_source::KubeCertSource};
use serde_json::json;

fn gate() -> Option<axum::response::Response> {
    if crate::features::allow_non_ga(certs::FEATURE_ID) {
        return None;
    }
    let (st, j) = err_json(
        403,
        "EXPERIMENTAL_DISABLED",
        "cert-lifecycle is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
    );
    Some((st, j).into_response())
}

fn caller(auth: &Option<axum::Extension<crate::api::auth::AuthIdentity>>) -> String {
    auth.as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into())
}

async fn source(state: &SharedState) -> KubeCertSource {
    let s = state.read().await;
    KubeCertSource::new(s.kube_client.client(), s.namespace.clone())
}

pub async fn certs_status_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let st = expiry::load_state(&expiry::state_path());
    Json(json!({
        "feature": certs::FEATURE_ID,
        "maturity": "experimental",
        "alert_thresholds_days": expiry::thresholds_from_env(),
        "check_interval_secs": expiry::interval_from_env(),
        "scheduler_leader": crate::api::leader::is_leader(),
        "last_check": st.last_check,
        "last_certificates": st.last_certificates,
        "last_alerts": st.last_alerts,
        "last_error": st.last_error,
        "watched_files": certs::file_specs_from_env().iter()
            .map(|f| json!({ "name": f.name, "env": f.env })).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct InventoryQuery {
    pub namespace: Option<String>,
}

pub async fn certs_inventory_handler(
    State(state): State<SharedState>,
    axum::extract::Query(q): axum::extract::Query<InventoryQuery>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    if let Some(ns) = &q.namespace {
        if let Err(e) = crate::kube::lifecycle::validate_k8s_name("namespace", ns) {
            return err_json(400, "INVALID_INPUT", &e.to_string()).into_response();
        }
    }
    let src = source(&state).await;
    match certs::collect(
        &src,
        &certs::file_specs_from_env(),
        q.namespace.as_deref(),
        chrono::Utc::now(),
    )
    .await
    {
        Ok(inv) => Json(inv).into_response(),
        Err(e) => err_json(502, "INVENTORY_FAILED", &sanitize_error(&e)).into_response(),
    }
}

pub async fn certs_alerts_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    Json(json!(expiry::recent_alerts(&expiry::alerts_path()))).into_response()
}

pub async fn certmanager_list_handler(State(state): State<SharedState>) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let src = source(&state).await;
    match certs::CertSource::list_certmanager(&src).await {
        Ok(None) => Json(json!({ "installed": false, "certificates": [] })).into_response(),
        Ok(Some(list)) => {
            let certificates: Vec<_> = list
                .iter()
                .filter_map(certmanager::parse_certificate)
                .collect();
            Json(json!({ "installed": true, "certificates": certificates })).into_response()
        }
        Err(e) => err_json(502, "CERT_MANAGER_FAILED", &sanitize_error(&e)).into_response(),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewBody {
    pub namespace: String,
    pub name: String,
    /// Must equal `renew <namespace>/<name>`.
    #[serde(default)]
    pub confirm: String,
}

pub async fn certmanager_renew_handler(
    State(state): State<SharedState>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Json(req): Json<RenewBody>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    for (k, v) in [("namespace", &req.namespace), ("name", &req.name)] {
        // Certificate names may be DNS subdomains; namespaces are labels.
        let ok = if k == "namespace" {
            crate::kube::lifecycle::validate_k8s_name(k, v).is_ok()
        } else {
            !v.is_empty()
                && v.len() <= 253
                && v.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
                && !v.starts_with(['-', '.'])
                && !v.ends_with(['-', '.'])
        };
        if !ok {
            return err_json(400, "INVALID_INPUT", &format!("invalid {k}")).into_response();
        }
    }
    let phrase = certmanager::renew_phrase(&req.namespace, &req.name);
    if req.confirm != phrase {
        return err_json(
            400,
            "CONFIRMATION_REQUIRED",
            &format!("set confirm to the exact phrase \"{phrase}\""),
        )
        .into_response();
    }
    let audit = state.read().await.audit.clone();
    let src = source(&state).await;
    let result = certmanager::renew(&src, &req.namespace, &req.name, chrono::Utc::now()).await;
    let entry = crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", &req.name),
        timestamp: chrono::Utc::now(),
        user: caller(&auth),
        severity: crate::audit_trail::AuditSeverity::High,
        action: crate::audit_trail::AuditAction::ConfigChange,
        resource_type: "certificate".into(),
        resource_name: format!("renew:{}", req.name),
        namespace: req.namespace.clone(),
        details: json!({
            "operation": "cert-manager-renew",
            "method": "status Issuing=True (ManuallyTriggered)",
            "error": result.as_ref().err().map(|e| e.to_string()),
        }),
        ip_address: String::new(),
        success: result.is_ok(),
    };
    audit.write().await.record(entry);
    match result {
        Ok(c) => (
            StatusCode::ACCEPTED,
            Json(json!({ "renewing": true, "certificate": c })),
        )
            .into_response(),
        Err(certmanager::RenewError::NotInstalled) => err_json(
            404,
            "CERT_MANAGER_NOT_INSTALLED",
            "cert-manager is not installed",
        )
        .into_response(),
        Err(certmanager::RenewError::NotFound) => {
            err_json(404, "NOT_FOUND", "no such cert-manager Certificate").into_response()
        }
        Err(certmanager::RenewError::AlreadyIssuing) => err_json(
            409,
            "ALREADY_ISSUING",
            "an issuance is already in progress for this Certificate",
        )
        .into_response(),
        Err(certmanager::RenewError::Conflict(m)) => err_json(409, "CONFLICT", &m).into_response(),
        Err(certmanager::RenewError::Failed(m)) => {
            err_json(502, "RENEW_FAILED", &sanitize_error(&m)).into_response()
        }
    }
}

/// Scheduler loop (leader only, feature enabled): scan, evaluate thresholds, notify.
pub fn spawn_cert_expiry_loop(state: SharedState) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            if crate::features::allow_non_ga(certs::FEATURE_ID)
                && !crate::api::leader::skip_if_follower("cert-expiry")
            {
                let src = source(&state).await;
                if let Err(e) = expiry::check_once(
                    &src,
                    &certs::file_specs_from_env(),
                    &expiry::thresholds_from_env(),
                    &expiry::state_path(),
                    &expiry::alerts_path(),
                    chrono::Utc::now(),
                )
                .await
                {
                    log::warn!("certificate expiry check failed: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(expiry::interval_from_env())).await;
        }
    });
}
