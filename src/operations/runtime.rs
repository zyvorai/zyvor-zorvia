// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Reconciler for durable operations: claims queued operations, runs the
//! handler registered for their `kind`, heartbeats while they run, and
//! records the outcome. Only the lease leader should call `tick`.

use super::{OpState, Operation, OperationsDb};
use crate::kube::KubeClient;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// A running operation whose row has not been touched for this long is
/// treated as orphaned (its process died) and re-queued.
const ORPHAN_AFTER_SECS: i64 = 120;
const HEARTBEAT_SECS: u64 = 30;

/// What a handler decided about its operation.
pub enum Outcome {
    Succeeded(Option<serde_json::Value>),
    /// Transient failure; retried until the operation's `max_attempts`.
    Retry(String),
    /// Permanent failure.
    Failed(String),
    Cancelled,
}

pub type HandlerFuture = Pin<Box<dyn Future<Output = Outcome> + Send>>;
pub type HandlerFn = fn(OpContext) -> HandlerFuture;

pub struct OpContext {
    pub op: Operation,
    pub db: Arc<OperationsDb>,
    pub client: KubeClient,
}

impl OpContext {
    pub fn progress(&self, phase: &str, pct: u8) {
        if let Err(e) = self.db.set_progress(&self.op.id, phase, pct) {
            log::warn!("operation {}: progress update failed: {e}", self.op.id);
        }
    }

    /// True once cancellation was requested; handlers should stop and
    /// return `Outcome::Cancelled`.
    pub fn cancelled(&self) -> bool {
        matches!(self.db.get(&self.op.id), Ok(Some(o)) if o.cancel_requested)
    }
}

/// Handler for an operation `kind`, if one is registered.
pub fn handler_for(kind: &str) -> Option<HandlerFn> {
    match kind {
        crate::golden_images::convert::OP_KIND => Some(crate::golden_images::convert::run_op),
        crate::backup::op::OP_KIND => Some(crate::backup::op::run_op),
        crate::migration_import::OP_KIND => Some(crate::migration_import::runtime::run_import_op),
        crate::backup::restore::RESTORE_KIND => Some(crate::backup::restore::run_restore_op),
        crate::backup::restore::DRILL_KIND => Some(crate::backup::restore::run_drill_op),
        crate::backup::verify_key::KIND => Some(crate::backup::verify_key::run_op),
        _ => None,
    }
}

/// Operations this process is currently running: id -> kind.
fn in_flight() -> &'static Mutex<HashMap<String, String>> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_in_flight(id: &str) -> bool {
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(id)
}

fn running_of_kind(kind: &str) -> usize {
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|k| k.as_str() == kind)
        .count()
}

/// Cap on simultaneously running operations of a kind, when there is one.
/// Imports run privileged, disk-heavy Jobs, so a wave must not start them all
/// at once.
pub fn max_concurrency(kind: &str) -> Option<usize> {
    match kind {
        crate::migration_import::OP_KIND => Some(
            std::env::var("ZORVIA_IMPORT_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n: &usize| *n >= 1)
                .unwrap_or(2),
        ),
        _ => None,
    }
}

fn may_start(kind: &str, running: usize) -> bool {
    max_concurrency(kind).is_none_or(|cap| running < cap)
}

/// One reconcile pass. Claims queued operations and re-queues orphans.
pub async fn tick(client: &KubeClient) {
    let db = OperationsDb::global().clone();
    let active = match db.list_active() {
        Ok(a) => a,
        Err(e) => {
            log::error!("operations: cannot list active operations: {e}");
            return;
        }
    };
    let now = chrono::Utc::now();
    for op in active {
        if is_in_flight(&op.id) {
            continue;
        }
        match op.state {
            OpState::Queued => {
                if !may_start(&op.kind, running_of_kind(&op.kind)) {
                    continue; // at its concurrency cap; stays queued
                }
                match db.start(&op.id) {
                    Ok(true) => {
                        in_flight()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(op.id.clone(), op.kind.clone());
                        let mut running = op.clone();
                        running.state = OpState::Running;
                        running.attempts += 1;
                        tokio::spawn(run_one(running, db.clone(), client.clone()));
                    }
                    Ok(false) => {}
                    Err(e) => log::error!("operations: cannot claim {}: {e}", op.id),
                }
            }
            OpState::Running => {
                let stale = chrono::DateTime::parse_from_rfc3339(&op.updated)
                    .map(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds())
                    .unwrap_or(i64::MAX);
                if stale > ORPHAN_AFTER_SECS {
                    match db.requeue_orphan(&op.id) {
                        Ok(state) => log::warn!(
                            "operations: {} ({}) orphaned; now {}",
                            op.id,
                            op.kind,
                            state.as_str()
                        ),
                        Err(e) => log::error!("operations: cannot requeue {}: {e}", op.id),
                    }
                }
            }
            _ => {}
        }
    }
}

/// Re-queue operations a previous process left `running`. Call once, by the
/// leader, before the first `tick`.
pub fn recover() {
    match OperationsDb::global().recover_interrupted() {
        Ok(0) => {}
        Ok(n) => log::warn!("operations: recovered {n} interrupted operation(s)"),
        Err(e) => log::error!("operations: recovery failed: {e}"),
    }
}

async fn run_one(op: Operation, db: Arc<OperationsDb>, client: KubeClient) {
    let id = op.id.clone();
    let kind = op.kind.clone();

    let hb_db = db.clone();
    let hb_id = id.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(HEARTBEAT_SECS)).await;
            if let Err(e) = hb_db.touch(&hb_id) {
                log::warn!("operations: heartbeat for {hb_id} failed: {e}");
            }
        }
    });

    let outcome = match handler_for(&kind) {
        // Run in its own task so a panic becomes a retryable failure
        // instead of leaving the operation stuck in `running`.
        Some(h) => {
            let ctx = OpContext {
                op,
                db: db.clone(),
                client,
            };
            match tokio::spawn(h(ctx)).await {
                Ok(o) => o,
                Err(e) => Outcome::Retry(format!("handler panicked: {e}")),
            }
        }
        None => Outcome::Failed(format!("no handler registered for kind '{kind}'")),
    };
    heartbeat.abort();

    let recorded = match outcome {
        Outcome::Succeeded(result) => db.succeed(&id, result.as_ref()),
        Outcome::Retry(e) => db.fail(&id, &e, true).map(|_| ()),
        Outcome::Failed(e) => db.fail(&id, &e, false).map(|_| ()),
        Outcome::Cancelled => db.mark_cancelled(&id),
    };
    if let Err(e) = recorded {
        log::error!("operations: cannot record outcome of {id}: {e}");
    }
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_are_capped_other_kinds_are_not() {
        assert_eq!(max_concurrency("backup"), None);
        assert!(max_concurrency(crate::migration_import::OP_KIND).unwrap() >= 1);
        assert!(may_start("backup", 1000));
        let cap = max_concurrency(crate::migration_import::OP_KIND).unwrap();
        assert!(may_start(crate::migration_import::OP_KIND, cap - 1));
        assert!(!may_start(crate::migration_import::OP_KIND, cap));
    }
}
