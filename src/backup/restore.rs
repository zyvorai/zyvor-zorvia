// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Restore of an off-cluster backup as a durable operation, plus recovery
//! drills (restore into an isolated namespace, boot, check the guest, tear
//! down). Planning and VM rebuilding are pure and unit-tested; the operation
//! handlers drive the cluster.

use super::offcluster::{cleanup, follow_job, Followed, OffClusterTarget};
use crate::kube::backup_job::{build_restore_job, BackupDisk, BackupJobSpec, BackupSecretRef};
use crate::operations::runtime::{HandlerFuture, OpContext, Outcome};
use crate::operations::{NewOperation, OpState, Operation, OperationsDb};
use anyhow::{anyhow, bail, Result};
use k8s_openapi::api::core::v1::{Namespace, PersistentVolumeClaim};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

pub const RESTORE_KIND: &str = "backup-restore";
pub const DRILL_KIND: &str = "backup-drill";
const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreParams {
    pub source_op: String,
    pub namespace: String,
    pub new_vm_name: String,
    pub storage_class: Option<String>,
    pub start: bool,
    /// `Block` or `Filesystem`: the volume mode of the restored disks. Unset = the
    /// mode each disk had when it was backed up.
    #[serde(default)]
    pub volume_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceDisk {
    pub name: String,
    pub size_bytes: u64,
    /// The source volume was Block mode.
    pub block: bool,
}

/// Whether a disk is restored into a Block volume: an explicit request wins,
/// otherwise the disk's original mode.
pub fn target_is_block(requested: Option<&str>, source_block: bool) -> bool {
    match requested {
        Some("Block") => true,
        Some(_) => false,
        None => source_block,
    }
}

/// `Block` / `Filesystem` (case-insensitive) or an error.
pub fn parse_volume_mode(s: &str) -> Result<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Ok("Block"),
        "filesystem" | "fs" => Ok("Filesystem"),
        other => bail!("volume_mode must be Block or Filesystem, not '{other}'"),
    }
}

/// Everything needed to plan a restore, taken from a finished off-cluster
/// backup operation (so the server needs no object-store access to plan).
#[derive(Debug, Clone)]
pub struct RestoreSource {
    pub op_id: String,
    pub source_namespace: String,
    pub vm_name: String,
    pub manifest_key: String,
    pub disks: Vec<SourceDisk>,
    pub vm_spec: Value,
    pub encrypted: bool,
    pub created: String,
}

pub fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Interpret a backup operation as a restore source. Only a succeeded backup
/// that actually went off-cluster qualifies.
pub fn source_from_op(op: &Operation) -> Result<RestoreSource> {
    if op.kind != super::op::OP_KIND {
        bail!("operation {} is not a backup", op.id);
    }
    if op.state != OpState::Succeeded {
        bail!(
            "backup {} has not succeeded (state: {})",
            op.id,
            op.state.as_str()
        );
    }
    let oc = op
        .result
        .as_ref()
        .and_then(|r| r.get("offcluster"))
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            anyhow!(
                "backup {} is cluster-local only; nothing to restore from",
                op.id
            )
        })?;
    let manifest_key = oc
        .get("manifest_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("backup result has no manifest_key"))?
        .to_string();
    let disks: Vec<SourceDisk> = oc
        .get("disks")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    Some(SourceDisk {
                        name: d.get("name")?.as_str()?.to_string(),
                        size_bytes: d.get("size_bytes")?.as_u64()?,
                        block: d
                            .get("source_block")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if disks.is_empty() {
        bail!("backup result lists no disks");
    }
    let vm_spec = oc.get("vm_spec").cloned().unwrap_or(Value::Null);
    if !vm_spec.is_object() {
        bail!("backup result has no VM spec (taken before restore support?)");
    }
    Ok(RestoreSource {
        op_id: op.id.clone(),
        source_namespace: op.namespace.clone(),
        vm_name: op.resource.clone(),
        manifest_key,
        disks,
        vm_spec,
        encrypted: oc
            .get("encrypted")
            .and_then(|e| e.as_bool())
            .unwrap_or(false),
        created: op.completed.clone().unwrap_or_else(|| op.created.clone()),
    })
}

/// Requested PVC size for a restored disk: the image plus 10% and 64 MiB of
/// filesystem headroom, rounded up to whole GiB.
pub fn pvc_size_gi(disk_bytes: u64) -> String {
    let padded = disk_bytes + disk_bytes / 10 + 64 * 1024 * 1024;
    format!("{}Gi", padded.div_ceil(GIB).max(1))
}

/// Requested size of a Block restore target: the raw image rounded up to whole GiB
/// (no filesystem, so no headroom).
pub fn block_pvc_size_gi(disk_bytes: u64) -> String {
    format!("{}Gi", disk_bytes.div_ceil(GIB).max(1))
}

pub fn restore_target_pvc_manifest(
    name: &str,
    namespace: &str,
    size_bytes: u64,
    storage_class: Option<&str>,
    op_id: &str,
    block: bool,
) -> Value {
    let mut spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "volumeMode": if block { "Block" } else { "Filesystem" },
        "resources": {"requests": {"storage": if block {
            block_pvc_size_gi(size_bytes)
        } else {
            pvc_size_gi(size_bytes)
        }}},
    });
    if let Some(sc) = storage_class {
        spec["storageClassName"] = json!(sc);
    }
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "zorvia",
                "app.kubernetes.io/name": "zorvia-restore",
                "zorvia.io/operation": op_id,
            },
        },
        "spec": spec,
    })
}

/// Rebuild a VirtualMachine from a saved spec, pointing the restored disks at
/// their new PVCs. `mapping` is volume name -> PVC name. Drops identity that
/// would collide with the original VM (firmware uuid/serial, MAC addresses)
/// and, for drills (`isolate`), replaces the networks with a bare pod network.
pub fn restored_vm_manifest(
    saved: &Value,
    new_name: &str,
    namespace: &str,
    mapping: &BTreeMap<String, String>,
    op_id: &str,
    start: bool,
    isolate: bool,
) -> Result<Value> {
    let mut spec = saved
        .get("spec")
        .cloned()
        .ok_or_else(|| anyhow!("saved VM has no spec"))?;
    let obj = spec
        .as_object_mut()
        .ok_or_else(|| anyhow!("saved VM spec is not an object"))?;

    obj.remove("running");
    obj.insert(
        "runStrategy".into(),
        json!(if start { "Always" } else { "Halted" }),
    );

    // Volumes: point mapped ones at the restored PVCs.
    let mut replaced_dvs: Vec<String> = Vec::new();
    {
        let vols = obj
            .get_mut("template")
            .and_then(|t| t.get_mut("spec"))
            .and_then(|s| s.get_mut("volumes"))
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| anyhow!("saved VM has no volumes"))?;
        let mut seen = 0usize;
        for v in vols.iter_mut() {
            let Some(name) = v.get("name").and_then(|n| n.as_str()).map(String::from) else {
                continue;
            };
            let Some(pvc) = mapping.get(&name) else {
                continue;
            };
            seen += 1;
            if let Some(dv) = v.pointer("/dataVolume/name").and_then(|n| n.as_str()) {
                replaced_dvs.push(dv.to_string());
            }
            *v = json!({"name": name, "persistentVolumeClaim": {"claimName": pvc}});
        }
        if seen != mapping.len() {
            bail!("backup contains disks that are not volumes of the saved VM");
        }
    }

    // DataVolumeTemplates that fed the replaced volumes must go, or KubeVirt
    // would create (and re-import) them next to the restored PVCs.
    if let Some(tpls) = obj
        .get_mut("dataVolumeTemplates")
        .and_then(|t| t.as_array_mut())
    {
        tpls.retain(|t| {
            t.pointer("/metadata/name")
                .and_then(|n| n.as_str())
                .map(|n| !replaced_dvs.iter().any(|d| d == n))
                .unwrap_or(true)
        });
    }
    if obj
        .get("dataVolumeTemplates")
        .and_then(|t| t.as_array())
        .is_some_and(|a| a.is_empty())
    {
        obj.remove("dataVolumeTemplates");
    }

    let tspec = obj
        .get_mut("template")
        .and_then(|t| t.get_mut("spec"))
        .and_then(|s| s.as_object_mut())
        .ok_or_else(|| anyhow!("saved VM has no template spec"))?;
    if let Some(fw) = tspec
        .get_mut("domain")
        .and_then(|d| d.get_mut("firmware"))
        .and_then(|f| f.as_object_mut())
    {
        fw.remove("uuid");
        fw.remove("serial");
    }
    if let Some(ifaces) = tspec
        .get_mut("domain")
        .and_then(|d| d.pointer_mut("/devices/interfaces"))
        .and_then(|i| i.as_array_mut())
    {
        for i in ifaces {
            if let Some(o) = i.as_object_mut() {
                o.remove("macAddress");
            }
        }
    }
    if isolate {
        tspec.insert("networks".into(), json!([{"name": "default", "pod": {}}]));
        if let Some(devices) = tspec
            .get_mut("domain")
            .and_then(|d| d.get_mut("devices"))
            .and_then(|d| d.as_object_mut())
        {
            devices.insert(
                "interfaces".into(),
                json!([{"name": "default", "masquerade": {}}]),
            );
        }
    }
    // Keep Service-style selectors consistent with the new name.
    if let Some(labels) = obj
        .get_mut("template")
        .and_then(|t| t.pointer_mut("/metadata/labels"))
        .and_then(|l| l.as_object_mut())
    {
        if labels.contains_key("kubevirt.io/vm") {
            labels.insert("kubevirt.io/vm".into(), json!(new_name));
        }
    }

    let mut labels = saved
        .pointer("/metadata/labels")
        .and_then(|l| l.as_object())
        .cloned()
        .unwrap_or_default();
    labels.insert("app.kubernetes.io/managed-by".into(), json!("zorvia"));
    labels.insert("zorvia.io/restored-from".into(), json!(op_id));
    if isolate {
        labels.insert("zorvia.io/drill".into(), json!("true"));
    }
    Ok(json!({
        "apiVersion": "kubevirt.io/v1",
        "kind": "VirtualMachine",
        "metadata": {"name": new_name, "namespace": namespace, "labels": labels},
        "spec": spec,
    }))
}

/// Queue a restore of `source` as `new_vm_name`.
pub fn enqueue_restore_in(
    db: &OperationsDb,
    source: &RestoreSource,
    namespace: &str,
    new_vm_name: &str,
    storage_class: Option<String>,
    start: bool,
    volume_mode: Option<String>,
) -> Result<Operation> {
    if !dns_label(new_vm_name) {
        bail!("new_vm_name must be a DNS label (lowercase letters, digits, '-')");
    }
    let volume_mode = volume_mode
        .as_deref()
        .map(parse_volume_mode)
        .transpose()?
        .map(str::to_string);
    if !dns_label(namespace) {
        bail!("invalid namespace");
    }
    let dup = db.list_active()?.into_iter().any(|o| {
        o.kind == RESTORE_KIND
            && o.namespace == namespace
            && o.params.get("new_vm_name").and_then(|n| n.as_str()) == Some(new_vm_name)
    });
    if dup {
        bail!("a restore to '{new_vm_name}' is already in progress");
    }
    let mut new = NewOperation::new(RESTORE_KIND, new_vm_name, namespace);
    new.max_attempts = 1;
    new.params = serde_json::to_value(RestoreParams {
        source_op: source.op_id.clone(),
        namespace: namespace.to_string(),
        new_vm_name: new_vm_name.to_string(),
        storage_class,
        start,
        volume_mode,
    })?;
    Ok(db.create(new)?.0)
}

/// Queue a recovery drill of `source` in `drill_namespace`. The idempotency
/// key makes scheduled drills run at most once per slot.
pub fn enqueue_drill_in(
    db: &OperationsDb,
    source: &RestoreSource,
    drill_namespace: &str,
    idempotency_key: Option<String>,
) -> Result<(Operation, bool)> {
    if !dns_label(drill_namespace) {
        bail!("invalid drill namespace");
    }
    let short: String = source
        .op_id
        .trim_start_matches("op-")
        .chars()
        .take(10)
        .collect();
    let mut new = NewOperation::new(DRILL_KIND, &source.vm_name, drill_namespace);
    new.max_attempts = 1;
    new.idempotency_key = idempotency_key;
    new.params = serde_json::to_value(RestoreParams {
        source_op: source.op_id.clone(),
        namespace: drill_namespace.to_string(),
        new_vm_name: format!("drill-{short}"),
        storage_class: None,
        start: true,
        volume_mode: None,
    })?;
    db.create(new)
}

pub fn drill_namespace() -> String {
    std::env::var("ZORVIA_DRILL_NAMESPACE")
        .ok()
        .filter(|s| dns_label(s))
        .unwrap_or_else(|| "zorvia-drill".to_string())
}

/// Summary of a restorable backup (no VM spec: it can hold cloud-init data).
pub fn catalog_entry(src: &RestoreSource, op: &Operation) -> Value {
    json!({
        "operation_id": src.op_id,
        "vm_name": src.vm_name,
        "namespace": src.source_namespace,
        "created": src.created,
        "manifest_key": src.manifest_key,
        "encrypted": src.encrypted,
        "disks": src.disks.iter().map(|d| json!({"name": d.name, "size_bytes": d.size_bytes})).collect::<Vec<_>>(),
        "locked_until": op.result.as_ref().and_then(|r| r.pointer("/offcluster/locked_until")).cloned().unwrap_or(Value::Null),
        "verified": op.result.as_ref().and_then(|r| r.pointer("/offcluster/verified")).cloned().unwrap_or(Value::Null),
    })
}

/// Restorable off-cluster backups, newest first.
pub fn list_restorable(db: &OperationsDb, limit: usize) -> Result<Vec<(RestoreSource, Operation)>> {
    Ok(db
        .list(Some(super::op::OP_KIND), limit.clamp(1, 500))?
        .into_iter()
        .filter_map(|op| source_from_op(&op).ok().map(|s| (s, op)))
        .collect())
}

/// Latest restorable backup per (namespace, VM).
pub fn latest_per_vm(items: Vec<(RestoreSource, Operation)>) -> Vec<RestoreSource> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (src, _) in items {
        // `list` is newest-first, so the first hit per VM is the latest.
        if seen.insert((src.source_namespace.clone(), src.vm_name.clone())) {
            out.push(src);
        }
    }
    out
}

/// Does a guest's serial-console log show that userspace booted? Used when the
/// guest has no agent (most minimal images), so a drill can still prove the
/// restored disk boots. Recognises a login prompt or systemd reaching its
/// multi-user target; ANSI colour codes are ignored.
pub fn console_shows_boot(log: &str) -> bool {
    let mut clean = String::with_capacity(log.len());
    let mut chars = log.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            clean.push(c);
        }
    }
    let lower = clean.to_ascii_lowercase();
    lower.contains(" login:") || lower.contains("reached target multi-user.target")
}

// ---------------------------------------------------------------- handlers

pub(crate) fn vm_api(client: &kube::Client, ns: &str) -> Api<DynamicObject> {
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "kubevirt.io",
        "v1",
        "VirtualMachine",
    ));
    Api::namespaced_with(client.clone(), ns, &ar)
}

pub(super) fn short_id(op_id: &str) -> String {
    op_id.trim_start_matches("op-").chars().take(10).collect()
}

/// Everything after the disks are written: what was created, for cleanup.
struct Restored {
    pvcs: Vec<VolumeTarget>,
    job_name: String,
    mapping: BTreeMap<String, String>,
}

struct VolumeTarget {
    pvc_name: String,
}

async fn restore_disks(
    ctx: &OpContext,
    target: &OffClusterTarget,
    src: &RestoreSource,
    p: &RestoreParams,
    span: (u32, u32),
) -> std::result::Result<Restored, Outcome> {
    let client = ctx.client.client();
    let ns = p.namespace.as_str();
    let short = short_id(&ctx.op.id);

    if src.encrypted && target.encryption_key_key.is_none() {
        return Err(Outcome::Failed(
            "the backup is encrypted but ZORVIA_BACKUP_ENCRYPTION_KEY_KEY is not configured".into(),
        ));
    }

    let vms = vm_api(&client, ns);
    if vms.get(&p.new_vm_name).await.is_ok() {
        return Err(Outcome::Failed(format!(
            "a VM named '{}' already exists in namespace '{ns}'",
            p.new_vm_name
        )));
    }

    ctx.progress("creating volumes", 5);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), ns);
    let mut mapping = BTreeMap::new();
    let mut targets = Vec::new();
    let job_name = format!("zorvia-restore-{short}");
    for d in &src.disks {
        let pvc_name = format!("rs-{short}-{}", d.name);
        let manifest = restore_target_pvc_manifest(
            &pvc_name,
            ns,
            d.size_bytes,
            p.storage_class.as_deref(),
            &ctx.op.id,
            target_is_block(p.volume_mode.as_deref(), d.block),
        );
        let pvc: PersistentVolumeClaim = match serde_json::from_value(manifest) {
            Ok(p) => p,
            Err(e) => return Err(Outcome::Failed(format!("bad PVC manifest: {e}"))),
        };
        targets.push(VolumeTarget {
            pvc_name: pvc_name.clone(),
        });
        mapping.insert(d.name.clone(), pvc_name.clone());
        if let Err(e) = pvcs.create(&PostParams::default(), &pvc).await {
            cleanup_targets(&client, ns, &job_name, &targets).await;
            return Err(Outcome::Failed(format!(
                "cannot create PVC {pvc_name}: {e}"
            )));
        }
    }

    let spec = BackupJobSpec {
        backup_id: src.op_id.clone(),
        vm_name: src.vm_name.clone(),
        snapshot_name: String::new(),
        disks: src
            .disks
            .iter()
            .map(|d| BackupDisk {
                name: d.name.clone(),
                pvc: mapping[&d.name].clone(),
                // Block target: the agent writes the raw device; Filesystem: a disk.img file.
                block: target_is_block(p.volume_mode.as_deref(), d.block),
            })
            .collect(),
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
        active_deadline_secs: target.active_deadline_secs,
    };
    let job = match build_restore_job(&job_name, ns, &src.manifest_key, &spec) {
        Ok(j) => j,
        Err(e) => {
            cleanup_targets(&client, ns, &job_name, &targets).await;
            return Err(Outcome::Failed(e.to_string()));
        }
    };
    let jobs: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), ns);
    if let Err(e) = jobs.create(&PostParams::default(), &job).await {
        cleanup_targets(&client, ns, &job_name, &targets).await;
        return Err(Outcome::Failed(format!("cannot create restore Job: {e}")));
    }

    let followed = follow_job(
        ctx,
        &client,
        ns,
        &job_name,
        target.active_deadline_secs,
        span.0,
        span.1,
    )
    .await;
    let restored = Restored {
        pvcs: targets,
        job_name: job_name.clone(),
        mapping,
    };
    match followed {
        Followed::Done(_) => {
            // Job no longer needed; the PVCs now belong to the VM.
            cleanup(&client, ns, &job_name, &[]).await;
            Ok(restored)
        }
        Followed::Failed(e) => {
            cleanup_restored(&client, ns, &restored).await;
            Err(Outcome::Failed(e))
        }
        Followed::Cancelled => {
            cleanup_restored(&client, ns, &restored).await;
            Err(Outcome::Cancelled)
        }
    }
}

async fn cleanup_targets(client: &kube::Client, ns: &str, job_name: &str, t: &[VolumeTarget]) {
    let jobs: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), ns);
    let _ = jobs.delete(job_name, &DeleteParams::background()).await;
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), ns);
    for v in t {
        let _ = pvcs.delete(&v.pvc_name, &DeleteParams::default()).await;
    }
}

async fn cleanup_restored(client: &kube::Client, ns: &str, r: &Restored) {
    cleanup_targets(client, ns, &r.job_name, &r.pvcs).await;
}

pub(super) fn load_source(
    ctx: &OpContext,
    p: &RestoreParams,
) -> std::result::Result<RestoreSource, Outcome> {
    let src_op = match ctx.db.get(&p.source_op) {
        Ok(Some(o)) => o,
        Ok(None) => return Err(Outcome::Failed(format!("backup {} not found", p.source_op))),
        Err(e) => return Err(Outcome::Retry(format!("operations store: {e}"))),
    };
    source_from_op(&src_op).map_err(|e| Outcome::Failed(e.to_string()))
}

pub(super) async fn target_config() -> std::result::Result<OffClusterTarget, Outcome> {
    match OffClusterTarget::resolve().await {
        Ok(Some(t)) => Ok(t),
        Ok(None) => Err(Outcome::Failed(
            "no off-cluster target is configured (ZORVIA_BACKUP_S3_*)".into(),
        )),
        Err(e) => Err(Outcome::Failed(format!("off-cluster target: {e}"))),
    }
}

pub fn run_restore_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(async move {
        match restore_op(&ctx).await {
            Ok(o) | Err(o) => o,
        }
    })
}

async fn restore_op(ctx: &OpContext) -> std::result::Result<Outcome, Outcome> {
    let p: RestoreParams = serde_json::from_value(ctx.op.params.clone())
        .map_err(|e| Outcome::Failed(format!("invalid operation params: {e}")))?;
    let src = load_source(ctx, &p)?;
    let target = target_config().await?;
    let client = ctx.client.client();

    let restored = restore_disks(ctx, &target, &src, &p, (10, 80)).await?;

    ctx.progress("creating VM", 92);
    let manifest = restored_vm_manifest(
        &src.vm_spec,
        &p.new_vm_name,
        &p.namespace,
        &restored.mapping,
        &ctx.op.id,
        p.start,
        false,
    )
    .map_err(|e| Outcome::Failed(format!("cannot rebuild the VM: {e}")))?;
    let vm: DynamicObject = serde_json::from_value(manifest)
        .map_err(|e| Outcome::Failed(format!("bad VM manifest: {e}")))?;
    if let Err(e) = vm_api(&client, &p.namespace)
        .create(&PostParams::default(), &vm)
        .await
    {
        cleanup_restored(&client, &p.namespace, &restored).await;
        return Ok(Outcome::Failed(format!("cannot create VM: {e}")));
    }
    Ok(Outcome::Succeeded(Some(json!({
        "vm_name": p.new_vm_name,
        "namespace": p.namespace,
        "restored_from": src.op_id,
        "volumes": restored.mapping,
        "started": p.start,
    }))))
}

/// NetworkPolicy isolating restored guests in the drill namespace: denies all ingress and
/// egress for virt-launcher pods only, never for the restore Job's pods.
pub fn drill_isolation_policy(ns: &str) -> Value {
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": {"name": "zorvia-drill-isolation", "namespace": ns},
        "spec": {
            "podSelector": {"matchLabels": {"kubevirt.io": "virt-launcher"}},
            "policyTypes": ["Ingress", "Egress"],
        },
    })
}

async fn ensure_drill_namespace(client: &kube::Client, ns: &str) -> Result<()> {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    if namespaces.get(ns).await.is_err() {
        let n: Namespace = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Namespace",
            "metadata": {"name": ns, "labels": {"zorvia.io/drill": "true"}},
        }))?;
        if let Err(e) = namespaces.create(&PostParams::default(), &n).await {
            if !matches!(&e, kube::Error::Api(ae) if ae.code == 409) {
                return Err(e.into());
            }
        }
    }
    // Isolate the restored *guest*: deny all traffic in and out of virt-launcher pods so it
    // cannot reach (or impersonate) production. Only those pods -- the restore Job in the
    // same namespace must still reach the object store (a namespace-wide deny-all was
    // observed to time out its S3 requests).
    let policies: Api<NetworkPolicy> = Api::namespaced(client.clone(), ns);
    let policy: NetworkPolicy = serde_json::from_value(drill_isolation_policy(ns))?;
    // Replace rather than patch: an older, broader policy left by a previous version must be
    // corrected, and Zorvia's RBAC deliberately allows create/delete (not patch) on
    // NetworkPolicies. Nothing is running in the namespace's guest pods yet, so the
    // brief gap is harmless.
    match policies
        .delete("zorvia-drill-isolation", &DeleteParams::default())
        .await
    {
        Ok(_) => {}
        Err(kube::Error::Api(ae)) if ae.code == 404 => {}
        Err(e) => return Err(e.into()),
    }
    policies.create(&PostParams::default(), &policy).await?;
    Ok(())
}

pub fn run_drill_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(async move {
        match drill_op(&ctx).await {
            Ok(o) | Err(o) => o,
        }
    })
}

async fn drill_op(ctx: &OpContext) -> std::result::Result<Outcome, Outcome> {
    let p: RestoreParams = serde_json::from_value(ctx.op.params.clone())
        .map_err(|e| Outcome::Failed(format!("invalid operation params: {e}")))?;
    let src = load_source(ctx, &p)?;
    let target = target_config().await?;
    let client = ctx.client.client();
    let ns = p.namespace.as_str();

    ctx.progress("preparing isolated namespace", 2);
    ensure_drill_namespace(&client, ns)
        .await
        .map_err(|e| Outcome::Failed(format!("cannot prepare drill namespace: {e}")))?;

    let started = std::time::Instant::now();
    let restored = restore_disks(ctx, &target, &src, &p, (5, 70)).await?;
    let restore_secs = started.elapsed().as_secs();

    let outcome = drill_boot(ctx, &client, &src, &p, &restored, restore_secs).await;

    // Always tear the drill down: VM first, then its volumes.
    let _ = vm_api(&client, ns)
        .delete(&p.new_vm_name, &DeleteParams::default())
        .await;
    cleanup_restored(&client, ns, &restored).await;
    outcome
}

/// Read the VM's serial-console log from its virt-launcher pod (KubeVirt's
/// `guest-console-log` container) and check it for boot evidence.
async fn serial_console_shows_boot(client: &kube::Client, ns: &str, vm: &str) -> bool {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{ListParams, LogParams};
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let list = match pods
        .list(&ListParams::default().labels(&format!("vm.kubevirt.io/name={vm}")))
        .await
    {
        Ok(l) => l.items,
        Err(_) => return false,
    };
    let Some(name) = list.first().and_then(|p| p.metadata.name.clone()) else {
        return false;
    };
    let lp = LogParams {
        container: Some("guest-console-log".into()),
        tail_lines: Some(400),
        ..Default::default()
    };
    pods.logs(&name, &lp)
        .await
        .map(|l| console_shows_boot(&l))
        .unwrap_or(false)
}

async fn drill_boot(
    ctx: &OpContext,
    client: &kube::Client,
    src: &RestoreSource,
    p: &RestoreParams,
    restored: &Restored,
    restore_secs: u64,
) -> std::result::Result<Outcome, Outcome> {
    let ns = p.namespace.as_str();
    ctx.progress("booting restored VM", 78);
    let manifest = restored_vm_manifest(
        &src.vm_spec,
        &p.new_vm_name,
        ns,
        &restored.mapping,
        &ctx.op.id,
        true,
        true,
    )
    .map_err(|e| Outcome::Failed(format!("cannot rebuild the VM: {e}")))?;
    let vm: DynamicObject = serde_json::from_value(manifest)
        .map_err(|e| Outcome::Failed(format!("bad VM manifest: {e}")))?;
    let vms = vm_api(client, ns);
    vms.create(&PostParams::default(), &vm)
        .await
        .map_err(|e| Outcome::Failed(format!("cannot create drill VM: {e}")))?;
    // Explicit start, in case the runStrategy was rewritten by admission.
    let _ = vms
        .patch(
            &p.new_vm_name,
            &PatchParams::default(),
            &Patch::Merge(&json!({"spec": {"runStrategy": "Always"}})),
        )
        .await;

    let timeout = Duration::from_secs(
        std::env::var("ZORVIA_DRILL_BOOT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600),
    );
    let boot_started = std::time::Instant::now();
    let mut last_reason = String::from("VM did not start");
    let mut booted = false;
    loop {
        if ctx.cancelled() {
            return Ok(Outcome::Cancelled);
        }
        if let Ok(report) = ctx.client.guest_ready_report(ns, &p.new_vm_name).await {
            booted |= report.running;
            last_reason = report.reason.clone();
            if report.running && report.ready {
                // Evidence the guest itself is up: its agent, or a login
                // prompt / multi-user target on the serial console (most
                // minimal images have no agent).
                let agent = report.agent_connected;
                let console = !agent && serial_console_shows_boot(client, ns, &p.new_vm_name).await;
                if agent || console {
                    // Beyond "the agent is connected": ask the Zyvor agent itself.
                    let guest_probe = if agent {
                        crate::guest_rpc::probe(&ctx.client, ns, &p.new_vm_name).await
                    } else {
                        None
                    };
                    return Ok(Outcome::Succeeded(Some(json!({
                        "drill": {
                            "guest_probe": guest_probe,
                            "backup_operation": src.op_id,
                            "vm_name": src.vm_name,
                            "booted": true,
                            "guest_agent": agent,
                            "evidence": if agent { "guest agent" } else { "serial console" },
                            "restore_seconds": restore_secs,
                            "boot_seconds": boot_started.elapsed().as_secs(),
                        }
                    }))));
                }
            }
        }
        if boot_started.elapsed() >= timeout {
            return Ok(Outcome::Failed(format!(
                "drill failed: {} after {}s ({})",
                if booted {
                    "VM started but neither the guest agent nor a console login/boot target appeared"
                } else {
                    "VM never reached Running"
                },
                timeout.as_secs(),
                last_reason
            )));
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_vm() -> Value {
        json!({
            "apiVersion": "kubevirt.io/v1", "kind": "VirtualMachine",
            "metadata": {"name": "web-1", "labels": {"app": "web"}},
            "spec": {
                "running": true,
                "dataVolumeTemplates": [
                    {"metadata": {"name": "web-1-root"}, "spec": {"source": {"http": {"url": "x"}}}},
                    {"metadata": {"name": "other"}, "spec": {}}
                ],
                "template": {
                    "metadata": {"labels": {"kubevirt.io/vm": "web-1", "tier": "fe"}},
                    "spec": {
                        "domain": {
                            "firmware": {"uuid": "abc", "serial": "s1", "bootloader": {"efi": {}}},
                            "devices": {"interfaces": [{"name": "br", "bridge": {}, "macAddress": "02:00:00:00:00:01"}]},
                        },
                        "networks": [{"name": "br", "multus": {"networkName": "prod-vlan"}}],
                        "volumes": [
                            {"name": "rootdisk", "dataVolume": {"name": "web-1-root"}},
                            {"name": "data", "persistentVolumeClaim": {"claimName": "web-data"}},
                            {"name": "cloudinit", "cloudInitNoCloud": {"userData": "#cloud-config"}}
                        ]
                    }
                }
            }
        })
    }

    fn mapping() -> BTreeMap<String, String> {
        [("rootdisk", "rs-1-rootdisk"), ("data", "rs-1-data")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn rebuilds_vm_with_restored_volumes_and_no_colliding_identity() {
        let m = restored_vm_manifest(
            &saved_vm(),
            "web-1-restored",
            "default",
            &mapping(),
            "op-1",
            false,
            false,
        )
        .unwrap();
        assert_eq!(m["metadata"]["name"], "web-1-restored");
        assert_eq!(m["metadata"]["labels"]["zorvia.io/restored-from"], "op-1");
        assert_eq!(m["metadata"]["labels"]["app"], "web");
        let s = &m["spec"];
        assert_eq!(s["runStrategy"], "Halted");
        assert!(s.get("running").is_none());
        let vols = s["template"]["spec"]["volumes"].as_array().unwrap();
        assert_eq!(
            vols[0]["persistentVolumeClaim"]["claimName"],
            "rs-1-rootdisk"
        );
        assert!(vols[0].get("dataVolume").is_none());
        assert_eq!(vols[1]["persistentVolumeClaim"]["claimName"], "rs-1-data");
        // Unmapped volumes are untouched.
        assert!(vols[2].get("cloudInitNoCloud").is_some());
        // The DataVolumeTemplate that fed the replaced volume is gone, others stay.
        let tpls = s["dataVolumeTemplates"].as_array().unwrap();
        assert_eq!(tpls.len(), 1);
        assert_eq!(tpls[0]["metadata"]["name"], "other");
        // Identity that would collide with the original is dropped.
        let fw = &s["template"]["spec"]["domain"]["firmware"];
        assert!(fw.get("uuid").is_none() && fw.get("serial").is_none());
        assert!(fw.get("bootloader").is_some());
        assert!(s["template"]["spec"]["domain"]["devices"]["interfaces"][0]
            .get("macAddress")
            .is_none());
        assert_eq!(
            s["template"]["metadata"]["labels"]["kubevirt.io/vm"],
            "web-1-restored"
        );
        // Network config is kept as-is for a real restore.
        assert!(s["template"]["spec"]["networks"][0].get("multus").is_some());
    }

    #[test]
    fn start_flag_and_drill_isolation() {
        let m = restored_vm_manifest(
            &saved_vm(),
            "drill-x",
            "zorvia-drill",
            &mapping(),
            "op-1",
            true,
            true,
        )
        .unwrap();
        assert_eq!(m["spec"]["runStrategy"], "Always");
        assert_eq!(m["metadata"]["labels"]["zorvia.io/drill"], "true");
        let t = &m["spec"]["template"]["spec"];
        assert_eq!(t["networks"], json!([{"name": "default", "pod": {}}]));
        assert_eq!(
            t["domain"]["devices"]["interfaces"],
            json!([{"name": "default", "masquerade": {}}])
        );
    }

    #[test]
    fn rejects_mapping_for_unknown_volumes_and_missing_spec() {
        let mut bad = mapping();
        bad.insert("ghost".into(), "rs-1-ghost".into());
        assert!(restored_vm_manifest(&saved_vm(), "x", "d", &bad, "op-1", false, false).is_err());
        assert!(
            restored_vm_manifest(&json!({}), "x", "d", &mapping(), "op-1", false, false).is_err()
        );
    }

    #[test]
    fn sizes_pvcs_with_headroom() {
        assert_eq!(pvc_size_gi(1), "1Gi");
        assert_eq!(pvc_size_gi(10 * GIB), "12Gi"); // 10Gi + 10% + 64Mi
        assert_eq!(pvc_size_gi(20 * GIB), "23Gi");
        let m = restore_target_pvc_manifest(
            "rs-1-root",
            "d",
            10 * GIB,
            Some("longhorn"),
            "op-1",
            false,
        );
        assert_eq!(m["spec"]["storageClassName"], "longhorn");
        assert_eq!(m["spec"]["volumeMode"], "Filesystem");
        assert_eq!(m["spec"]["resources"]["requests"]["storage"], "12Gi");
        let m = restore_target_pvc_manifest("rs-1-root", "d", GIB, None, "op-1", false);
        assert!(m["spec"].get("storageClassName").is_none());
    }

    #[test]
    fn block_targets_are_raw_and_sized_without_filesystem_headroom() {
        assert_eq!(block_pvc_size_gi(GIB), "1Gi");
        assert_eq!(block_pvc_size_gi(GIB + 1), "2Gi");
        assert_eq!(block_pvc_size_gi(10 * GIB), "10Gi");
        let m = restore_target_pvc_manifest("rs-1-data", "d", 10 * GIB, Some("rbd"), "op-1", true);
        assert_eq!(m["spec"]["volumeMode"], "Block");
        assert_eq!(m["spec"]["resources"]["requests"]["storage"], "10Gi");
        assert_eq!(m["spec"]["storageClassName"], "rbd");
    }

    #[test]
    fn restore_mode_defaults_to_the_source_and_can_be_overridden() {
        assert!(target_is_block(None, true));
        assert!(!target_is_block(None, false));
        assert!(
            !target_is_block(Some("Filesystem"), true),
            "explicit Filesystem wins"
        );
        assert!(target_is_block(Some("Block"), false), "explicit Block wins");
        assert_eq!(parse_volume_mode("block").unwrap(), "Block");
        assert_eq!(parse_volume_mode(" Filesystem ").unwrap(), "Filesystem");
        assert!(parse_volume_mode("raw").is_err());
    }

    fn backup_op(db: &OperationsDb, offcluster: Option<Value>, done: bool) -> Operation {
        let (op, _) = db
            .create(NewOperation::new(
                super::super::op::OP_KIND,
                "web-1",
                "default",
            ))
            .unwrap();
        db.start(&op.id).unwrap();
        if done {
            let result = match offcluster {
                Some(oc) => json!({"snapshot": "s", "offcluster": oc}),
                None => json!({"snapshot": "s"}),
            };
            db.succeed(&op.id, Some(&result)).unwrap();
        }
        db.get(&op.id).unwrap().unwrap()
    }

    fn oc() -> Value {
        json!({"success": true, "manifest_key": "web-1/op/manifest.json", "encrypted": true,
               "disks": [{"name": "rootdisk", "size_bytes": 1000, "stored_bytes": 1028}],
               "vm_spec": saved_vm()})
    }

    #[test]
    fn only_finished_offcluster_backups_are_restorable() {
        let db = OperationsDb::open(":memory:").unwrap();
        let good = backup_op(&db, Some(oc()), true);
        let src = source_from_op(&good).unwrap();
        assert_eq!(src.manifest_key, "web-1/op/manifest.json");
        assert_eq!(
            src.disks,
            vec![SourceDisk {
                name: "rootdisk".into(),
                size_bytes: 1000,
                block: false,
            }]
        );
        assert!(src.encrypted);

        assert!(source_from_op(&backup_op(&db, None, true)).is_err()); // cluster-local
        assert!(source_from_op(&backup_op(&db, Some(oc()), false)).is_err()); // not finished
        let mut no_spec = oc();
        no_spec.as_object_mut().unwrap().remove("vm_spec");
        assert!(source_from_op(&backup_op(&db, Some(no_spec), true)).is_err());
        let other = db
            .create(NewOperation::new("golden-image-convert", "vm", "default"))
            .unwrap()
            .0;
        assert!(source_from_op(&other).is_err());
    }

    #[test]
    fn enqueue_validates_and_blocks_duplicate_restores() {
        let db = OperationsDb::open(":memory:").unwrap();
        let src = source_from_op(&backup_op(&db, Some(oc()), true)).unwrap();
        assert!(enqueue_restore_in(&db, &src, "default", "Bad_Name", None, false, None).is_err());
        assert!(
            enqueue_restore_in(
                &db,
                &src,
                "default",
                "web-1-x",
                None,
                false,
                Some("raw".into())
            )
            .is_err(),
            "unknown volume_mode is refused before queuing"
        );
        let op = enqueue_restore_in(
            &db,
            &src,
            "default",
            "web-1-new",
            None,
            true,
            Some("block".into()),
        )
        .unwrap();
        assert_eq!(op.params["volume_mode"], "Block");
        assert_eq!(op.kind, RESTORE_KIND);
        assert_eq!(op.max_attempts, 1);
        assert_eq!(op.params["source_op"], src.op_id);
        assert!(enqueue_restore_in(&db, &src, "default", "web-1-new", None, false, None).is_err());
    }

    #[test]
    fn drills_dedupe_per_slot_and_pick_latest_backup() {
        let db = OperationsDb::open(":memory:").unwrap();
        let first = source_from_op(&backup_op(&db, Some(oc()), true)).unwrap();
        let second = source_from_op(&backup_op(&db, Some(oc()), true)).unwrap();
        let (a, created) =
            enqueue_drill_in(&db, &first, "zorvia-drill", Some("drill:x:1".into())).unwrap();
        let (b, created2) =
            enqueue_drill_in(&db, &first, "zorvia-drill", Some("drill:x:1".into())).unwrap();
        assert!(created && !created2);
        assert_eq!(a.id, b.id);
        assert!(a.params["new_vm_name"]
            .as_str()
            .unwrap()
            .starts_with("drill-"));
        assert_eq!(a.params["start"], true);

        let listed = list_restorable(&db, 10).unwrap();
        assert_eq!(listed.len(), 2);
        let latest = latest_per_vm(listed);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].op_id, second.op_id);
        assert!(enqueue_drill_in(&db, &first, "Bad NS", None).is_err());
    }

    #[test]
    fn detects_a_booted_guest_from_its_serial_console() {
        // Real systemd output (ANSI colours included) from a restored Debian guest.
        let booted = "[\u{1b}[0;32m  OK  \u{1b}[0m] Reached target \u{1b}[0;1;39mmulti-user.target\u{1b}[0m - Multi-User System.\n";
        assert!(console_shows_boot(booted));
        assert!(console_shows_boot(
            "Debian GNU/Linux 12 cap-src ttyS0\n\ncap-src login: "
        ));
        // Still booting, or nothing at all.
        assert!(!console_shows_boot(
            "[    1.2] Freeing unused kernel image memory\nStarting systemd-udevd\n"
        ));
        assert!(!console_shows_boot(""));
    }

    #[test]
    fn drill_isolation_targets_guest_pods_only() {
        let p = drill_isolation_policy("zorvia-drill");
        // A namespace-wide selector ({}) would also cut off the restore Job.
        assert_eq!(
            p["spec"]["podSelector"]["matchLabels"]["kubevirt.io"],
            "virt-launcher"
        );
        assert_ne!(p["spec"]["podSelector"], json!({}));
        assert_eq!(p["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
        // No allow rules: the selected pods get no traffic at all.
        assert!(p["spec"].get("ingress").is_none() && p["spec"].get("egress").is_none());
        assert_eq!(p["metadata"]["namespace"], "zorvia-drill");
    }

    #[test]
    fn catalog_hides_the_vm_spec() {
        let db = OperationsDb::open(":memory:").unwrap();
        let op = backup_op(&db, Some(oc()), true);
        let src = source_from_op(&op).unwrap();
        let e = catalog_entry(&src, &op);
        assert!(e.get("vm_spec").is_none());
        assert_eq!(e["disks"][0]["name"], "rootdisk");
        assert_eq!(e["encrypted"], true);
    }
}
