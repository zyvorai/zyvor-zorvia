// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Support cases and diagnostic bundles.
//!
//! Cases are org-isolated in `crate::commercial::support`. Diagnostic
//! export is local by default; uploading to a case is a separate route that
//! requires an explicit `confirm: true`.

use super::commercial_handlers::{caller_of, fail, store, with, Auth};
use super::*;
use crate::commercial::diagnostics::{self, Bundle};
use crate::commercial::store::CommercialError;
use crate::commercial::support::{CaseStatus, NewAttachment, NewCase, SupportLimits};
use axum::extract::Json as AxumJson;
use axum::response::Response;
use base64::Engine;
use serde_json::json;

// ── cases ────────────────────────────────────────────────────────

pub async fn support_cases_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        // Retention is enforced opportunistically; failure is non-fatal.
        if let Err(e) = s.purge_expired_attachments() {
            log::warn!("support attachment purge failed: {e}");
        }
        s.list_cases(c)
    })
}

pub async fn support_case_create(auth: Auth, AxumJson(body): AxumJson<NewCase>) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| s.create_case(c, body))
}

pub async fn support_case_get(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.case_detail(c, &id))
}

#[derive(Debug, Deserialize)]
pub struct AttachmentBody {
    pub filename: String,
    #[serde(default)]
    pub content_type: Option<String>,
    pub data_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct MessageBody {
    pub body: String,
    #[serde(default)]
    pub internal: bool,
    #[serde(default)]
    pub attachments: Vec<AttachmentBody>,
}

pub async fn support_case_message(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<MessageBody>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        let mut files = Vec::with_capacity(body.attachments.len());
        for a in &body.attachments {
            let data = base64::engine::general_purpose::STANDARD
                .decode(a.data_base64.trim())
                .map_err(|_| {
                    CommercialError::Invalid("attachment data_base64 is not valid base64".into())
                })?;
            files.push(NewAttachment {
                filename: a.filename.clone(),
                content_type: a
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".into()),
                data,
            });
        }
        s.add_message(c, &id, &body.body, body.internal, false, files)
    })
}

#[derive(Debug, Deserialize)]
pub struct CaseStatusBody {
    pub to: String,
}

pub async fn support_case_status(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<CaseStatusBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        let to = CaseStatus::parse(&body.to)
            .ok_or_else(|| CommercialError::Invalid("unknown case status".into()))?;
        s.set_case_status(c, &id, to)
    })
}

#[derive(Debug, Deserialize)]
pub struct EscalateBody {
    pub reason: String,
}

pub async fn support_case_escalate(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<EscalateBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.escalate_case(c, &id, &body.reason)
    })
}

#[derive(Debug, Deserialize)]
pub struct AssignBody {
    pub owner: Option<String>,
}

pub async fn support_case_assign(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<AssignBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.assign_case(c, &id, body.owner.as_deref())
    })
}

/// ASCII-safe `filename="..."` value.
fn header_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn support_attachment_download(
    auth: Auth,
    Path((id, aid)): Path<(String, String)>,
) -> Response {
    let caller = match caller_of(&auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let store = match store() {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match store.get_attachment(&caller, &id, &aid) {
        // Always an opaque download: never rendered inline, never sniffed.
        Ok((meta, data)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!(
                        "attachment; filename=\"{}\"",
                        header_filename(&meta.filename)
                    ),
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            data,
        )
            .into_response(),
        Err(e) => fail(e),
    }
}

// ── diagnostics ──────────────────────────────────────────────────

async fn build_bundle(state: &SharedState, who: &str) -> Bundle {
    let (client, audit, config) = {
        let s = state.read().await;
        let config = json!({
            "namespace": s.namespace,
            "atlas_integration_enabled": s.atlas.is_some(),
            "kryton_integration_enabled": s.kryton.is_some(),
            "lab_mode": crate::api::auth::lab_mode(),
            "experimental_features_enabled": crate::features::experimental_enabled(),
            "commercial_integrations": {
                "notifications": false,
                "payments": false,
                "remote_management": false
            },
        });
        (s.client().client(), s.audit.clone(), config)
    };
    let failures: Vec<serde_json::Value> = audit
        .read()
        .await
        .failed_actions()
        .iter()
        .rev()
        .take(100)
        .map(|e| {
            json!({
                "at": e.timestamp,
                "action": format!("{:?}", e.action),
                "resource_type": e.resource_type,
                "resource_name": e.resource_name,
                "namespace": e.namespace,
                "details": e.details,
            })
        })
        .collect();
    diagnostics::collect(&client, who, failures, config).await
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticsBody {
    /// Defaults to true: show exactly what would be exported, export nothing.
    #[serde(default = "default_true")]
    pub preview: bool,
}

fn default_true() -> bool {
    true
}

/// Preview (default) or download a bundle. Never uploads anywhere.
pub async fn support_diagnostics(
    State(state): State<SharedState>,
    auth: Auth,
    body: Option<AxumJson<DiagnosticsBody>>,
) -> Response {
    let caller = match caller_of(&auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let preview = body.is_none_or(|b| b.0.preview);
    let bundle = build_bundle(&state, &caller.username).await;
    let manifest = bundle.manifest();
    if preview {
        return Json(json!({ "preview": true, "manifest": manifest, "bundle": bundle }))
            .into_response();
    }
    let stamp = bundle.generated_at.format("%Y%m%dT%H%M%SZ");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"zorvia-diagnostics-{stamp}.json\""),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bundle.to_bytes(),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct UploadBody {
    #[serde(default)]
    pub confirm: bool,
}

/// Attach a freshly generated bundle to a support case. Requires an
/// explicit `confirm: true`; a preview or download never triggers this.
pub async fn support_diagnostics_upload(
    State(state): State<SharedState>,
    auth: Auth,
    Path(case_id): Path<String>,
    AxumJson(body): AxumJson<UploadBody>,
) -> Response {
    let caller = match caller_of(&auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if !body.confirm {
        let (st, j) = err_json(
            400,
            "CONFIRMATION_REQUIRED",
            "Uploading a diagnostic bundle to support requires \"confirm\": true",
        );
        return (st, j).into_response();
    }
    let store = match store() {
        Ok(s) => s,
        Err(r) => return *r,
    };
    // Authorize against the case before doing any collection work.
    if let Err(e) = store.get_case(&caller, &case_id) {
        return fail(e);
    }
    let bundle = build_bundle(&state, &caller.username).await;
    let manifest = bundle.manifest();
    let data = bundle.to_bytes();
    let max = SupportLimits::from_env().attachment_max_bytes;
    if data.len() > max {
        let (st, j) = err_json(
            400,
            "BUNDLE_TOO_LARGE",
            &format!(
                "The bundle is {} bytes; the attachment limit is {max}. Download it instead.",
                data.len()
            ),
        );
        return (st, j).into_response();
    }
    let stamp = bundle.generated_at.format("%Y%m%dT%H%M%SZ");
    let note = format!(
        "Diagnostic bundle uploaded by {} (sha256 {}).",
        caller.username, manifest.sha256
    );
    let file = NewAttachment {
        filename: format!("zorvia-diagnostics-{stamp}.json"),
        content_type: "application/json".into(),
        data,
    };
    // Posted as the customer: an upload is not a support response.
    match store.add_message(&caller, &case_id, &note, false, true, vec![file]) {
        Ok(case) => (
            StatusCode::CREATED,
            Json(json!({ "case": case, "manifest": manifest })),
        )
            .into_response(),
        Err(e) => fail(e),
    }
}
