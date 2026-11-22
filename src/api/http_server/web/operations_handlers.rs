// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Durable operations API and the reconciler loop that drives them.
//! See `crate::operations`.

use super::*;
use crate::operations::{runtime, OpState, OperationsDb};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct ListOperationsQuery {
    kind: Option<String>,
    limit: Option<usize>,
}

pub async fn list_operations_handler(
    axum::extract::Query(q): axum::extract::Query<ListOperationsQuery>,
) -> impl IntoResponse {
    match OperationsDb::global().list(q.kind.as_deref(), q.limit.unwrap_or(50)) {
        Ok(ops) => Json(json!({ "operations": ops })).into_response(),
        Err(e) => {
            let (st, j) = err_json(
                500,
                "OPERATIONS_UNAVAILABLE",
                &sanitize_error(&e.to_string()),
            );
            (st, j).into_response()
        }
    }
}

pub async fn get_operation_handler(Path(id): Path<String>) -> impl IntoResponse {
    match OperationsDb::global().get(&id) {
        Ok(Some(op)) => Json(json!(op)).into_response(),
        Ok(None) => {
            let (st, j) = err_json(404, "NOT_FOUND", "operation not found");
            (st, j).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(
                500,
                "OPERATIONS_UNAVAILABLE",
                &sanitize_error(&e.to_string()),
            );
            (st, j).into_response()
        }
    }
}

pub async fn cancel_operation_handler(Path(id): Path<String>) -> impl IntoResponse {
    match OperationsDb::global().request_cancel(&id) {
        Ok(Some(state)) => {
            let code = if state == OpState::Running { 202 } else { 200 };
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
            (status, Json(json!({ "id": id, "state": state }))).into_response()
        }
        Ok(None) => {
            let (st, j) = err_json(404, "NOT_FOUND", "operation not found");
            (st, j).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(
                500,
                "OPERATIONS_UNAVAILABLE",
                &sanitize_error(&e.to_string()),
            );
            (st, j).into_response()
        }
    }
}

/// Reconciler loop. Only the lease leader acts; it first re-queues work a
/// previous process left running, then claims and runs queued operations.
pub fn spawn_operations_reconciler(state: SharedState) {
    tokio::spawn(async move {
        let mut recovered = false;
        let mut ticks: u64 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if crate::api::leader::skip_if_follower("operations") {
                continue;
            }
            if !recovered {
                runtime::recover();
                recovered = true;
            }
            let client = state.read().await.client();
            runtime::tick(&client).await;

            ticks += 1;
            // Every ~10 minutes: queue due recovery drills.
            if ticks % 120 == 1 {
                schedule_due_drills();
            }
            // Hourly: expire cluster-local backup snapshots past retention.
            if ticks % 720 == 1 {
                let namespace = state.read().await.namespace.clone();
                match crate::backup::retention::sweep_local(&namespace).await {
                    Ok(0) => {}
                    Ok(n) => log::info!("retention: removed {n} expired backup snapshot(s)"),
                    Err(e) => log::warn!("retention sweep failed: {e}"),
                }
            }
        }
    });
}

/// With `ZORVIA_DRILL_INTERVAL_HOURS` set, restore-test the latest off-cluster
/// backup of every VM once per interval. The idempotency key (backup id +
/// interval slot) makes repeated calls within a slot no-ops.
fn schedule_due_drills() {
    let Some(hours) = std::env::var("ZORVIA_DRILL_INTERVAL_HOURS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|h| *h >= 1)
    else {
        return;
    };
    let db = OperationsDb::global();
    let slot = chrono::Utc::now().timestamp() / (hours * 3600);
    let items = match crate::backup::restore::list_restorable(db, 500) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("drill scheduler: {e}");
            return;
        }
    };
    let ns = crate::backup::restore::drill_namespace();
    for src in crate::backup::restore::latest_per_vm(items) {
        let key = format!("drill:{}:{slot}", src.op_id);
        match crate::backup::restore::enqueue_drill_in(db, &src, &ns, Some(key)) {
            Ok((op, true)) => log::info!(
                "drill scheduler: queued drill {} for {}",
                op.id,
                src.vm_name
            ),
            Ok(_) => {}
            Err(e) => log::warn!(
                "drill scheduler: cannot queue drill for {}: {e}",
                src.vm_name
            ),
        }
    }
}
