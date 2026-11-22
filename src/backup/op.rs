// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Durable backup operations: a scheduled (or API-queued) backup is an
//! `operations` row that survives restarts. The handler creates the
//! VirtualMachineSnapshot -- reusing it if a previous attempt already did --
//! and reports success only once KubeVirt says the snapshot Succeeded.

use crate::operations::runtime::{HandlerFuture, OpContext, Outcome};
use crate::operations::{NewOperation, Operation, OperationsDb};
use crate::snapshots::{SnapshotConfig, SnapshotManager, SnapshotStatus};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Operation `kind` for a VM backup.
pub const OP_KIND: &str = "backup";
pub const LABEL_RETENTION_DAYS: &str = "zorvia.io/retention-days";

/// A snapshot's own failure deadline is 1h; allow a little more before we
/// give up waiting on it.
const SNAPSHOT_TIMEOUT_SECS: u64 = 3900;

#[derive(Debug, Serialize, Deserialize)]
struct BackupParams {
    namespace: String,
    vm_name: String,
    snapshot_name: String,
    backup_type: String,
    retention_days: Option<u32>,
    description: Option<String>,
}

/// Deterministic snapshot name for an idempotency key, so a retry (or a
/// re-enqueue after a crash) targets the same VirtualMachineSnapshot
/// instead of creating a second one.
fn snapshot_name_for(vm_name: &str, idempotency_key: &str) -> String {
    let digest = Sha256::digest(idempotency_key.as_bytes());
    let suffix: String = digest.iter().take(5).map(|b| format!("{b:02x}")).collect();
    let mut name = format!("backup-{vm_name}-{suffix}");
    name.truncate(253);
    name
}

/// Queue a backup of `vm_name`. A second call with the same
/// `idempotency_key` returns the existing operation (`created == false`).
pub fn enqueue_in(
    db: &OperationsDb,
    namespace: &str,
    vm_name: &str,
    backup_type: &str,
    retention_days: Option<u32>,
    description: Option<String>,
    idempotency_key: &str,
) -> Result<(Operation, bool)> {
    let mut new = NewOperation::new(OP_KIND, vm_name, namespace);
    new.idempotency_key = Some(idempotency_key.to_string());
    new.params = serde_json::to_value(BackupParams {
        namespace: namespace.to_string(),
        vm_name: vm_name.to_string(),
        snapshot_name: snapshot_name_for(vm_name, idempotency_key),
        backup_type: backup_type.to_string(),
        retention_days,
        description,
    })?;
    db.create(new)
}

pub fn enqueue(
    namespace: &str,
    vm_name: &str,
    backup_type: &str,
    retention_days: Option<u32>,
    description: Option<String>,
    idempotency_key: &str,
) -> Result<(Operation, bool)> {
    enqueue_in(
        OperationsDb::global(),
        namespace,
        vm_name,
        backup_type,
        retention_days,
        description,
        idempotency_key,
    )
}

pub fn run_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(run_backup_op(ctx))
}

async fn run_backup_op(ctx: OpContext) -> Outcome {
    let p: BackupParams = match serde_json::from_value(ctx.op.params.clone()) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(format!("invalid operation params: {e}")),
    };
    let manager = match SnapshotManager::new(&p.namespace).await {
        Ok(m) => m,
        Err(e) => return Outcome::Retry(format!("cannot connect to cluster: {e}")),
    };

    ctx.progress("creating-snapshot", 1);
    // Guest-side pre-snapshot hooks (Zyvor agent), recorded with the backup.
    let guest_quiesce =
        crate::guest_rpc::pre_snapshot_hooks(&ctx.client, &p.namespace, &p.vm_name).await;
    if manager.get_snapshot(&p.snapshot_name).await.is_err() {
        let mut config = SnapshotConfig::new(&p.vm_name, &p.snapshot_name)
            .with_label("zorvia.io/backup", "true")
            .with_label("zorvia.io/backup-type", &p.backup_type);
        if let Some(days) = p.retention_days {
            config = config.with_label(LABEL_RETENTION_DAYS, days.to_string());
        }
        if let Some(desc) = &p.description {
            config = config.with_description(desc.clone());
        }
        if let Err(e) = manager.create_snapshot(&config).await {
            // A racing/previous attempt may have created it after our check.
            if manager.get_snapshot(&p.snapshot_name).await.is_err() {
                return Outcome::Retry(format!("failed to create snapshot: {e}"));
            }
        }
    }

    let started = std::time::Instant::now();
    loop {
        if ctx.cancelled() {
            if let Err(e) = manager.delete_snapshot(&p.snapshot_name).await {
                log::warn!("backup {}: cleanup after cancel failed: {e}", ctx.op.id);
            }
            return Outcome::Cancelled;
        }
        match manager.get_snapshot(&p.snapshot_name).await {
            Ok(info) => match info.status {
                SnapshotStatus::Succeeded => {
                    // With an S3 target configured the backup continues
                    // off-cluster; otherwise it stays a cluster-local
                    // snapshot (the previous behaviour).
                    match super::offcluster::OffClusterTarget::resolve().await {
                        Err(e) => return Outcome::Failed(format!("off-cluster target: {e}")),
                        Ok(Some(target)) => {
                            let out = super::offcluster::run(
                                &ctx,
                                &target,
                                &p.namespace,
                                &p.vm_name,
                                &p.snapshot_name,
                            )
                            .await;
                            return match out {
                                Outcome::Succeeded(Some(mut v)) if v.is_object() => {
                                    v["guest_quiesce"] = serde_json::json!(guest_quiesce);
                                    Outcome::Succeeded(Some(v))
                                }
                                other => other,
                            };
                        }
                        Ok(None) => {}
                    }
                    let warning = info.captured_no_volumes().then_some({
                        "VM has no PVC/DataVolume-backed disks; only its configuration was captured"
                    });
                    return Outcome::Succeeded(Some(serde_json::json!({
                        "snapshot": p.snapshot_name,
                        "vm_name": p.vm_name,
                        "warning": warning,
                        "guest_quiesce": guest_quiesce,
                    })));
                }
                SnapshotStatus::Failed => {
                    return Outcome::Failed(
                        info.error
                            .unwrap_or_else(|| "VirtualMachineSnapshot failed".into()),
                    )
                }
                _ => ctx.progress("snapshotting", 50),
            },
            Err(e) => log::debug!("backup {}: snapshot status: {e}", ctx.op.id),
        }
        if started.elapsed() >= Duration::from_secs(SNAPSHOT_TIMEOUT_SECS) {
            return Outcome::Failed(format!(
                "timed out after {SNAPSHOT_TIMEOUT_SECS}s waiting for snapshot '{}'",
                p.snapshot_name
            ));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_name_is_deterministic_and_key_specific() {
        let a = snapshot_name_for("web-1", "sched:nightly:2026-09-30T02:00:00Z:web-1");
        assert_eq!(
            a,
            snapshot_name_for("web-1", "sched:nightly:2026-09-30T02:00:00Z:web-1")
        );
        assert_ne!(
            a,
            snapshot_name_for("web-1", "sched:nightly:2026-10-01T02:00:00Z:web-1")
        );
        assert!(a.starts_with("backup-web-1-"));
    }

    #[test]
    fn enqueue_dedupes_on_idempotency_key() {
        let db = OperationsDb::open(":memory:").unwrap();
        let (a, created) = enqueue_in(&db, "default", "web-1", "full", Some(7), None, "k").unwrap();
        assert!(created);
        let (b, created) = enqueue_in(&db, "default", "web-1", "full", Some(7), None, "k").unwrap();
        assert!(!created);
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, OP_KIND);
        assert_eq!(a.params["snapshot_name"], b.params["snapshot_name"]);
        assert_eq!(a.params["retention_days"], 7);
    }
}
