// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Off-cluster backup catalog, restore and recovery-drill API. All routes are
//! cluster.admin (see `permissions.rs`). Restores and drills are durable
//! operations; the response carries the operation id to poll on
//! `GET /operations/{id}`.

use super::*;
use crate::backup::restore;
use crate::operations::OperationsDb;
use serde_json::json;

fn unavailable(e: impl std::fmt::Display) -> axum::response::Response {
    let (st, j) = err_json(500, "OPERATIONS_UNAVAILABLE", &sanitize_error(&e));
    (st, j).into_response()
}

pub async fn list_offcluster_backups_handler() -> impl IntoResponse {
    match restore::list_restorable(OperationsDb::global(), 200) {
        Ok(items) => {
            let backups: Vec<_> = items
                .iter()
                .map(|(src, op)| restore::catalog_entry(src, op))
                .collect();
            Json(json!({ "backups": backups })).into_response()
        }
        Err(e) => unavailable(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateOffClusterBackupBody {
    pub vm_name: String,
    /// Defaults to the namespace Zorvia manages.
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /backups/offcluster` -- back a VM up to the configured S3 target now.
/// Queues the same durable `backup` operation the scheduler uses (snapshot, copy
/// to S3, read-back verify) and returns its id to poll on `/operations/{id}`.
pub async fn create_offcluster_backup_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateOffClusterBackupBody>,
) -> impl IntoResponse {
    if !restore::dns_label(&body.vm_name) {
        let (st, j) = err_json(400, "INVALID_NAME", "vm_name must be a DNS label");
        return (st, j).into_response();
    }
    // Refuse up front rather than queue an operation that can only fall back to
    // a cluster-local snapshot.
    match crate::backup::offcluster::OffClusterTarget::resolve().await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let (st, j) = err_json(
                409,
                "NO_OFFCLUSTER_TARGET",
                "no off-cluster target is configured (set ZORVIA_BACKUP_S3_* or ZORVIA_BACKUP_ATLAS_BUCKET)",
            );
            return (st, j).into_response();
        }
        Err(e) => {
            let (st, j) = err_json(409, "OFFCLUSTER_TARGET_INVALID", &sanitize_error(&e));
            return (st, j).into_response();
        }
    }
    let namespace = match body.namespace.clone() {
        Some(ns) => ns,
        None => state.read().await.namespace.clone(),
    };
    if !restore::dns_label(&namespace) {
        let (st, j) = err_json(400, "INVALID_NAMESPACE", "namespace must be a DNS label");
        return (st, j).into_response();
    }
    let key = format!("manual:{}", uuid::Uuid::new_v4().simple());
    match crate::backup::op::enqueue(
        &namespace,
        &body.vm_name,
        "full",
        body.retention_days,
        body.description.clone(),
        &key,
    ) {
        Ok((op, _)) => (
            StatusCode::ACCEPTED,
            Json(json!({ "operation_id": op.id, "state": op.state })),
        )
            .into_response(),
        Err(e) => unavailable(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    pub new_vm_name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    pub start: bool,
    /// `Block` or `Filesystem`; unset restores each disk in the mode it had.
    #[serde(default)]
    pub volume_mode: Option<String>,
}

fn load_source(op_id: &str) -> Result<restore::RestoreSource, Box<axum::response::Response>> {
    match OperationsDb::global().get(op_id) {
        Ok(Some(op)) => restore::source_from_op(&op).map_err(|e| {
            let (st, j) = err_json(409, "NOT_RESTORABLE", &e.to_string());
            Box::new((st, j).into_response())
        }),
        Ok(None) => {
            let (st, j) = err_json(404, "NOT_FOUND", "backup not found");
            Err(Box::new((st, j).into_response()))
        }
        Err(e) => Err(Box::new(unavailable(e))),
    }
}

pub async fn restore_offcluster_handler(
    Path(op_id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> impl IntoResponse {
    let src = match load_source(&op_id) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let namespace = body
        .namespace
        .clone()
        .unwrap_or_else(|| src.source_namespace.clone());
    match restore::enqueue_restore_in(
        OperationsDb::global(),
        &src,
        &namespace,
        &body.new_vm_name,
        body.storage_class.clone(),
        body.start,
        body.volume_mode.clone(),
    ) {
        Ok(op) => (
            StatusCode::ACCEPTED,
            Json(json!({ "operation_id": op.id, "state": op.state })),
        )
            .into_response(),
        Err(e) => {
            let (st, j) = err_json(400, "INVALID", &e.to_string());
            (st, j).into_response()
        }
    }
}

pub async fn drill_offcluster_handler(Path(op_id): Path<String>) -> impl IntoResponse {
    let src = match load_source(&op_id) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match restore::enqueue_drill_in(
        OperationsDb::global(),
        &src,
        &restore::drill_namespace(),
        None,
    ) {
        Ok((op, _)) => (
            StatusCode::ACCEPTED,
            Json(json!({ "operation_id": op.id, "state": op.state })),
        )
            .into_response(),
        Err(e) => {
            let (st, j) = err_json(400, "INVALID", &e.to_string());
            (st, j).into_response()
        }
    }
}

/// `POST /backups/offcluster/{op_id}/verify-key`: prove the configured encryption key
/// still opens this backup (first part of each disk), without restoring it.
pub async fn verify_key_offcluster_handler(Path(op_id): Path<String>) -> impl IntoResponse {
    match crate::backup::verify_key::enqueue_in(OperationsDb::global(), &op_id) {
        Ok(op) => (
            StatusCode::ACCEPTED,
            Json(json!({ "operation_id": op.id, "state": op.state })),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            let code = if msg.contains("not found") { 404 } else { 400 };
            let (st, j) = err_json(code, "INVALID", &msg);
            (st, j).into_response()
        }
    }
}
