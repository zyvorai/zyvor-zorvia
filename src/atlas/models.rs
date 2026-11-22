// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! DTOs mirroring `atlas-api-types` (`../atlas/crates/atlas-api-types/src/lib.rs`).
//! Atlas's gateway returns typed JSON for backends/storage-classes but raw
//! `serde_json::Value` for clusters/pools/volumes/ceph-status/ceph-df/create-volume
//! (see `atlas-gateway/src/routes/{inventory,volumes}.rs`) — this module matches
//! that contract exactly rather than guessing an untyped route into a typed one.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    Ceph,
    Nfs,
    Zfs,
    San,
    CloudBlock,
    Kubernetes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendMode {
    ManagedRook,
    External,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Warn,
    Critical,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    #[default]
    Block,
    Filesystem,
    Object,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub block: bool,
    pub file: bool,
    pub object: bool,
    pub snapshots: bool,
    pub clone: bool,
    pub expansion: bool,
    pub replication: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageBackend {
    pub id: String,
    pub name: String,
    pub backend_type: BackendType,
    pub mode: BackendMode,
    pub status: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    pub connection_ref: Option<String>,
    #[serde(default)]
    pub cordoned: bool,
}

/// `POST /backends` request. `backend_type`/`mode` are free strings on the
/// wire (Atlas parses them permissively -- e.g. any unrecognized `mode`
/// falls back to `External`); `server`/`targets` only matter for `nfs`/`zfs`
/// backend types, which Atlas instantiates live (others land as a `pending`
/// catalog row with no live driver).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateBackendRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
}

/// `POST /rbd-images` request. A raw RBD image is a *separate identity
/// space* (`rbd:<pool>/<image>`) from the `StorageVolume` abstraction the
/// volume routes use -- not the same resource, don't conflate them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRbdImageRequest {
    pub name: String,
    pub size_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// `POST /rbd-images/{pool}/{image}/clone` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneRbdImageRequest {
    pub name: String,
    /// Snapshot name to create + protect on the parent; Atlas defaults this
    /// to `<clone>-base` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// `POST /buckets` request -- provisions an RGW bucket via an ObjectBucketClaim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBucketRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_objects: Option<i64>,
    /// e.g. `"2G"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<String>,
}

/// `POST /backup-jobs` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBackupRequest {
    pub volume_id: String,
    pub bucket_id: String,
    /// `"manifest"` (default) or `"data"` (also exports the RBD image data
    /// to S3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_secs: Option<i64>,
}

/// `POST /restore-jobs` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRestoreRequest {
    pub backup_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    /// `"snapshot"` (default) or `"data"` (reconstruct from the RBD diff in S3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `POST /ai/advisor` request. `mode` is one of `auto` (default -- use an
/// external provider if `ATLAS_AI_BASE_URL`/`ATLAS_AI_MODEL` are configured
/// on Atlas, else the deterministic local advisor), `local` (never send
/// operational context to an external model), or `llm` (require a
/// configured provider, error if unavailable).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiAdvisorRequest {
    #[serde(default)]
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `POST /ai/what-if` request -- projects capacity/risk under a hypothetical
/// (e.g. "what if we add 2TiB and resolve every open alert").
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiWhatIfRequest {
    #[serde(default)]
    pub add_capacity_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon_days: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_growth_bytes_per_day: Option<f64>,
    #[serde(default)]
    pub assume_alerts_resolved: bool,
    #[serde(default)]
    pub assume_recovery_complete: bool,
}

/// `POST /dr/peers` request. `secret_ref` should name a k8s Secret holding
/// the peer bootstrap token -- never the token itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterDrPeerRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_fsid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
}

/// `POST /dr/failover` request -- `confirm: true` is required, this is
/// destructive. `force` overrides a not-`ready` preflight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrFailoverRequest {
    pub mirror_id: String,
    pub confirm: bool,
    #[serde(default)]
    pub force: bool,
}

/// `PUT /tenants/:id/policies/:intent` request -- overrides an intent's
/// storage-class placement for one tenant (admin).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantPolicyRequest {
    pub storage_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_mode: Option<String>,
}

/// `PUT /tenants/:id/quota` request. `0` means unlimited for either field.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TenantQuotaRequest {
    pub max_bytes: i64,
    pub max_volumes: i64,
}

/// `POST /volumes/:id/schedule` request. `kind` is `snapshot` (default) or
/// `backup` (which additionally requires `bucket_id`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateScheduleRequest {
    pub interval_secs: i64,
    #[serde(default)]
    pub keep: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `POST /databridge/sources` request -- registers a cloud/source database
/// (synchronous, not a job). `kind` must be one of `postgres`/`mysql`/
/// `mariadb`/`oracle`/`sqlserver`/`mongodb`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateDataBridgeSourceRequest {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// k8s Secret with the source credentials (used only in `real` driver mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_mode: Option<String>,
    /// `fake` (default) or `real`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_mode: Option<String>,
}

/// `POST /databridge/plans` request (synchronous, not a job).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateMigrationPlanRequest {
    pub name: String,
    pub source_id: String,
    /// Rollback window after cutover, in seconds (Atlas default: 72h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_window_secs: Option<i64>,
}

/// `POST /databridge/object` request -- registers an S3-protocol
/// object-store migration (synchronous; no copy happens until `start`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateObjectMigrationRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provider: Option<String>,
    pub source_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_region: Option<String>,
    pub source_bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_provider: Option<String>,
    pub dest_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_region: Option<String>,
    pub dest_bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_namespace: Option<String>,
    /// `full` or `incremental` (default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_size_mb: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageClassInfo {
    pub name: String,
    pub provisioner: String,
    pub reclaim_policy: Option<String>,
    pub volume_binding_mode: Option<String>,
    pub allow_volume_expansion: Option<bool>,
    #[serde(default)]
    pub is_ceph: bool,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// Sibling-product ownership tag for a volume (PDF §5.3, §11 `product_bindings`
/// in Atlas). Populate this when creating a volume in the context of a VM so
/// it's traceable back to Zorvia in Atlas's own inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owner {
    pub product: String,
    pub resource_type: String,
    pub resource_id: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "data_disk".into()
}

impl Owner {
    /// The shape Zorvia always uses: a volume owned by one of its own VMs.
    pub fn for_vm(vm_name: &str) -> Self {
        Self {
            product: "zorvia".into(),
            resource_type: "vm".into(),
            resource_id: vm_name.to_string(),
            role: default_role(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct K8sVolumeOpts {
    pub namespace: Option<String>,
    #[serde(default = "default_true")]
    pub create_pvc: bool,
    #[serde(default)]
    pub access_modes: Vec<String>,
    pub volume_mode: Option<String>,
    pub storage_class: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVolumeRequest {
    pub tenant_id: String,
    pub name: String,
    pub size_bytes: i64,
    #[serde(default)]
    pub kind: VolumeKind,
    pub policy: Option<String>,
    pub pool: Option<String>,
    #[serde(default)]
    pub owner: Option<Owner>,
    #[serde(default)]
    pub kubernetes: Option<K8sVolumeOpts>,
}

/// A job record as surfaced by the API (`GET /jobs`, `GET /jobs/:id`) --
/// every Atlas write (volume create/expand/delete included) returns one of
/// these ids and this is how Zorvia can actually find out what happened to
/// it, instead of only ever seeing "queued".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub tenant_id: String,
    pub job_type: String,
    pub state: String,
    pub requested_by: String,
    pub progress_percent: i64,
    pub error: Option<String>,
    #[serde(default)]
    pub result: serde_json::Value,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl JobRecord {
    pub fn is_terminal(&self) -> bool {
        self.state == "succeeded" || self.state == "failed"
    }
}
