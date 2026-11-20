// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Off-cluster stage of a backup operation: once a VirtualMachineSnapshot has
//! succeeded, restore each of its volume snapshots into a temporary PVC, run
//! the `backup-agent` Job against them, follow its JSON progress lines from
//! the pod logs, and clean everything up. Opt-in: with no S3 target
//! configured (`ZORVIA_BACKUP_S3_*`) a backup stays cluster-local.
//!
//! The credential Secret is *not* created by Zorvia (that would need
//! cluster-wide Secret access): it must already exist in the VM's namespace.

use crate::kube::backup_job::{
    build_backup_job, BackupDisk, BackupJobSpec, BackupSecretRef, DISKS_ROOT,
};
use crate::operations::runtime::{OpContext, Outcome};
use anyhow::{anyhow, bail, Result};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use serde_json::{json, Value};
use std::time::Duration;

/// Where and how backups leave the cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct OffClusterTarget {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    /// Secret (in the VM's namespace) holding the credentials.
    pub secret_name: String,
    pub access_key_key: String,
    pub secret_key_key: String,
    pub encryption_key_key: Option<String>,
    pub encryption_key_id: String,
    pub lock_mode: Option<String>,
    pub lock_days: Option<u32>,
    pub part_mb: Option<u32>,
    pub active_deadline_secs: i64,
}

impl OffClusterTarget {
    /// `Ok(None)` when no target is configured at all; an error when it is
    /// configured incompletely (better to fail loudly than silently keep a
    /// backup in-cluster).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let opt = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let endpoint = opt("ZORVIA_BACKUP_S3_ENDPOINT");
        let bucket = opt("ZORVIA_BACKUP_S3_BUCKET");
        if endpoint.is_none() && bucket.is_none() {
            return Ok(None);
        }
        let endpoint = endpoint.ok_or_else(|| anyhow!("ZORVIA_BACKUP_S3_ENDPOINT is not set"))?;
        let bucket = bucket.ok_or_else(|| anyhow!("ZORVIA_BACKUP_S3_BUCKET is not set"))?;
        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
            bail!("ZORVIA_BACKUP_S3_ENDPOINT must start with http:// or https://");
        }
        let secret_name = opt("ZORVIA_BACKUP_S3_SECRET")
            .ok_or_else(|| anyhow!("ZORVIA_BACKUP_S3_SECRET is not set"))?;

        let lock_mode = opt("ZORVIA_BACKUP_LOCK_MODE")
            .filter(|m| !m.eq_ignore_ascii_case("none"))
            .map(|m| m.to_ascii_uppercase());
        if let Some(m) = &lock_mode {
            if m != "GOVERNANCE" && m != "COMPLIANCE" {
                bail!("ZORVIA_BACKUP_LOCK_MODE must be GOVERNANCE, COMPLIANCE or none");
            }
        }
        let num = |k: &str| -> Result<Option<u32>> {
            opt(k)
                .map(|v| v.parse().map_err(|_| anyhow!("{k} must be a number")))
                .transpose()
        };
        let lock_days = num("ZORVIA_BACKUP_LOCK_DAYS")?;
        if lock_mode.is_some() && lock_days.is_none() {
            bail!("ZORVIA_BACKUP_LOCK_DAYS is required when a lock mode is set");
        }

        Ok(Some(Self {
            endpoint,
            region: opt("ZORVIA_BACKUP_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            bucket,
            prefix: opt("ZORVIA_BACKUP_S3_PREFIX").unwrap_or_default(),
            secret_name,
            access_key_key: opt("ZORVIA_BACKUP_S3_ACCESS_KEY_KEY")
                .unwrap_or_else(|| "access-key".into()),
            secret_key_key: opt("ZORVIA_BACKUP_S3_SECRET_KEY_KEY")
                .unwrap_or_else(|| "secret-key".into()),
            encryption_key_key: opt("ZORVIA_BACKUP_ENCRYPTION_KEY_KEY"),
            encryption_key_id: opt("ZORVIA_BACKUP_ENCRYPTION_KEY_ID")
                .unwrap_or_else(|| "default".into()),
            lock_mode,
            lock_days,
            part_mb: num("ZORVIA_BACKUP_PART_MB")?,
            active_deadline_secs: num("ZORVIA_BACKUP_JOB_DEADLINE_SECS")?
                .map(i64::from)
                .unwrap_or(6 * 3600),
        }))
    }

    pub fn from_env() -> Result<Option<Self>> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// The configured target. With `ZORVIA_BACKUP_ATLAS_BUCKET` set, the
    /// endpoint, bucket, region and credential Secret come from that Atlas
    /// bucket (`ATLAS_URL` must be set); everything else (prefix, encryption,
    /// Object Lock, part size) still comes from `ZORVIA_BACKUP_*`.
    pub async fn resolve() -> Result<Option<Self>> {
        let Some(id) = std::env::var(ATLAS_BUCKET_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
        else {
            return Self::from_env();
        };
        let client = crate::atlas::Client::from_env()?.ok_or_else(|| {
            anyhow!("{ATLAS_BUCKET_ENV} is set but ATLAS_URL is not, so Atlas cannot be reached")
        })?;
        let bucket = client
            .get_bucket(id.trim())
            .await
            .map_err(|e| anyhow!("cannot read Atlas bucket '{}': {e}", id.trim()))?;
        let overlay = atlas_overlay(&bucket)?;
        Self::from_lookup(|k| overlay.get(k).cloned().or_else(|| std::env::var(k).ok()))
    }

    /// Non-secret `BACKUP_*` env for the agent Job.
    pub fn agent_env(&self) -> Vec<(String, String)> {
        let mut e = vec![
            ("BACKUP_S3_ENDPOINT".to_string(), self.endpoint.clone()),
            ("BACKUP_S3_REGION".into(), self.region.clone()),
            ("BACKUP_S3_BUCKET".into(), self.bucket.clone()),
            ("BACKUP_S3_PREFIX".into(), self.prefix.clone()),
            (
                "BACKUP_ENCRYPTION_KEY_ID".into(),
                self.encryption_key_id.clone(),
            ),
        ];
        if let (Some(mode), Some(days)) = (&self.lock_mode, self.lock_days) {
            e.push(("BACKUP_LOCK_MODE".into(), mode.clone()));
            e.push(("BACKUP_RETAIN_DAYS".into(), days.to_string()));
        }
        if let Some(mb) = self.part_mb {
            e.push(("BACKUP_PART_MB".into(), mb.to_string()));
        }
        e
    }
}

/// Environment variable naming the Atlas bucket to use as the backup target.
pub const ATLAS_BUCKET_ENV: &str = "ZORVIA_BACKUP_ATLAS_BUCKET";

/// Turn an Atlas bucket record into the `ZORVIA_BACKUP_*` settings it implies.
/// Atlas buckets are Rook ObjectBucketClaims, whose credential Secret uses the
/// `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` keys; explicit
/// `ZORVIA_BACKUP_S3_*_KEY` settings still win (they are not overridden here).
pub fn atlas_overlay(bucket: &Value) -> Result<std::collections::HashMap<String, String>> {
    let text = |k: &str| {
        bucket
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
    };
    let id = text("id").unwrap_or("?");
    if text("state") != Some("bound") {
        bail!(
            "Atlas bucket {id} is not bound (state: {}); wait for provisioning to finish",
            text("state").unwrap_or("unknown")
        );
    }
    let endpoint = text("endpoint").ok_or_else(|| anyhow!("Atlas bucket {id} has no endpoint"))?;
    let name =
        text("bucket_name").ok_or_else(|| anyhow!("Atlas bucket {id} has no bucket_name"))?;
    let secret = text("secret_ref")
        .ok_or_else(|| anyhow!("Atlas bucket {id} has no secret_ref (credentials Secret)"))?;

    let mut m = std::collections::HashMap::new();
    m.insert(
        "ZORVIA_BACKUP_S3_ENDPOINT".to_string(),
        endpoint.to_string(),
    );
    m.insert("ZORVIA_BACKUP_S3_BUCKET".to_string(), name.to_string());
    m.insert("ZORVIA_BACKUP_S3_SECRET".to_string(), secret.to_string());
    if let Some(r) = text("region") {
        m.insert("ZORVIA_BACKUP_S3_REGION".to_string(), r.to_string());
    }
    if std::env::var("ZORVIA_BACKUP_S3_ACCESS_KEY_KEY").is_err() {
        m.insert(
            "ZORVIA_BACKUP_S3_ACCESS_KEY_KEY".to_string(),
            "AWS_ACCESS_KEY_ID".to_string(),
        );
    }
    if std::env::var("ZORVIA_BACKUP_S3_SECRET_KEY_KEY").is_err() {
        m.insert(
            "ZORVIA_BACKUP_S3_SECRET_KEY_KEY".to_string(),
            "AWS_SECRET_ACCESS_KEY".to_string(),
        );
    }
    Ok(m)
}

/// One volume of the snapshot to copy off-cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeRestore {
    pub volume_name: String,
    pub pvc_name: String,
    pub volume_snapshot: String,
    pub pvc_spec: Value,
}

impl VolumeRestore {
    /// A Block-mode volume is mounted as a raw device, not as a filesystem.
    pub fn is_block(&self) -> bool {
        self.pvc_spec.get("volumeMode").and_then(|m| m.as_str()) == Some("Block")
    }
}

fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Bytes in a Kubernetes quantity string: `11811160064`, `11Gi`, `500M`, `1.5Gi`.
pub fn quantity_bytes(q: &str) -> Option<u64> {
    let q = q.trim();
    let split = q
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(q.len());
    let (num, suffix) = q.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match suffix {
        "" => 1.0,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024.0,
        "Mi" => 1024.0_f64.powi(2),
        "Gi" => 1024.0_f64.powi(3),
        "Ti" => 1024.0_f64.powi(4),
        "Pi" => 1024.0_f64.powi(5),
        "Ei" => 1024.0_f64.powi(6),
        _ => return None,
    };
    Some((n * mult).ceil() as u64)
}

/// Raise a claim's storage request to at least the snapshot's restore size. A
/// claim smaller than the snapshot it restores from is refused by the CSI
/// provisioner ("requested volume size ... is smaller than ..."), and storage
/// backends routinely round a snapshot up (seen live: a 10.6 GiB request from
/// CDI against an 11 GiB Ceph RBD snapshot).
pub fn ensure_size_at_least(pvc_spec: &mut Value, restore_size: &str) {
    let Some(want) = quantity_bytes(restore_size) else {
        return;
    };
    let current = pvc_spec
        .pointer("/resources/requests/storage")
        .and_then(|v| v.as_str())
        .and_then(quantity_bytes);
    if current.is_none_or(|c| c < want) {
        pvc_spec["resources"]["requests"]["storage"] = json!(restore_size);
    }
}

/// Work out which volumes to restore from a VirtualMachineSnapshotContent.
/// Volumes KubeVirt did not snapshot (no `volumeSnapshotName`) are skipped;
/// block-mode volumes are kept and flagged (`VolumeRestore::is_block`): the agent
/// reads the raw device, and a restore writes it back as a `disk.img` file.
pub fn plan_restores(content: &Value, short_id: &str) -> Result<Vec<VolumeRestore>> {
    let backups = content
        .pointer("/spec/volumeBackups")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for vb in backups {
        let Some(vs) = vb.get("volumeSnapshotName").and_then(|v| v.as_str()) else {
            continue;
        };
        let volume = vb
            .get("volumeName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("volumeBackup without volumeName"))?;
        if !dns_label(volume) {
            bail!("volume name '{volume}' cannot be used as a disk name");
        }
        let src = vb
            .pointer("/persistentVolumeClaim/spec")
            .ok_or_else(|| anyhow!("volume '{volume}' has no PVC spec in the snapshot"))?;
        // Whitelist: never carry over volumeName/dataSource/etc. from the
        // original claim.
        let mut spec = serde_json::Map::new();
        for k in ["accessModes", "resources", "storageClassName", "volumeMode"] {
            if let Some(v) = src.get(k) {
                spec.insert(k.to_string(), v.clone());
            }
        }
        spec.insert(
            "dataSource".into(),
            json!({
                "apiGroup": "snapshot.storage.k8s.io",
                "kind": "VolumeSnapshot",
                "name": vs,
            }),
        );
        let mut pvc_name = format!("bk-{short_id}-{volume}");
        pvc_name.truncate(253);
        out.push(VolumeRestore {
            volume_name: volume.to_string(),
            pvc_name,
            volume_snapshot: vs.to_string(),
            pvc_spec: Value::Object(spec),
        });
    }
    Ok(out)
}

pub fn restore_pvc_manifest(r: &VolumeRestore, namespace: &str, backup_id: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": r.pvc_name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "zorvia",
                "app.kubernetes.io/name": "zorvia-backup",
                "zorvia.io/backup-id": backup_id,
            },
        },
        "spec": r.pvc_spec,
    })
}

#[derive(Debug, Default, PartialEq)]
pub struct AgentOutput {
    pub progress: Option<(u8, String)>,
    pub result: Option<Value>,
}

/// Parse the agent's stdout: progress lines (`{"progress":N,"phase":".."}`)
/// and the final result line (has a `success` field). Non-JSON lines are
/// ignored.
pub fn parse_agent_logs(logs: &str) -> AgentOutput {
    let mut out = AgentOutput::default();
    for line in logs.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("success").is_some() {
            out.result = Some(v);
        } else if let Some(p) = v.get("progress").and_then(|p| p.as_u64()) {
            let phase = v
                .get("phase")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            out.progress = Some((p.min(100) as u8, phase));
        }
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum JobState {
    Running,
    Succeeded,
    Failed(String),
}

pub fn job_state(job: &Job) -> JobState {
    let Some(status) = &job.status else {
        return JobState::Running;
    };
    if status.succeeded.unwrap_or(0) > 0 {
        return JobState::Succeeded;
    }
    for c in status.conditions.iter().flatten() {
        if c.type_ == "Failed" && c.status == "True" {
            let why = c
                .message
                .clone()
                .or_else(|| c.reason.clone())
                .unwrap_or_else(|| "job failed".into());
            return JobState::Failed(why);
        }
    }
    JobState::Running
}

/// A pod stuck because its config can't be satisfied (typically the
/// credential Secret is missing) will never start; report it now instead of
/// waiting out the deadline.
pub fn pod_config_error(pod: &Pod) -> Option<String> {
    pod.status
        .as_ref()?
        .container_statuses
        .iter()
        .flatten()
        .find_map(|cs| {
            let w = cs.state.as_ref()?.waiting.as_ref()?;
            (w.reason.as_deref() == Some("CreateContainerConfigError")).then(|| {
                format!(
                    "backup pod cannot start: {} (is the credential Secret present in this namespace?)",
                    w.message.clone().unwrap_or_default()
                )
            })
        })
}

fn is_conflict(e: &kube::Error) -> bool {
    matches!(e, kube::Error::Api(ae) if ae.code == 409)
}

pub(crate) async fn cleanup(
    client: &kube::Client,
    namespace: &str,
    job_name: &str,
    restores: &[VolumeRestore],
) {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    if let Err(e) = jobs.delete(job_name, &DeleteParams::background()).await {
        if !matches!(&e, kube::Error::Api(ae) if ae.code == 404) {
            log::warn!("backup cleanup: deleting Job {job_name}: {e}");
        }
    }
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    for r in restores {
        if let Err(e) = pvcs.delete(&r.pvc_name, &DeleteParams::default()).await {
            if !matches!(&e, kube::Error::Api(ae) if ae.code == 404) {
                log::warn!("backup cleanup: deleting PVC {}: {e}", r.pvc_name);
            }
        }
    }
}

/// Run the off-cluster stage for an already-succeeded snapshot.
pub async fn run(
    ctx: &OpContext,
    target: &OffClusterTarget,
    namespace: &str,
    vm_name: &str,
    snapshot_name: &str,
) -> Outcome {
    let client = ctx.client.client();
    let backup_id = ctx.op.id.clone();
    let short = backup_id
        .trim_start_matches("op-")
        .chars()
        .take(10)
        .collect::<String>();
    let job_name = format!("zorvia-backup-{short}");

    // 1. Which volume snapshots does the VM snapshot hold?
    ctx.progress("preparing", 10);
    let snaps: Api<crate::snapshots::crds::VirtualMachineSnapshot> =
        Api::namespaced(client.clone(), namespace);
    let content_name = match snaps.get(snapshot_name).await {
        Ok(s) => s.status.and_then(|st| st.content_name),
        Err(e) => return Outcome::Retry(format!("cannot read snapshot {snapshot_name}: {e}")),
    };
    let Some(content_name) = content_name else {
        return Outcome::Retry("snapshot has no content yet".into());
    };
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "snapshot.kubevirt.io",
        "v1beta1",
        "VirtualMachineSnapshotContent",
    ));
    let contents: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
    let content = match contents.get(&content_name).await {
        Ok(c) => c,
        Err(e) => return Outcome::Retry(format!("cannot read snapshot content: {e}")),
    };
    let mut restores = match plan_restores(&content.data, &short) {
        Ok(r) => r,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    // Size each temporary claim to at least its snapshot's restore size.
    let vs_ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "snapshot.storage.k8s.io",
        "v1",
        "VolumeSnapshot",
    ));
    let volume_snapshots: Api<DynamicObject> =
        Api::namespaced_with(client.clone(), namespace, &vs_ar);
    for r in restores.iter_mut() {
        match volume_snapshots.get(&r.volume_snapshot).await {
            Ok(vs) => {
                let size = vs.data.pointer("/status/restoreSize").and_then(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                });
                if let Some(size) = size {
                    ensure_size_at_least(&mut r.pvc_spec, &size);
                }
            }
            Err(e) => log::warn!(
                "backup: cannot read VolumeSnapshot {} to size its claim: {e}",
                r.volume_snapshot
            ),
        }
    }
    if restores.is_empty() {
        return Outcome::Succeeded(Some(json!({
            "snapshot": snapshot_name,
            "vm_name": vm_name,
            "offcluster": null,
            "warning": "the snapshot holds no PVC-backed volumes, so nothing was uploaded off-cluster",
        })));
    }

    // 2. Restore each volume snapshot into a temporary PVC.
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    for r in &restores {
        let manifest = restore_pvc_manifest(r, namespace, &backup_id);
        let pvc: PersistentVolumeClaim = match serde_json::from_value(manifest) {
            Ok(p) => p,
            Err(e) => return Outcome::Failed(format!("bad PVC manifest: {e}")),
        };
        match pvcs.create(&PostParams::default(), &pvc).await {
            Ok(_) => {}
            Err(e) if is_conflict(&e) => {}
            Err(e) => {
                cleanup(&client, namespace, &job_name, &restores).await;
                return Outcome::Retry(format!("cannot create PVC {}: {e}", r.pvc_name));
            }
        }
    }

    // 3. Start the agent Job (a Job left by a previous attempt is reused).
    // The raw object, not Zorvia's typed VirtualMachine model: that model
    // covers only part of the spec and would silently drop fields
    // (dataVolumeTemplates, firmware, ...) that a restore needs.
    let vm_ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "kubevirt.io",
        "v1",
        "VirtualMachine",
    ));
    let vms: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &vm_ar);
    let vm_spec = match vms.get(vm_name).await {
        Ok(vm) => {
            let mut v = json!({
                "apiVersion": "kubevirt.io/v1",
                "kind": "VirtualMachine",
                "metadata": {
                    "name": vm.metadata.name,
                    "labels": vm.metadata.labels,
                },
            });
            if let Some(spec) = vm.data.get("spec") {
                v["spec"] = spec.clone();
            }
            v
        }
        Err(e) => {
            cleanup(&client, namespace, &job_name, &restores).await;
            return Outcome::Retry(format!("cannot read VM {vm_name}: {e}"));
        }
    };
    let mut env = target.agent_env();
    env.push(("BACKUP_VM_SPEC".into(), vm_spec.to_string()));
    let spec = BackupJobSpec {
        backup_id: backup_id.clone(),
        vm_name: vm_name.to_string(),
        snapshot_name: snapshot_name.to_string(),
        disks: restores
            .iter()
            .map(|r| BackupDisk {
                name: r.volume_name.clone(),
                pvc: r.pvc_name.clone(),
                block: r.is_block(),
            })
            .collect(),
        env,
        secrets: BackupSecretRef {
            secret_name: target.secret_name.clone(),
            access_key_key: target.access_key_key.clone(),
            secret_key_key: target.secret_key_key.clone(),
            encryption_key_key: target.encryption_key_key.clone(),
        },
        active_deadline_secs: target.active_deadline_secs,
    };
    let job = match build_backup_job(&job_name, namespace, &spec) {
        Ok(j) => j,
        Err(e) => {
            cleanup(&client, namespace, &job_name, &restores).await;
            return Outcome::Failed(e.to_string());
        }
    };
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    match jobs.create(&PostParams::default(), &job).await {
        Ok(_) => {}
        Err(e) if is_conflict(&e) => {}
        Err(e) => {
            cleanup(&client, namespace, &job_name, &restores).await;
            return Outcome::Retry(format!("cannot create backup Job: {e}"));
        }
    }

    // 4. Follow it.
    let followed = follow_job(
        ctx,
        &client,
        namespace,
        &job_name,
        target.active_deadline_secs,
        10,
        89,
    )
    .await;
    cleanup(&client, namespace, &job_name, &restores).await;
    match followed {
        Followed::Done(r) => Outcome::Succeeded(Some(json!({
            "snapshot": snapshot_name,
            "vm_name": vm_name,
            "offcluster": r,
        }))),
        Followed::Failed(e) => Outcome::Failed(e),
        Followed::Cancelled => Outcome::Cancelled,
    }
}

/// How a followed agent Job ended.
pub(crate) enum Followed {
    /// The agent's final result line (its `success` is true).
    Done(Value),
    Failed(String),
    Cancelled,
}

/// Poll an agent Job until it ends: maps the agent's progress lines onto
/// `base..base+span` percent of the operation, stops early if the pod cannot
/// start (missing credential Secret) or the operation is cancelled.
pub(crate) async fn follow_job(
    ctx: &OpContext,
    client: &kube::Client,
    namespace: &str,
    job_name: &str,
    deadline_secs: i64,
    base: u32,
    span: u32,
) -> Followed {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let started = std::time::Instant::now();
    let give_up = Duration::from_secs(deadline_secs.max(60) as u64 + 300);
    loop {
        if ctx.cancelled() {
            return Followed::Cancelled;
        }
        let pod_list = pods
            .list(&ListParams::default().labels(&format!("job-name={job_name}")))
            .await
            .map(|l| l.items)
            .unwrap_or_default();
        if let Some(msg) = pod_list.iter().find_map(pod_config_error) {
            return Followed::Failed(msg);
        }
        let mut logs = String::new();
        if let Some(name) = pod_list.first().and_then(|p| p.metadata.name.clone()) {
            let lp = LogParams {
                tail_lines: Some(200),
                ..Default::default()
            };
            logs = pods.logs(&name, &lp).await.unwrap_or_default();
            if let Some((pct, phase)) = parse_agent_logs(&logs).progress {
                ctx.progress(&phase, (base + pct as u32 * span / 100).min(99) as u8);
            }
        }
        match jobs.get(job_name).await.map(|j| job_state(&j)) {
            Ok(JobState::Succeeded) => {
                return match parse_agent_logs(&logs).result {
                    Some(r) if r.get("success") == Some(&json!(true)) => Followed::Done(r),
                    _ => Followed::Failed("Job finished but reported no successful result".into()),
                };
            }
            Ok(JobState::Failed(why)) => {
                let detail = parse_agent_logs(&logs)
                    .result
                    .and_then(|r| r.get("error").and_then(|e| e.as_str()).map(String::from));
                return Followed::Failed(detail.unwrap_or(why));
            }
            Ok(JobState::Running) | Err(_) => {}
        }
        if started.elapsed() >= give_up {
            return Followed::Failed("timed out waiting for the Job".into());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Where the agent expects each disk (re-exported for docs/tests).
pub fn disk_path(name: &str) -> String {
    format!("{DISKS_ROOT}/{name}/disk.img")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    const BASE: [(&str, &str); 3] = [
        ("ZORVIA_BACKUP_S3_ENDPOINT", "https://s3.example.com"),
        ("ZORVIA_BACKUP_S3_BUCKET", "vm-backups"),
        ("ZORVIA_BACKUP_S3_SECRET", "backup-s3"),
    ];

    #[test]
    fn unconfigured_means_cluster_local_backups() {
        assert_eq!(OffClusterTarget::from_lookup(lookup(&[])).unwrap(), None);
    }

    #[test]
    fn partial_or_invalid_config_is_an_error() {
        assert!(OffClusterTarget::from_lookup(lookup(&[BASE[0]])).is_err());
        assert!(OffClusterTarget::from_lookup(lookup(&[BASE[0], BASE[1]])).is_err()); // no secret
        let mut bad = BASE.to_vec();
        bad[0] = ("ZORVIA_BACKUP_S3_ENDPOINT", "s3.example.com");
        assert!(OffClusterTarget::from_lookup(lookup(&bad)).is_err());
        let mut lock = BASE.to_vec();
        lock.push(("ZORVIA_BACKUP_LOCK_MODE", "compliance"));
        assert!(OffClusterTarget::from_lookup(lookup(&lock)).is_err()); // needs days
        lock.push(("ZORVIA_BACKUP_LOCK_DAYS", "30"));
        assert!(OffClusterTarget::from_lookup(lookup(&lock)).is_ok());
    }

    #[test]
    fn parses_target_and_builds_agent_env() {
        let mut cfg = BASE.to_vec();
        cfg.extend([
            ("ZORVIA_BACKUP_LOCK_MODE", "compliance"),
            ("ZORVIA_BACKUP_LOCK_DAYS", "30"),
            ("ZORVIA_BACKUP_ENCRYPTION_KEY_KEY", "enc-key"),
        ]);
        let t = OffClusterTarget::from_lookup(lookup(&cfg))
            .unwrap()
            .unwrap();
        assert_eq!(t.lock_mode.as_deref(), Some("COMPLIANCE"));
        assert_eq!(t.access_key_key, "access-key");
        assert_eq!(t.active_deadline_secs, 6 * 3600);
        let env = t.agent_env();
        assert!(env.contains(&("BACKUP_LOCK_MODE".into(), "COMPLIANCE".into())));
        assert!(env.contains(&("BACKUP_RETAIN_DAYS".into(), "30".into())));
        // Secrets never appear in the plain env.
        assert!(!env
            .iter()
            .any(|(k, _)| k.contains("SECRET_ACCESS") || k == "BACKUP_ENCRYPTION_KEY"));
    }

    fn content() -> Value {
        json!({"spec": {"volumeBackups": [
            {"volumeName": "rootdisk", "volumeSnapshotName": "vmsnapshot-x-volume-rootdisk",
             "persistentVolumeClaim": {"metadata": {"name": "web-root"}, "spec": {
                "accessModes": ["ReadWriteOnce"], "volumeMode": "Filesystem",
                "resources": {"requests": {"storage": "20Gi"}},
                "storageClassName": "longhorn", "volumeName": "pvc-123",
                "dataSource": {"kind": "Old"}}}},
            {"volumeName": "cloudinit", "persistentVolumeClaim": {"spec": {}}}
        ]}})
    }

    #[test]
    fn plans_restore_only_for_snapshotted_volumes() {
        let plan = plan_restores(&content(), "abc123").unwrap();
        assert_eq!(plan.len(), 1);
        let r = &plan[0];
        assert_eq!(r.pvc_name, "bk-abc123-rootdisk");
        assert_eq!(r.pvc_spec["dataSource"]["kind"], "VolumeSnapshot");
        assert_eq!(
            r.pvc_spec["dataSource"]["name"],
            "vmsnapshot-x-volume-rootdisk"
        );
        assert_eq!(r.pvc_spec["storageClassName"], "longhorn");
        assert!(
            r.pvc_spec.get("volumeName").is_none(),
            "bound PV name must not carry over"
        );
        let m = restore_pvc_manifest(r, "default", "op-1");
        assert_eq!(m["metadata"]["labels"]["zorvia.io/backup-id"], "op-1");
        assert_eq!(m["spec"]["resources"]["requests"]["storage"], "20Gi");
    }

    #[test]
    fn keeps_block_mode_volumes_and_rejects_bad_volume_names() {
        let c = content();
        assert!(!plan_restores(&c, "x").unwrap()[0].is_block());
        let mut c = content();
        c["spec"]["volumeBackups"][0]["persistentVolumeClaim"]["spec"]["volumeMode"] =
            json!("Block");
        let plan = plan_restores(&c, "x").unwrap();
        assert!(plan[0].is_block());
        assert_eq!(
            plan[0].pvc_spec["volumeMode"], "Block",
            "temp PVC must stay Block"
        );
        let mut c = content();
        c["spec"]["volumeBackups"][0]["volumeName"] = json!("Root_Disk");
        assert!(plan_restores(&c, "x").is_err());
        assert!(plan_restores(&json!({}), "x").unwrap().is_empty());
    }

    #[test]
    fn parses_progress_and_result_lines() {
        let logs = "starting\n{\"progress\":10,\"phase\":\"uploading root\"}\n\
                    {\"progress\":55,\"phase\":\"verifying root\"}\n\
                    {\"success\":true,\"backup_id\":\"op-1\"}\n";
        let out = parse_agent_logs(logs);
        assert_eq!(out.progress, Some((55, "verifying root".into())));
        assert_eq!(out.result.unwrap()["backup_id"], "op-1");
        assert_eq!(parse_agent_logs("garbage\n\n"), AgentOutput::default());
    }

    #[test]
    fn classifies_job_state() {
        let job = |v: Value| -> Job { serde_json::from_value(v).unwrap() };
        assert_eq!(
            job_state(&job(json!({"metadata": {}, "status": {"succeeded": 1}}))),
            JobState::Succeeded
        );
        assert_eq!(job_state(&job(json!({"metadata": {}}))), JobState::Running);
        let failed = job(json!({"metadata": {}, "status": {"conditions": [
            {"type": "Failed", "status": "True", "message": "BackoffLimitExceeded"}]}}));
        assert_eq!(
            job_state(&failed),
            JobState::Failed("BackoffLimitExceeded".into())
        );
    }

    #[test]
    fn detects_missing_secret_from_pod_status() {
        let pod: Pod = serde_json::from_value(json!({"metadata": {}, "status": {
            "containerStatuses": [{"name": "backup-agent", "image": "i", "imageID": "",
              "ready": false, "restartCount": 0,
              "state": {"waiting": {"reason": "CreateContainerConfigError",
                "message": "secret \"backup-s3\" not found"}}}]}}))
        .unwrap();
        let msg = pod_config_error(&pod).unwrap();
        assert!(msg.contains("backup-s3") && msg.contains("Secret"));
        let ok: Pod = serde_json::from_value(json!({"metadata": {}})).unwrap();
        assert!(pod_config_error(&ok).is_none());
    }

    fn atlas_bucket() -> Value {
        json!({"id": "bkt_1", "name": "vm-backups", "bucket_name": "vm-backups-63f4",
               "endpoint": "http://rook-ceph-rgw-store.rook-ceph:80", "region": "us-east-1",
               "secret_ref": "vm-backups", "namespace": "rook-ceph", "state": "bound"})
    }

    #[test]
    fn atlas_bucket_becomes_a_backup_target() {
        let overlay = atlas_overlay(&atlas_bucket()).unwrap();
        let get = |k: &str| overlay.get(k).cloned();
        let t = OffClusterTarget::from_lookup(get).unwrap().unwrap();
        assert_eq!(t.endpoint, "http://rook-ceph-rgw-store.rook-ceph:80");
        assert_eq!(t.bucket, "vm-backups-63f4");
        assert_eq!(t.secret_name, "vm-backups");
        assert_eq!(t.region, "us-east-1");
        // Rook OBC credential Secrets use the AWS_* key names.
        assert_eq!(t.access_key_key, "AWS_ACCESS_KEY_ID");
        assert_eq!(t.secret_key_key, "AWS_SECRET_ACCESS_KEY");
    }

    #[test]
    fn atlas_bucket_must_be_bound_and_complete() {
        let mut b = atlas_bucket();
        b["state"] = json!("provisioning");
        assert!(atlas_overlay(&b)
            .unwrap_err()
            .to_string()
            .contains("not bound"));
        for field in ["endpoint", "bucket_name", "secret_ref"] {
            let mut b = atlas_bucket();
            b.as_object_mut().unwrap().remove(field);
            assert!(atlas_overlay(&b).is_err(), "missing {field}");
        }
    }

    #[test]
    fn parses_kubernetes_quantities() {
        assert_eq!(quantity_bytes("11811160064"), Some(11_811_160_064));
        assert_eq!(quantity_bytes("11Gi"), Some(11 * 1024 * 1024 * 1024));
        assert_eq!(quantity_bytes("500M"), Some(500_000_000));
        assert_eq!(quantity_bytes("1.5Gi"), Some(1_610_612_736));
        assert_eq!(quantity_bytes("1Ki"), Some(1024));
        assert_eq!(quantity_bytes("12Xi"), None);
        assert_eq!(quantity_bytes(""), None);
    }

    #[test]
    fn claims_are_raised_to_the_snapshot_restore_size_but_never_shrunk() {
        // The live failure: CDI asked for 11381663335 bytes, Ceph's snapshot is 11Gi.
        let mut spec = json!({"resources": {"requests": {"storage": "11381663335"}}});
        ensure_size_at_least(&mut spec, "11811160064");
        assert_eq!(spec["resources"]["requests"]["storage"], "11811160064");
        // Already large enough: untouched.
        let mut bigger = json!({"resources": {"requests": {"storage": "20Gi"}}});
        ensure_size_at_least(&mut bigger, "11Gi");
        assert_eq!(bigger["resources"]["requests"]["storage"], "20Gi");
        // No request recorded: take the restore size.
        let mut none = json!({});
        ensure_size_at_least(&mut none, "5Gi");
        assert_eq!(none["resources"]["requests"]["storage"], "5Gi");
        // Garbage restore size: leave the spec alone.
        let mut keep = json!({"resources": {"requests": {"storage": "1Gi"}}});
        ensure_size_at_least(&mut keep, "lots");
        assert_eq!(keep["resources"]["requests"]["storage"], "1Gi");
    }

    #[test]
    fn disk_paths_match_the_agent_contract() {
        assert_eq!(disk_path("rootdisk"), "/disks/rootdisk/disk.img");
    }
}
