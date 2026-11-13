//! Phase 5 enterprise capability plans (gated by `ZORVIA_EXPERIMENTAL` + feature flags).
//!
//! These produce actionable plans / dry-runs. They do not call AWS, vSphere, or
//! remote clusters. Live fleet inventory is implemented in `multi_cluster::fleet`.

use serde::{Deserialize, Serialize};

fn require(flag: &str) -> Result<(), String> {
    if crate::features::enterprise_flag(flag) {
        Ok(())
    } else {
        Err(format!(
            "Enterprise feature disabled. Set ZORVIA_EXPERIMENTAL=1 and {flag}=1"
        ))
    }
}

/// S3 immutable backup export plan (no network I/O).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3ImmutableBackupPlan {
    pub vm_name: String,
    pub snapshot_name: String,
    pub bucket: String,
    pub region: String,
    pub object_key: String,
    pub object_lock_mode: String,
    pub retention_days: u32,
    pub notes: Vec<String>,
}

impl S3ImmutableBackupPlan {
    pub fn try_plan(
        vm_name: impl Into<String>,
        snapshot_name: impl Into<String>,
        bucket: impl Into<String>,
        region: impl Into<String>,
        prefix: impl Into<String>,
        retention_days: u32,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_S3_BACKUP")?;
        let vm_name = vm_name.into();
        let snapshot_name = snapshot_name.into();
        let bucket = bucket.into();
        let region = region.into();
        let prefix = prefix.into().trim_matches('/').to_string();
        let object_key = format!(
            "{}/{}/{}.tar.zst",
            if prefix.is_empty() {
                "zorvia-backups".into()
            } else {
                prefix
            },
            vm_name,
            snapshot_name
        );
        Ok(Self {
            vm_name,
            snapshot_name,
            bucket,
            region,
            object_key,
            object_lock_mode: "COMPLIANCE".into(),
            retention_days: retention_days.max(1),
            notes: vec![
                "Plan only — VolumeSnapshot bytes are not uploaded in this release.".into(),
                "Enable S3 Object Lock (compliance) on the bucket before first write.".into(),
                "Wire Velero/Kopia or a custom exporter to execute this plan.".into(),
            ],
        })
    }
}

/// Transiva VMware → KubeVirt conversion plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransivaPlan {
    pub source_vm: String,
    pub vcenter_endpoint: String,
    pub target_namespace: String,
    pub stages: Vec<String>,
    pub notes: Vec<String>,
}

impl TransivaPlan {
    pub fn try_plan(
        source_vm: impl Into<String>,
        vcenter_endpoint: impl Into<String>,
        target_namespace: impl Into<String>,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_TRANSIVA")?;
        Ok(Self {
            source_vm: source_vm.into(),
            vcenter_endpoint: vcenter_endpoint.into(),
            target_namespace: target_namespace.into(),
            stages: vec![
                "discover".into(),
                "convert".into(),
                "import".into(),
                "validate".into(),
            ],
            notes: vec![
                "Plan only — Transiva agent/API integration is not shipped yet.".into(),
                "Ensure network reachability from the conversion worker to vCenter.".into(),
            ],
        })
    }
}

/// Golden-image build → scan → sign → promote pipeline plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenPipelinePlan {
    pub image_name: String,
    pub version: String,
    pub namespace: String,
    pub stages: Vec<String>,
    pub notes: Vec<String>,
}

impl GoldenPipelinePlan {
    pub fn try_plan(
        image_name: impl Into<String>,
        version: impl Into<String>,
        namespace: impl Into<String>,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_GOLDEN_PIPELINE")?;
        let image_name = image_name.into();
        let version = version.into();
        Ok(Self {
            image_name: image_name.clone(),
            version: version.clone(),
            namespace: namespace.into(),
            stages: vec![
                "build".into(),
                "scan".into(),
                "sign".into(),
                "promote".into(),
            ],
            notes: vec![
                format!(
                    "POST /api/v1/enterprise/golden-pipeline/run applies CDI DataVolume+DataSource for {image_name}:{version}."
                ),
                "Scan/sign (cosign) remain operator-owned; promote is the stable DataSource alias.".into(),
            ],
        })
    }
}

/// Result of executing the build+promote stages against the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenPipelineRun {
    pub image_name: String,
    pub version: String,
    pub namespace: String,
    pub versioned_name: String,
    pub data_volume_applied: bool,
    pub data_source_applied: bool,
    pub stages_completed: Vec<String>,
    pub notes: Vec<String>,
}

impl GoldenPipelineRun {
    /// Build golden-image manifests and apply them via the KubeVirt/CDI client.
    pub async fn try_run(
        image_name: impl Into<String>,
        version: impl Into<String>,
        namespace: impl Into<String>,
        source: impl Into<String>,
        size: impl Into<String>,
        client: &crate::kube::KubeClient,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_GOLDEN_PIPELINE")?;
        let image_name = image_name.into();
        let version = version.into();
        let namespace = namespace.into();
        let source = source.into();
        let size = size.into();
        let source_type = if source.starts_with("http://") || source.starts_with("https://") {
            crate::golden_images::ImageSourceType::Http
        } else {
            crate::golden_images::ImageSourceType::Registry
        };
        let source = if source_type == crate::golden_images::ImageSourceType::Registry
            && !source.starts_with("docker://")
        {
            format!("docker://{source}")
        } else {
            source
        };
        let spec = crate::golden_images::GoldenImageSpec {
            name: image_name.clone(),
            version: version.clone(),
            namespace: namespace.clone(),
            source_type,
            source,
            size: if size.trim().is_empty() {
                "20Gi".into()
            } else {
                size
            },
            storage_class: None,
            checksum: None,
        };
        let bundle = crate::golden_images::GoldenImageBundle::build(spec)
            .map_err(|e| format!("bundle build failed: {e}"))?;
        client
            .apply_data_volume(&namespace, &bundle.data_volume)
            .await
            .map_err(|e| format!("DataVolume apply failed: {e}"))?;
        client
            .apply_data_source(&namespace, &bundle.data_source)
            .await
            .map_err(|e| format!("DataSource apply failed: {e}"))?;
        Ok(Self {
            image_name,
            version,
            namespace,
            versioned_name: bundle.versioned_name,
            data_volume_applied: true,
            data_source_applied: true,
            stages_completed: vec!["build".into(), "promote".into()],
            notes: vec![
                "build+promote applied via CDI. scan/sign skipped (no cosign hook in this release)."
                    .into(),
            ],
        })
    }
}

/// Cross-cluster VM mobility / DR plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClusterDrPlan {
    pub source_cluster: String,
    pub target_cluster: String,
    pub vm_names: Vec<String>,
    pub rpo_seconds: u64,
    pub notes: Vec<String>,
}

impl CrossClusterDrPlan {
    pub fn try_plan(
        source_cluster: impl Into<String>,
        target_cluster: impl Into<String>,
        vm_names: Vec<String>,
        rpo_seconds: u64,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_CROSS_CLUSTER_DR")?;
        if vm_names.is_empty() {
            return Err("vm_names must not be empty".into());
        }
        Ok(Self {
            source_cluster: source_cluster.into(),
            target_cluster: target_cluster.into(),
            vm_names,
            rpo_seconds,
            notes: vec![
                "Plan only — POST /api/v1/enterprise/cross-cluster-dr/run queues off-cluster backups (same-cluster DR rehearsal).".into(),
                "Requires an off-cluster S3 target (ZORVIA_BACKUP_S3_* or Atlas bucket). Not site failover.".into(),
            ],
        })
    }
}

/// Same-cluster DR rehearsal: queue off-cluster backups and/or restores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClusterDrRun {
    pub source_cluster: String,
    pub target_cluster: String,
    pub rpo_seconds: u64,
    pub namespace: String,
    pub restore_namespace: String,
    pub backup_operation_ids: Vec<String>,
    pub restore_operation_ids: Vec<String>,
    pub notes: Vec<String>,
}

impl CrossClusterDrRun {
    /// Phase 1: enqueue off-cluster backups for each VM.
    /// Phase 2 (when `backup_operation_ids` is set): enqueue restores into
    /// `restore_namespace` as `{vm}-dr`.
    pub async fn try_run(
        source_cluster: impl Into<String>,
        target_cluster: impl Into<String>,
        vm_names: Vec<String>,
        rpo_seconds: u64,
        namespace: impl Into<String>,
        restore_namespace: impl Into<String>,
        backup_operation_ids: Vec<String>,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_CROSS_CLUSTER_DR")?;
        let source_cluster = source_cluster.into();
        let target_cluster = target_cluster.into();
        let namespace = namespace.into();
        let restore_namespace = restore_namespace.into();
        if !backup_operation_ids.is_empty() {
            let mut restore_operation_ids = Vec::new();
            let mut notes = vec![
                "Restore phase: queuing restores from prior off-cluster backup operations.".into(),
                "This is a same-cluster DR rehearsal via object storage, not cross-site failover."
                    .into(),
            ];
            for op_id in &backup_operation_ids {
                let op = crate::operations::OperationsDb::global()
                    .get(op_id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("backup operation '{op_id}' not found"))?;
                let src = crate::backup::restore::source_from_op(&op).map_err(|e| e.to_string())?;
                let new_name = format!("{}-dr", src.vm_name);
                let restore_op = crate::backup::restore::enqueue_restore_in(
                    crate::operations::OperationsDb::global(),
                    &src,
                    &restore_namespace,
                    &new_name,
                    None,
                    false,
                    None,
                )
                .map_err(|e| e.to_string())?;
                restore_operation_ids.push(restore_op.id);
            }
            notes.push(format!(
                "Poll GET /api/operations/{{id}} for each restore; RPO target metadata: {rpo_seconds}s"
            ));
            return Ok(Self {
                source_cluster,
                target_cluster,
                rpo_seconds,
                namespace,
                restore_namespace,
                backup_operation_ids,
                restore_operation_ids,
                notes,
            });
        }
        if vm_names.is_empty() {
            return Err(
                "vm_names must not be empty (or pass backup_operation_ids for restore phase)".into(),
            );
        }
        match crate::backup::offcluster::OffClusterTarget::resolve().await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(
                    "no off-cluster target is configured (set ZORVIA_BACKUP_S3_* or ZORVIA_BACKUP_ATLAS_BUCKET)"
                        .into(),
                );
            }
            Err(e) => return Err(format!("off-cluster target invalid: {e}")),
        }
        let mut backup_ids = Vec::new();
        for vm in &vm_names {
            let key = format!("dr-rehearsal:{}", uuid::Uuid::new_v4().simple());
            let (op, _) = crate::backup::op::enqueue(
                &namespace,
                vm,
                "full",
                None,
                Some(format!(
                    "cross-cluster-dr rehearsal {source_cluster}->{target_cluster}"
                )),
                &key,
            )
            .map_err(|e| e.to_string())?;
            backup_ids.push(op.id);
        }
        Ok(Self {
            source_cluster,
            target_cluster,
            rpo_seconds,
            namespace,
            restore_namespace: restore_namespace.clone(),
            backup_operation_ids: backup_ids,
            restore_operation_ids: Vec::new(),
            notes: vec![
                "Backup phase queued. When each backup succeeds, POST the same /run with backup_operation_ids to restore into the DR namespace.".into(),
                format!(
                    "Planned restore namespace: {restore_namespace}; new VM names: {{vm}}-dr"
                ),
                "Same-cluster object-store rehearsal — not ACM/site failover.".into(),
                format!("RPO target metadata only: {rpo_seconds}s"),
            ],
        })
    }
}

/// GPU / SR-IOV / NUMA placement suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuSriovNumaPlan {
    pub vm_name: String,
    pub gpu_count: u32,
    pub sriov_networks: Vec<String>,
    pub numa_passthrough: bool,
    pub suggested_patches: serde_json::Value,
    pub notes: Vec<String>,
    /// Live device/placement preflight against cluster inventory (never applied).
    #[serde(default)]
    pub preflight_ok: bool,
    #[serde(default)]
    pub preflight_issues: Vec<String>,
}

impl GpuSriovNumaPlan {
    pub fn try_plan(
        vm_name: impl Into<String>,
        gpu_count: u32,
        sriov_networks: Vec<String>,
        numa_passthrough: bool,
    ) -> Result<Self, String> {
        require("ZORVIA_FEATURE_GPU_NUMA")?;
        let vm_name = vm_name.into();
        let suggested_patches = serde_json::json!({
            "domain": {
                "devices": {
                    "hostDevices": if gpu_count > 0 {
                        serde_json::json!([{ "name": "gpu", "deviceName": "nvidia.com/gpu", "count": gpu_count }])
                    } else {
                        serde_json::json!([])
                    },
                    "interfaces": sriov_networks.iter().map(|n| serde_json::json!({
                        "name": n,
                        "sriov": {},
                    })).collect::<Vec<_>>(),
                },
                "cpu": { "dedicatedCpuPlacement": numa_passthrough, "numa": { "guestMappingPassthrough": {} } },
            }
        });
        Ok(Self {
            vm_name,
            gpu_count,
            sriov_networks,
            numa_passthrough,
            suggested_patches,
            notes: vec![
                "Patches are not applied to the VM CR; preflight_issues reflect live inventory.".into(),
                "Lab tip: advertise a fake extended resource (e.g. lab.zorvia.io/fake-gpu) to exercise permit/preflight without NVIDIA hardware.".into(),
            ],
            preflight_ok: true,
            preflight_issues: Vec::new(),
        })
    }

    /// Attach live preflight issue strings (still no apply).
    pub fn with_preflight_issues(mut self, issues: Vec<String>) -> Self {
        self.preflight_ok = issues.is_empty();
        self.preflight_issues = issues;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_block_by_default() {
        if crate::features::experimental_enabled() {
            return;
        }
        assert!(S3ImmutableBackupPlan::try_plan("vm", "snap", "b", "us-east-1", "", 30).is_err());
        assert!(TransivaPlan::try_plan("vm", "https://vc", "default").is_err());
        assert!(GoldenPipelinePlan::try_plan("img", "1.0", "default").is_err());
        assert!(CrossClusterDrPlan::try_plan("a", "b", vec!["vm".into()], 300).is_err());
        assert!(GpuSriovNumaPlan::try_plan("vm", 1, vec!["net1".into()], true).is_err());
    }
}
