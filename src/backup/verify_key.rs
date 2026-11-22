// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! "Is my encryption key still good?" as a durable operation.
//!
//! A backup is only as recoverable as its key. This runs the backup agent in
//! `verify-key` mode against a finished off-cluster backup: it reads the manifest
//! and the first part of each disk and proves the configured key decrypts it. No
//! volumes, no VM, a few tens of MiB of reads, minutes less than a recovery drill.
//! It does not replace a drill (which proves the data restores and boots).

use super::offcluster::{cleanup, follow_job, Followed};
use super::restore::{
    dns_label, load_source, short_id, source_from_op, target_config, RestoreParams,
};
use crate::kube::backup_job::{build_verify_key_job, BackupJobSpec, BackupSecretRef};
use crate::operations::runtime::{HandlerFuture, OpContext, Outcome};
use crate::operations::{NewOperation, Operation, OperationsDb};
use anyhow::{bail, Result};
use kube::api::{Api, PostParams};
use serde_json::json;

pub const KIND: &str = "backup-verify-key";

/// Queue a key check of the backup finished by operation `source_op_id`, run in
/// the namespace of the VM it came from (that is where the credential Secret is).
pub fn enqueue_in(db: &OperationsDb, source_op_id: &str) -> Result<Operation> {
    let Some(src_op) = db.get(source_op_id)? else {
        bail!("backup {source_op_id} not found");
    };
    let src = source_from_op(&src_op)?;
    if !dns_label(&src.source_namespace) {
        bail!("invalid namespace");
    }
    let dup = db.list_active()?.into_iter().any(|o| {
        o.kind == KIND && o.params.get("source_op").and_then(|n| n.as_str()) == Some(source_op_id)
    });
    if dup {
        bail!("a key check of this backup is already in progress");
    }
    let mut new = NewOperation::new(KIND, &src.vm_name, &src.source_namespace);
    new.max_attempts = 1;
    new.params = serde_json::to_value(RestoreParams {
        source_op: src.op_id.clone(),
        namespace: src.source_namespace.clone(),
        new_vm_name: String::new(),
        storage_class: None,
        start: false,
        volume_mode: None,
    })?;
    Ok(db.create(new)?.0)
}

pub fn run_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(async move {
        match verify(&ctx).await {
            Ok(o) | Err(o) => o,
        }
    })
}

async fn verify(ctx: &OpContext) -> std::result::Result<Outcome, Outcome> {
    let p: RestoreParams = serde_json::from_value(ctx.op.params.clone())
        .map_err(|e| Outcome::Failed(format!("invalid operation params: {e}")))?;
    let src = load_source(ctx, &p)?;
    let target = target_config().await?;
    if src.encrypted && target.encryption_key_key.is_none() {
        return Err(Outcome::Failed(
            "the backup is encrypted but ZORVIA_BACKUP_ENCRYPTION_KEY_KEY is not configured".into(),
        ));
    }
    let client = ctx.client.client();
    let ns = p.namespace.as_str();
    let job_name = format!("zorvia-verify-key-{}", short_id(&ctx.op.id));

    let spec = BackupJobSpec {
        backup_id: src.op_id.clone(),
        vm_name: src.vm_name.clone(),
        snapshot_name: String::new(),
        disks: vec![],
        env: target.agent_env(),
        secrets: BackupSecretRef {
            secret_name: target.secret_name.clone(),
            access_key_key: target.access_key_key.clone(),
            secret_key_key: target.secret_key_key.clone(),
            encryption_key_key: src
                .encrypted
                .then(|| target.encryption_key_key.clone())
                .flatten(),
        },
        active_deadline_secs: 900,
    };
    let job = build_verify_key_job(&job_name, ns, &src.manifest_key, &spec)
        .map_err(|e| Outcome::Failed(e.to_string()))?;
    ctx.progress("starting key check", 5);
    let jobs: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), ns);
    jobs.create(&PostParams::default(), &job)
        .await
        .map_err(|e| Outcome::Failed(format!("cannot create key-check Job: {e}")))?;

    let followed = follow_job(
        ctx,
        &client,
        ns,
        &job_name,
        spec.active_deadline_secs,
        10,
        85,
    )
    .await;
    cleanup(&client, ns, &job_name, &[]).await;
    match followed {
        Followed::Done(result) => Ok(Outcome::Succeeded(Some(json!({
            "backup": src.op_id,
            "vm": src.vm_name,
            "encrypted": src.encrypted,
            "key_check": result,
        })))),
        Followed::Failed(e) => Ok(Outcome::Failed(format!("key check failed: {e}"))),
        Followed::Cancelled => Ok(Outcome::Cancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_backup_is_rejected_without_queuing() {
        let db = OperationsDb::open(":memory:").unwrap();
        let e = enqueue_in(&db, "op-missing").unwrap_err().to_string();
        assert!(e.contains("not found"), "{e}");
        assert!(db.list_active().unwrap().is_empty());
    }
}
