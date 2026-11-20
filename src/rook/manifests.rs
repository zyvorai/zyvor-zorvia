// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Pure Rook/Ceph CRD manifest builders. No cluster access — these just
//! construct the JSON objects `RookClient` applies, following the same
//! raw-manifest pattern as `crate::kube::cdi` for CDI DataVolumes.

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::kube::lifecycle::validate_k8s_name;

/// Failure domain Ceph spreads replicas/shards across. "host" is the safe
/// default for most clusters (survives losing one node).
pub const DEFAULT_FAILURE_DOMAIN: &str = "host";

#[derive(Debug, Clone)]
pub struct CephClusterSpec {
    pub name: String,
    pub namespace: String,
    /// Number of Ceph monitors (odd number, typically 3).
    pub mon_count: u32,
    /// Host path Ceph daemons store config/state under.
    pub data_dir_host_path: String,
    /// Use every node in the cluster for storage (Rook quickstart default).
    pub use_all_nodes: bool,
    /// Use every unformatted device Rook discovers on selected nodes.
    pub use_all_devices: bool,
    /// Optional device name regex filter (e.g. "^sd[b-z]$"), applied only
    /// when `use_all_devices` is false.
    pub device_filter: Option<String>,
}

impl Default for CephClusterSpec {
    fn default() -> Self {
        Self {
            name: "rook-ceph".into(),
            namespace: "rook-ceph".into(),
            mon_count: 3,
            data_dir_host_path: "/var/lib/rook".into(),
            use_all_nodes: true,
            use_all_devices: false,
            device_filter: None,
        }
    }
}

impl CephClusterSpec {
    pub fn validate(&self) -> Result<()> {
        validate_k8s_name("CephCluster name", &self.name)?;
        validate_k8s_name("namespace", &self.namespace)?;
        if self.mon_count == 0 || self.mon_count > 9 {
            bail!("mon_count must be between 1 and 9");
        }
        if self.data_dir_host_path.trim().is_empty() {
            bail!("data_dir_host_path must not be empty");
        }
        Ok(())
    }
}

/// Build a `CephCluster` manifest using Rook's host-storage quickstart shape
/// (useAllNodes/useAllDevices). PVC-based `storageClassDeviceSets` (for
/// dynamically provisioned OSD volumes) are not yet supported — a known
/// follow-up, not something this VM management tool needs by default.
pub fn ceph_cluster_manifest(spec: &CephClusterSpec) -> Result<Value> {
    spec.validate()?;
    let mut storage = json!({
        "useAllNodes": spec.use_all_nodes,
        "useAllDevices": spec.use_all_devices,
    });
    if !spec.use_all_devices {
        if let Some(filter) = &spec.device_filter {
            storage["deviceFilter"] = json!(filter);
        }
    }
    Ok(json!({
        "apiVersion": "ceph.rook.io/v1",
        "kind": "CephCluster",
        "metadata": {
            "name": spec.name,
            "namespace": spec.namespace,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "spec": {
            "cephVersion": { "allowUnsupported": false },
            "dataDirHostPath": spec.data_dir_host_path,
            "mon": { "count": spec.mon_count, "allowMultiplePerNode": false },
            "dashboard": { "enabled": true },
            "storage": storage,
            "healthCheck": {
                "daemonHealth": {
                    "mon": { "disabled": false },
                    "osd": { "disabled": false },
                    "status": { "disabled": false },
                }
            },
        }
    }))
}

#[derive(Debug, Clone)]
pub struct CephBlockPoolSpec {
    pub name: String,
    /// The CephCluster's namespace (Rook CRDs are looked up by the operator
    /// via the namespace they're created in, not a separate cluster ref).
    pub namespace: String,
    pub failure_domain: String,
    /// Replica count for a replicated pool. Mutually exclusive with erasure coding.
    pub replicated_size: Option<u32>,
    /// (data_chunks, coding_chunks) for an erasure-coded pool.
    pub erasure_coded: Option<(u32, u32)>,
    pub device_class: Option<String>,
}

impl CephBlockPoolSpec {
    pub fn validate(&self) -> Result<()> {
        validate_k8s_name("CephBlockPool name", &self.name)?;
        validate_k8s_name("namespace", &self.namespace)?;
        match (self.replicated_size, self.erasure_coded) {
            (Some(_), Some(_)) => bail!("pool can't be both replicated and erasure-coded"),
            (None, None) => bail!("pool needs either replicated_size or erasure_coded"),
            (Some(0), _) => bail!("replicated_size must be at least 1"),
            _ => {}
        }
        Ok(())
    }
}

pub fn ceph_block_pool_manifest(spec: &CephBlockPoolSpec) -> Result<Value> {
    spec.validate()?;
    let mut pool_spec = json!({ "failureDomain": spec.failure_domain });
    if let Some(size) = spec.replicated_size {
        pool_spec["replicated"] = json!({ "size": size });
    } else if let Some((data, coding)) = spec.erasure_coded {
        pool_spec["erasureCoded"] = json!({ "dataChunks": data, "codingChunks": coding });
    }
    if let Some(class) = &spec.device_class {
        pool_spec["deviceClass"] = json!(class);
    }
    Ok(json!({
        "apiVersion": "ceph.rook.io/v1",
        "kind": "CephBlockPool",
        "metadata": {
            "name": spec.name,
            "namespace": spec.namespace,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "spec": pool_spec,
    }))
}

#[derive(Debug, Clone)]
pub struct CephFilesystemSpec {
    pub name: String,
    pub namespace: String,
    pub failure_domain: String,
    pub metadata_pool_replicated_size: u32,
    pub data_pool_replicated_size: u32,
    pub active_mds_count: u32,
}

impl CephFilesystemSpec {
    pub fn validate(&self) -> Result<()> {
        validate_k8s_name("CephFilesystem name", &self.name)?;
        validate_k8s_name("namespace", &self.namespace)?;
        if self.active_mds_count == 0 {
            bail!("active_mds_count must be at least 1");
        }
        Ok(())
    }
}

pub fn ceph_filesystem_manifest(spec: &CephFilesystemSpec) -> Result<Value> {
    spec.validate()?;
    Ok(json!({
        "apiVersion": "ceph.rook.io/v1",
        "kind": "CephFilesystem",
        "metadata": {
            "name": spec.name,
            "namespace": spec.namespace,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "spec": {
            "metadataPool": {
                "failureDomain": spec.failure_domain,
                "replicated": { "size": spec.metadata_pool_replicated_size },
            },
            "dataPools": [{
                "name": "data0",
                "failureDomain": spec.failure_domain,
                "replicated": { "size": spec.data_pool_replicated_size },
            }],
            "metadataServer": {
                "activeCount": spec.active_mds_count,
                "activeStandby": spec.active_mds_count > 1,
            },
        }
    }))
}

#[derive(Debug, Clone)]
pub struct CephObjectStoreSpec {
    pub name: String,
    pub namespace: String,
    pub failure_domain: String,
    pub metadata_pool_replicated_size: u32,
    pub data_pool_replicated_size: u32,
    pub gateway_port: u16,
    pub gateway_instances: u32,
}

impl CephObjectStoreSpec {
    pub fn validate(&self) -> Result<()> {
        validate_k8s_name("CephObjectStore name", &self.name)?;
        validate_k8s_name("namespace", &self.namespace)?;
        if self.gateway_instances == 0 {
            bail!("gateway_instances must be at least 1");
        }
        Ok(())
    }
}

pub fn ceph_object_store_manifest(spec: &CephObjectStoreSpec) -> Result<Value> {
    spec.validate()?;
    Ok(json!({
        "apiVersion": "ceph.rook.io/v1",
        "kind": "CephObjectStore",
        "metadata": {
            "name": spec.name,
            "namespace": spec.namespace,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "spec": {
            "metadataPool": {
                "failureDomain": spec.failure_domain,
                "replicated": { "size": spec.metadata_pool_replicated_size },
            },
            "dataPool": {
                "failureDomain": spec.failure_domain,
                "replicated": { "size": spec.data_pool_replicated_size },
            },
            "preservePoolsOnDelete": false,
            "gateway": {
                "port": spec.gateway_port,
                "instances": spec.gateway_instances,
            }
        }
    }))
}

/// StorageClass wired to Rook's RBD (block) CSI provisioner. Secret names
/// match Rook's documented quickstart defaults for `rook-ceph.rbd.csi.ceph.com`.
pub fn rbd_storage_class_manifest(
    sc_name: &str,
    cluster_namespace: &str,
    pool_name: &str,
    reclaim_policy: &str,
) -> Result<Value> {
    validate_k8s_name("StorageClass name", sc_name)?;
    validate_k8s_name("namespace", cluster_namespace)?;
    validate_k8s_name("pool name", pool_name)?;
    Ok(json!({
        "apiVersion": "storage.k8s.io/v1",
        "kind": "StorageClass",
        "metadata": {
            "name": sc_name,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "provisioner": "rook-ceph.rbd.csi.ceph.com",
        "reclaimPolicy": reclaim_policy,
        "allowVolumeExpansion": true,
        "parameters": {
            "clusterID": cluster_namespace,
            "pool": pool_name,
            "imageFormat": "2",
            "imageFeatures": "layering",
            "csi.storage.k8s.io/provisioner-secret-name": "rook-csi-rbd-provisioner",
            "csi.storage.k8s.io/provisioner-secret-namespace": cluster_namespace,
            "csi.storage.k8s.io/controller-expand-secret-name": "rook-csi-rbd-provisioner",
            "csi.storage.k8s.io/controller-expand-secret-namespace": cluster_namespace,
            "csi.storage.k8s.io/node-stage-secret-name": "rook-csi-rbd-node",
            "csi.storage.k8s.io/node-stage-secret-namespace": cluster_namespace,
            "csi.storage.k8s.io/fstype": "ext4",
        }
    }))
}

/// StorageClass wired to Rook's CephFS CSI provisioner.
pub fn cephfs_storage_class_manifest(
    sc_name: &str,
    cluster_namespace: &str,
    filesystem_name: &str,
    reclaim_policy: &str,
) -> Result<Value> {
    validate_k8s_name("StorageClass name", sc_name)?;
    validate_k8s_name("namespace", cluster_namespace)?;
    validate_k8s_name("filesystem name", filesystem_name)?;
    Ok(json!({
        "apiVersion": "storage.k8s.io/v1",
        "kind": "StorageClass",
        "metadata": {
            "name": sc_name,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "provisioner": "rook-ceph.cephfs.csi.ceph.com",
        "reclaimPolicy": reclaim_policy,
        "allowVolumeExpansion": true,
        "parameters": {
            "clusterID": cluster_namespace,
            "fsName": filesystem_name,
            "pool": format!("{filesystem_name}-data0"),
            "csi.storage.k8s.io/provisioner-secret-name": "rook-csi-cephfs-provisioner",
            "csi.storage.k8s.io/provisioner-secret-namespace": cluster_namespace,
            "csi.storage.k8s.io/controller-expand-secret-name": "rook-csi-cephfs-provisioner",
            "csi.storage.k8s.io/controller-expand-secret-namespace": cluster_namespace,
            "csi.storage.k8s.io/node-stage-secret-name": "rook-csi-cephfs-node",
            "csi.storage.k8s.io/node-stage-secret-namespace": cluster_namespace,
        }
    }))
}

/// VolumeSnapshotClass wired to Rook's RBD CSI provisioner.
pub fn rbd_volume_snapshot_class_manifest(
    vsc_name: &str,
    cluster_namespace: &str,
) -> Result<Value> {
    validate_k8s_name("VolumeSnapshotClass name", vsc_name)?;
    validate_k8s_name("namespace", cluster_namespace)?;
    Ok(json!({
        "apiVersion": "snapshot.storage.k8s.io/v1",
        "kind": "VolumeSnapshotClass",
        "metadata": {
            "name": vsc_name,
            "labels": { "app.kubernetes.io/managed-by": "zorvia" },
        },
        "driver": "rook-ceph.rbd.csi.ceph.com",
        "deletionPolicy": "Delete",
        "parameters": {
            "clusterID": cluster_namespace,
            "csi.storage.k8s.io/snapshotter-secret-name": "rook-csi-rbd-provisioner",
            "csi.storage.k8s.io/snapshotter-secret-namespace": cluster_namespace,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_manifest_uses_quickstart_defaults() {
        let spec = CephClusterSpec::default();
        let m = ceph_cluster_manifest(&spec).unwrap();
        assert_eq!(m["kind"], "CephCluster");
        assert_eq!(m["spec"]["mon"]["count"], 3);
        assert_eq!(m["spec"]["storage"]["useAllNodes"], true);
        assert_eq!(m["spec"]["storage"]["useAllDevices"], false);
    }

    #[test]
    fn cluster_manifest_applies_device_filter() {
        let spec = CephClusterSpec {
            use_all_devices: false,
            device_filter: Some("^sd[b-z]$".into()),
            ..Default::default()
        };
        let m = ceph_cluster_manifest(&spec).unwrap();
        assert_eq!(m["spec"]["storage"]["deviceFilter"], "^sd[b-z]$");
    }

    #[test]
    fn cluster_manifest_rejects_bad_mon_count() {
        let mut spec = CephClusterSpec {
            mon_count: 0,
            ..Default::default()
        };
        assert!(ceph_cluster_manifest(&spec).is_err());
        spec.mon_count = 20;
        assert!(ceph_cluster_manifest(&spec).is_err());
    }

    #[test]
    fn block_pool_replicated() {
        let spec = CephBlockPoolSpec {
            name: "fast-ssd".into(),
            namespace: "rook-ceph".into(),
            failure_domain: DEFAULT_FAILURE_DOMAIN.into(),
            replicated_size: Some(3),
            erasure_coded: None,
            device_class: Some("ssd".into()),
        };
        let m = ceph_block_pool_manifest(&spec).unwrap();
        assert_eq!(m["spec"]["replicated"]["size"], 3);
        assert_eq!(m["spec"]["deviceClass"], "ssd");
        assert!(m["spec"].get("erasureCoded").is_none());
    }

    #[test]
    fn block_pool_erasure_coded() {
        let spec = CephBlockPoolSpec {
            name: "archive".into(),
            namespace: "rook-ceph".into(),
            failure_domain: DEFAULT_FAILURE_DOMAIN.into(),
            replicated_size: None,
            erasure_coded: Some((4, 2)),
            device_class: None,
        };
        let m = ceph_block_pool_manifest(&spec).unwrap();
        assert_eq!(m["spec"]["erasureCoded"]["dataChunks"], 4);
        assert_eq!(m["spec"]["erasureCoded"]["codingChunks"], 2);
    }

    #[test]
    fn block_pool_rejects_both_or_neither_mode() {
        let mut spec = CephBlockPoolSpec {
            name: "x".into(),
            namespace: "rook-ceph".into(),
            failure_domain: DEFAULT_FAILURE_DOMAIN.into(),
            replicated_size: Some(3),
            erasure_coded: Some((4, 2)),
            device_class: None,
        };
        assert!(ceph_block_pool_manifest(&spec).is_err());
        spec.replicated_size = None;
        spec.erasure_coded = None;
        assert!(ceph_block_pool_manifest(&spec).is_err());
    }

    #[test]
    fn filesystem_manifest_shape() {
        let spec = CephFilesystemSpec {
            name: "cephfs".into(),
            namespace: "rook-ceph".into(),
            failure_domain: DEFAULT_FAILURE_DOMAIN.into(),
            metadata_pool_replicated_size: 3,
            data_pool_replicated_size: 3,
            active_mds_count: 1,
        };
        let m = ceph_filesystem_manifest(&spec).unwrap();
        assert_eq!(m["kind"], "CephFilesystem");
        assert_eq!(m["spec"]["dataPools"][0]["replicated"]["size"], 3);
        assert_eq!(m["spec"]["metadataServer"]["activeStandby"], false);
    }

    #[test]
    fn object_store_manifest_shape() {
        let spec = CephObjectStoreSpec {
            name: "s3-store".into(),
            namespace: "rook-ceph".into(),
            failure_domain: DEFAULT_FAILURE_DOMAIN.into(),
            metadata_pool_replicated_size: 3,
            data_pool_replicated_size: 3,
            gateway_port: 80,
            gateway_instances: 2,
        };
        let m = ceph_object_store_manifest(&spec).unwrap();
        assert_eq!(m["kind"], "CephObjectStore");
        assert_eq!(m["spec"]["gateway"]["instances"], 2);
    }

    #[test]
    fn rbd_storage_class_references_correct_provisioner() {
        let m = rbd_storage_class_manifest("rook-ceph-block", "rook-ceph", "fast-ssd", "Delete")
            .unwrap();
        assert_eq!(m["provisioner"], "rook-ceph.rbd.csi.ceph.com");
        assert_eq!(m["parameters"]["pool"], "fast-ssd");
        assert_eq!(m["parameters"]["clusterID"], "rook-ceph");
    }

    #[test]
    fn cephfs_storage_class_references_correct_provisioner() {
        let m =
            cephfs_storage_class_manifest("rook-cephfs", "rook-ceph", "cephfs", "Delete").unwrap();
        assert_eq!(m["provisioner"], "rook-ceph.cephfs.csi.ceph.com");
        assert_eq!(m["parameters"]["fsName"], "cephfs");
    }

    #[test]
    fn volume_snapshot_class_shape() {
        let m = rbd_volume_snapshot_class_manifest("rook-ceph-snap", "rook-ceph").unwrap();
        assert_eq!(m["driver"], "rook-ceph.rbd.csi.ceph.com");
        assert_eq!(m["deletionPolicy"], "Delete");
    }
}
