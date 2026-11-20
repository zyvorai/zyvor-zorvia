// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Capacity reconciliation and invoice records. Records only: there is no
//! payment provider, nothing is charged, and no card data is handled.
//!
//! Org isolation lives in `crate::commercial::billing`; `/admin/` routes
//! require `users.admin`, and the local collector needs `cluster.admin`
//! (see permissions.rs).

use super::commercial_handlers::{caller_of, fail, store, with, Auth};
use super::*;
use crate::commercial::billing::{NewInvoice, NewObservation};
use crate::commercial::capacity;
use axum::extract::Json as AxumJson;
use axum::response::Response;
use serde_json::json;

pub async fn billing_invoices_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.list_invoices(c))
}

pub async fn billing_invoice_get(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.get_invoice(c, &id))
}

pub async fn billing_invoice_document(auth: Auth, Path(id): Path<String>) -> Response {
    let caller = match caller_of(&auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let store = match store() {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match store.invoice_document(&caller, &id) {
        Ok(text) => (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    "text/plain; charset=utf-8".to_string(),
                ),
                (
                    header::CONTENT_DISPOSITION,
                    format!(
                        "attachment; filename=\"invoice-{}.txt\"",
                        id.chars()
                            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                            .collect::<String>()
                    ),
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            text,
        )
            .into_response(),
        Err(e) => fail(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReconcileQuery {
    pub contract_id: String,
    /// RFC 3339; defaults to now.
    pub as_of: Option<String>,
}

pub async fn billing_capacity_reconcile(auth: Auth, Query(q): Query<ReconcileQuery>) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        let as_of = q
            .as_of
            .as_deref()
            .map(|v| chrono::DateTime::parse_from_rfc3339(v).map(|t| t.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|_| {
                crate::commercial::store::CommercialError::Invalid("as_of must be RFC 3339".into())
            })?;
        s.reconcile_capacity(c, &q.contract_id, as_of)
    })
}

/// Observe the local cluster's node counts and record them. A failed or
/// empty discovery is recorded as an *error* observation (unknown), never
/// as zero.
pub async fn billing_capacity_observe(State(state): State<SharedState>, auth: Auth) -> Response {
    let caller = match caller_of(&auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let store = match store() {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let client = state.read().await.client().client();
    let observed = match capacity::observe_local(&client).await {
        Ok(o) => o,
        Err(e) => {
            let (st, j) = err_json(502, "CLUSTER_IDENTITY_UNAVAILABLE", &e);
            return (st, j).into_response();
        }
    };
    let input = match observed.counts {
        Ok(c) if c.total > 0 => NewObservation {
            cluster_id: observed.cluster_id.clone(),
            source: "collector".into(),
            status: "ok".into(),
            total_nodes: Some(c.total),
            control_plane_nodes: Some(c.control_plane),
            worker_nodes: Some(c.workers),
            error: None,
            observed_at: None,
        },
        other => NewObservation {
            cluster_id: observed.cluster_id.clone(),
            source: "collector".into(),
            status: "error".into(),
            total_nodes: None,
            control_plane_nodes: None,
            worker_nodes: None,
            error: Some(match other {
                Err(e) => e,
                Ok(_) => "discovery returned no nodes; treated as unknown".into(),
            }),
            observed_at: None,
        },
    };
    match store.record_observation(&caller, input) {
        Ok(obs) => (
            StatusCode::CREATED,
            Json(json!({ "cluster_id": observed.cluster_id, "observation": obs })),
        )
            .into_response(),
        Err(e) => fail(e),
    }
}

// ── service desk ─────────────────────────────────────────────────

pub async fn billing_admin_observation_record(
    auth: Auth,
    AxumJson(body): AxumJson<NewObservation>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.record_observation(c, body)
    })
}

#[derive(Debug, Deserialize)]
pub struct ObservationsQuery {
    pub cluster_id: Option<String>,
}

pub async fn billing_admin_observations_list(
    auth: Auth,
    Query(q): Query<ObservationsQuery>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.list_observations(c, q.cluster_id.as_deref())
    })
}

pub async fn billing_admin_invoice_create(
    auth: Auth,
    AxumJson(body): AxumJson<NewInvoice>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| s.create_invoice(c, body))
}

pub async fn billing_admin_invoice_issue(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.issue_invoice(c, &id))
}

#[derive(Debug, Deserialize)]
pub struct PaidBody {
    pub payment_reference: String,
}

pub async fn billing_admin_invoice_paid(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<PaidBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.mark_invoice_paid(c, &id, &body.payment_reference)
    })
}

#[derive(Debug, Deserialize)]
pub struct VoidBody {
    pub reason: String,
}

pub async fn billing_admin_invoice_void(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<VoidBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.void_invoice(c, &id, &body.reason)
    })
}
