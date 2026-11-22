// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Disk image upload API (see `crate::image_upload`). The body is streamed
//! through to the CDI upload proxy; it is never buffered in memory. These
//! routes live outside the 30 s `TimeoutLayer` that wraps the rest of `/api`
//! because an upload legitimately takes longer.
//!
//! `POST /images/uploads/{name}?size=&namespace=&storage_class=&format=&sha256=`
//! `GET  /images/uploads/{name}?namespace=`   (DataVolume phase / progress)
//! `POST /images/validate?format=`            (header-only format check, small body)

use super::*;
use crate::disk_conversion::{detect_format, validate_image_header, DiskFormat};
use crate::image_upload::{self, ProxyConfig, UploadRequest};
use futures_util::{StreamExt, TryStreamExt};
use serde_json::json;

const FEATURE_ID: &str = "image-upload";

fn gate() -> Option<axum::response::Response> {
    if crate::features::allow_non_ga(FEATURE_ID) {
        return None;
    }
    let (st, j) = err_json(
        403,
        "EXPERIMENTAL_DISABLED",
        "image-upload is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
    );
    Some((st, j).into_response())
}

fn parse_format(v: &Option<String>) -> Result<Option<DiskFormat>, ()> {
    match v.as_deref().filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => DiskFormat::parse(s).map(Some).ok_or(()),
    }
}

fn bad_format() -> axum::response::Response {
    let (st, j) = err_json(400, "INVALID_FORMAT", "unknown image format");
    (st, j).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    pub size: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

pub async fn upload_image_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    auth: Option<axum::Extension<crate::api::auth::AuthIdentity>>,
    Query(q): Query<UploadQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let declared = match parse_format(&q.format) {
        Ok(f) => f,
        Err(()) => return bad_format(),
    };
    let cfg = match ProxyConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            let (st, j) = err_json(503, "UPLOAD_PROXY_NOT_CONFIGURED", &e.to_string());
            return (st, j).into_response();
        }
    };
    let (kube, audit, default_ns) = {
        let s = state.read().await;
        (s.kube_client.clone(), s.audit.clone(), s.namespace.clone())
    };
    let namespace = q.namespace.clone().unwrap_or(default_ns);
    let content_length = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let req = UploadRequest {
        name: name.clone(),
        namespace: namespace.clone(),
        size: q.size,
        storage_class: q.storage_class,
        declared_format: declared,
        expected_sha256: q.sha256,
        max_bytes: image_upload::max_bytes_from_env(),
    };
    let stream = body
        .into_data_stream()
        .map_err(std::io::Error::other)
        .boxed();
    let user = auth
        .as_ref()
        .and_then(|a| a.0.username.clone())
        .unwrap_or_else(|| "api-token".into());
    let result = image_upload::upload_image(&kube, req, &cfg, stream, content_length).await;
    let entry = |success: bool, details: serde_json::Value| crate::audit_trail::AuditEntry {
        id: crate::utils::generate_id("audit", &name),
        timestamp: chrono::Utc::now(),
        user: user.clone(),
        severity: crate::audit_trail::AuditSeverity::Medium,
        action: crate::audit_trail::AuditAction::Import,
        resource_type: "image-upload".into(),
        resource_name: name.clone(),
        namespace: namespace.clone(),
        details,
        ip_address: String::new(),
        success,
    };
    match result {
        Ok(report) => {
            audit.write().await.record(entry(
                true,
                json!({ "bytes": report.bytes, "format": report.format, "sha256": report.sha256 }),
            ));
            (StatusCode::CREATED, Json(json!(report))).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            audit
                .write()
                .await
                .record(entry(false, json!({ "error": msg })));
            let (status, code) = if msg.contains("upload limit") || msg.contains("larger than") {
                (413, "IMAGE_TOO_LARGE")
            } else if msg.contains("sha256 mismatch") {
                (422, "CHECKSUM_MISMATCH")
            } else if msg.contains("format")
                || msg.contains("signature")
                || msg.contains("empty")
                || msg.contains("invalid")
                || msg.contains("must be")
                || msg.contains("declared")
            {
                (400, "INVALID_IMAGE")
            } else {
                (502, "UPLOAD_FAILED")
            };
            let (st, j) = err_json(status, code, &msg);
            (st, j).into_response()
        }
    }
}

/// The effective upload size cap, so clients can show the real limit.
pub fn upload_config_json(max_bytes: u64) -> serde_json::Value {
    json!({ "max_bytes": max_bytes, "default_max_bytes": image_upload::DEFAULT_MAX_BYTES })
}

/// `GET /images/upload-config` -> `{max_bytes, default_max_bytes}`.
pub async fn get_upload_config_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    Json(upload_config_json(image_upload::max_bytes_from_env())).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UploadStatusQuery {
    #[serde(default)]
    pub namespace: Option<String>,
}

pub async fn get_upload_status_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    Query(q): Query<UploadStatusQuery>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let (kube, default_ns) = {
        let s = state.read().await;
        (s.kube_client.clone(), s.namespace.clone())
    };
    let namespace = q.namespace.unwrap_or(default_ns);
    match kube.get_data_volume(&namespace, &name).await {
        Ok(obj) => {
            let p = crate::kube::cdi::data_volume_progress_from_object(&obj);
            Json(json!({
                "name": name,
                "namespace": namespace,
                "phase": crate::kube::cdi::data_volume_phase_from_object(&obj),
                "progress_percent": p.percent,
            }))
            .into_response()
        }
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &format!("DataVolume not found: {e}"));
            (st, j).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ValidateQuery {
    #[serde(default)]
    pub format: Option<String>,
}

/// Header-only check: send the first ~33 KiB of an image (bounded by the global
/// 10 MiB body limit) and get the detected format back.
pub async fn validate_image_handler(
    Query(q): Query<ValidateQuery>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let declared = match parse_format(&q.format) {
        Ok(f) => f,
        Err(()) => return bad_format(),
    };
    match validate_image_header(&body, declared.as_ref()) {
        Ok(f) => Json(json!({
            "valid": true,
            "format": f.name(),
            "detected": detect_format(&body).map(|d| d.name()),
            "converted_to_raw_by_cdi": !matches!(f, DiskFormat::Raw | DiskFormat::Iso),
        }))
        .into_response(),
        Err(msg) => Json(json!({ "valid": false, "error": msg })).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_config_reports_cap_and_default() {
        let v = upload_config_json(5 * 1024 * 1024 * 1024);
        assert_eq!(v["max_bytes"], 5u64 * 1024 * 1024 * 1024);
        assert_eq!(v["default_max_bytes"], image_upload::DEFAULT_MAX_BYTES);
    }
}
