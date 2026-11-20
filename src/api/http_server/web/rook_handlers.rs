// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Rook-Ceph storage API: bootstrap the operator, provision a CephCluster,
//! and manage block pools / filesystems / object stores + the
//! StorageClass/VolumeSnapshotClass objects that tie them to workloads.
//!
//! Rook conventionally lives in its own namespace (default "rook-ceph"),
//! independent of whatever namespace VMs are created in, so every request
//! here takes an explicit `namespace` rather than reusing the server's VM
//! namespace.

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

use crate::rook::{
    BootstrapOptions, CephBlockPoolSpec, CephClusterSpec, CephFilesystemSpec, CephObjectStoreSpec,
    RookClient,
};

fn default_namespace() -> String {
    "rook-ceph".to_string()
}

fn rook_error(action: &str, e: impl std::fmt::Display) -> axum::response::Response {
    let raw = sanitize_error(&e);
    let lower = raw.to_ascii_lowercase();
    let (code, kind) = if lower.contains("notfound") || lower.contains("not found") {
        (404, "NOT_FOUND")
    } else if lower.contains("forbidden") || lower.contains("unauthorized") {
        (403, "FORBIDDEN")
    } else if lower.contains("already exists") || lower.contains("conflict") {
        (409, "CONFLICT")
    } else {
        (500, "ROOK_FAILED")
    };
    let (st, j) = err_json(code, kind, &format!("Failed to {action}: {raw}"));
    (st, j).into_response()
}

#[derive(Debug, Deserialize)]
pub struct BootstrapBody {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub async fn rook_bootstrap(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<BootstrapBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let opts = BootstrapOptions {
        namespace: body.namespace.unwrap_or_else(default_namespace),
        version: body
            .version
            .unwrap_or_else(|| BootstrapOptions::default().version),
        manifest_base_url: std::env::var("ROOK_MANIFEST_BASE_URL").ok(),
    };
    match client.bootstrap_operator(&opts).await {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => rook_error("bootstrap the Rook operator", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct NamespaceQuery {
    #[serde(default)]
    pub namespace: Option<String>,
}

pub async fn rook_cluster_status(
    State(state): State<SharedState>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.cluster_health(&namespace, &namespace).await {
        Ok(summary) => Json(json!(summary)).into_response(),
        Err(e) => rook_error("read CephCluster status", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateClusterBody {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub mon_count: Option<u32>,
    #[serde(default)]
    pub data_dir_host_path: Option<String>,
    #[serde(default)]
    pub use_all_nodes: Option<bool>,
    #[serde(default)]
    pub use_all_devices: Option<bool>,
    #[serde(default)]
    pub device_filter: Option<String>,
}

pub async fn rook_create_cluster(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateClusterBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let defaults = CephClusterSpec::default();
    let spec = CephClusterSpec {
        name: body.namespace.clone(),
        namespace: body.namespace,
        mon_count: body.mon_count.unwrap_or(defaults.mon_count),
        data_dir_host_path: body
            .data_dir_host_path
            .unwrap_or(defaults.data_dir_host_path),
        use_all_nodes: body.use_all_nodes.unwrap_or(defaults.use_all_nodes),
        use_all_devices: body.use_all_devices.unwrap_or(defaults.use_all_devices),
        device_filter: body.device_filter,
    };
    match client.create_or_update_ceph_cluster(&spec).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create CephCluster", e),
    }
}

pub async fn rook_delete_cluster(
    State(state): State<SharedState>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.delete_ceph_cluster(&namespace, &namespace).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rook_error("delete CephCluster", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreatePoolBody {
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub failure_domain: Option<String>,
    #[serde(default)]
    pub replicated_size: Option<u32>,
    #[serde(default)]
    pub erasure_coded: Option<(u32, u32)>,
    #[serde(default)]
    pub device_class: Option<String>,
}

pub async fn rook_list_pools(
    State(state): State<SharedState>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.list_block_pools(&namespace).await {
        Ok(items) => Json(json!(items)).into_response(),
        Err(e) => rook_error("list CephBlockPools", e),
    }
}

pub async fn rook_create_pool(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreatePoolBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let spec = CephBlockPoolSpec {
        name: body.name,
        namespace: body.namespace,
        failure_domain: body
            .failure_domain
            .unwrap_or_else(|| crate::rook::manifests::DEFAULT_FAILURE_DOMAIN.to_string()),
        replicated_size: body.replicated_size.or(if body.erasure_coded.is_none() {
            Some(3)
        } else {
            None
        }),
        erasure_coded: body.erasure_coded,
        device_class: body.device_class,
    };
    match client.create_block_pool(&spec).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create CephBlockPool", e),
    }
}

pub async fn rook_delete_pool(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.delete_block_pool(&namespace, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rook_error("delete CephBlockPool", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateFilesystemBody {
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub failure_domain: Option<String>,
    #[serde(default)]
    pub metadata_pool_replicated_size: Option<u32>,
    #[serde(default)]
    pub data_pool_replicated_size: Option<u32>,
    #[serde(default)]
    pub active_mds_count: Option<u32>,
}

pub async fn rook_list_filesystems(
    State(state): State<SharedState>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.list_filesystems(&namespace).await {
        Ok(items) => Json(json!(items)).into_response(),
        Err(e) => rook_error("list CephFilesystems", e),
    }
}

pub async fn rook_create_filesystem(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateFilesystemBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let spec = CephFilesystemSpec {
        name: body.name,
        namespace: body.namespace,
        failure_domain: body
            .failure_domain
            .unwrap_or_else(|| crate::rook::manifests::DEFAULT_FAILURE_DOMAIN.to_string()),
        metadata_pool_replicated_size: body.metadata_pool_replicated_size.unwrap_or(3),
        data_pool_replicated_size: body.data_pool_replicated_size.unwrap_or(3),
        active_mds_count: body.active_mds_count.unwrap_or(1),
    };
    match client.create_filesystem(&spec).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create CephFilesystem", e),
    }
}

pub async fn rook_delete_filesystem(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.delete_filesystem(&namespace, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rook_error("delete CephFilesystem", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateObjectStoreBody {
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub failure_domain: Option<String>,
    #[serde(default)]
    pub metadata_pool_replicated_size: Option<u32>,
    #[serde(default)]
    pub data_pool_replicated_size: Option<u32>,
    #[serde(default)]
    pub gateway_port: Option<u16>,
    #[serde(default)]
    pub gateway_instances: Option<u32>,
}

pub async fn rook_list_object_stores(
    State(state): State<SharedState>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.list_object_stores(&namespace).await {
        Ok(items) => Json(json!(items)).into_response(),
        Err(e) => rook_error("list CephObjectStores", e),
    }
}

pub async fn rook_create_object_store(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateObjectStoreBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let spec = CephObjectStoreSpec {
        name: body.name,
        namespace: body.namespace,
        failure_domain: body
            .failure_domain
            .unwrap_or_else(|| crate::rook::manifests::DEFAULT_FAILURE_DOMAIN.to_string()),
        metadata_pool_replicated_size: body.metadata_pool_replicated_size.unwrap_or(3),
        data_pool_replicated_size: body.data_pool_replicated_size.unwrap_or(3),
        gateway_port: body.gateway_port.unwrap_or(80),
        gateway_instances: body.gateway_instances.unwrap_or(1),
    };
    match client.create_object_store(&spec).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create CephObjectStore", e),
    }
}

pub async fn rook_delete_object_store(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    Query(q): Query<NamespaceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let namespace = q.namespace.unwrap_or_else(default_namespace);
    match client.delete_object_store(&namespace, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rook_error("delete CephObjectStore", e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CreateStorageClassBody {
    Rbd {
        name: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        pool: String,
        #[serde(default = "default_reclaim_policy")]
        reclaim_policy: String,
    },
    Cephfs {
        name: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        filesystem: String,
        #[serde(default = "default_reclaim_policy")]
        reclaim_policy: String,
    },
}

fn default_reclaim_policy() -> String {
    "Delete".to_string()
}

pub async fn rook_create_storage_class(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateStorageClassBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let manifest = match &body {
        CreateStorageClassBody::Rbd {
            name,
            namespace,
            pool,
            reclaim_policy,
        } => crate::rook::manifests::rbd_storage_class_manifest(
            name,
            namespace,
            pool,
            reclaim_policy,
        ),
        CreateStorageClassBody::Cephfs {
            name,
            namespace,
            filesystem,
            reclaim_policy,
        } => crate::rook::manifests::cephfs_storage_class_manifest(
            name,
            namespace,
            filesystem,
            reclaim_policy,
        ),
    };
    let manifest = match manifest {
        Ok(m) => m,
        Err(e) => {
            let (st, j) = err_json(400, "INVALID", &e.to_string());
            return (st, j).into_response();
        }
    };
    match client.apply_storage_class(&manifest).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create StorageClass", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateVolumeSnapshotClassBody {
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

pub async fn rook_create_volume_snapshot_class(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateVolumeSnapshotClassBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let client = RookClient::new(s.kube_client.client());
    drop(s);
    let manifest = match crate::rook::manifests::rbd_volume_snapshot_class_manifest(
        &body.name,
        &body.namespace,
    ) {
        Ok(m) => m,
        Err(e) => {
            let (st, j) = err_json(400, "INVALID", &e.to_string());
            return (st, j).into_response();
        }
    };
    match client.apply_volume_snapshot_class(&manifest).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => rook_error("create VolumeSnapshotClass", e),
    }
}
