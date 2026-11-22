// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Kubernetes ResourceQuota management. Lists, creates, and deletes the
//! native K8s ResourceQuota object per namespace, so limits actually
//! constrain VM/pod creation rather than being tracked only in the UI
//! (as the deleted `src/api/handlers/quotas.rs` stub did, always returning
//! `Json(vec![])`).

use super::*;
use axum::extract::Json as AxumJson;
use k8s_openapi::api::core::v1::{ResourceQuota, ResourceQuotaSpec};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct QuotaSummary {
    pub name: String,
    pub namespace: String,
    pub hard: BTreeMap<String, String>,
    pub used: BTreeMap<String, String>,
    pub created: Option<String>,
}

fn to_summary(q: ResourceQuota) -> QuotaSummary {
    let namespace = q.metadata.namespace.clone().unwrap_or_default();
    let name = q.metadata.name.clone().unwrap_or_default();
    let created = q
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.to_string());
    let status = q.status.unwrap_or_default();
    // Prefer status.hard (what the API server is actually enforcing); fall
    // back to spec.hard for a quota so new its status hasn't been computed
    // yet.
    let hard = status
        .hard
        .or_else(|| q.spec.and_then(|s| s.hard))
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect();
    let used = status
        .used
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.0))
        .collect();
    QuotaSummary {
        name,
        namespace,
        hard,
        used,
        created,
    }
}

fn quota_error(action: &str, e: impl std::fmt::Display) -> axum::response::Response {
    let raw = sanitize_error(&e);
    let lower = raw.to_ascii_lowercase();
    let (code, kind) = if lower.contains("notfound") || lower.contains("not found") {
        (404, "NOT_FOUND")
    } else if lower.contains("forbidden") || lower.contains("unauthorized") {
        (403, "FORBIDDEN")
    } else if lower.contains("already exists") || lower.contains("conflict") {
        (409, "CONFLICT")
    } else {
        (500, "QUOTA_FAILED")
    };
    let (st, j) = err_json(code, kind, &format!("Failed to {action}: {raw}"));
    (st, j).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListQuotasQuery {
    pub namespace: Option<String>,
}

pub async fn list_quotas_handler(
    State(state): State<SharedState>,
    Query(params): Query<ListQuotasQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = params.namespace.unwrap_or_else(|| s.namespace.clone());
    let client = s.kube_client.client();
    drop(s);

    let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
    match api.list(&ListParams::default()).await {
        Ok(list) => {
            Json(list.items.into_iter().map(to_summary).collect::<Vec<_>>()).into_response()
        }
        Err(e) => quota_error("list quotas", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateQuotaBody {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub hard: BTreeMap<String, String>,
}

pub async fn create_quota_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<CreateQuotaBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "Quota name is required");
        return (st, j).into_response();
    }
    if body.hard.is_empty() {
        let (st, j) = err_json(
            400,
            "INVALID_LIMITS",
            "At least one resource limit is required",
        );
        return (st, j).into_response();
    }

    let s = state.read().await;
    let namespace = body
        .namespace
        .clone()
        .unwrap_or_else(|| s.namespace.clone());
    let client = s.kube_client.client();
    drop(s);

    let hard: BTreeMap<String, Quantity> = body
        .hard
        .iter()
        .map(|(k, v)| (k.clone(), Quantity(v.clone())))
        .collect();
    let quota = ResourceQuota {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(body.name.clone()),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        spec: Some(ResourceQuotaSpec {
            hard: Some(hard),
            ..Default::default()
        }),
        status: None,
    };

    let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
    let result = api.create(&PostParams::default(), &quota).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Create,
        "quota",
        &body.name,
        result.is_ok(),
        result.as_ref().err().map(|e| sanitize_error(e)),
    )
    .await;
    match result {
        Ok(created) => (StatusCode::CREATED, Json(to_summary(created))).into_response(),
        Err(e) => quota_error("create quota", e),
    }
}

pub async fn delete_quota_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = s.kube_client.client();
    drop(s);

    let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
    let result = api.delete(&name, &DeleteParams::default()).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Delete,
        "quota",
        &name,
        result.is_ok(),
        result.as_ref().err().map(|e| sanitize_error(e)),
    )
    .await;
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => quota_error("delete quota", e),
    }
}
