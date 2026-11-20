// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Raw-HTTP client for Rook/Ceph CRDs, following the same
//! `kube::Client::request` pattern `crate::kube::cdi` uses for CDI
//! DataVolumes — no full `kube::CustomResource` typing, since Rook's CRD
//! schemas are large and this crate only needs a curated subset of fields.

#[cfg(feature = "web")]
use anyhow::bail;
use anyhow::Result;
use kube::Client;
#[cfg(feature = "web")]
use serde::Deserialize;
use serde_json::Value;

use crate::kube::lifecycle::validate_k8s_name;
use crate::rook::health::{summarize_ceph_cluster_status, CephHealthSummary};
use crate::rook::manifests::{
    ceph_block_pool_manifest, ceph_cluster_manifest, ceph_filesystem_manifest,
    ceph_object_store_manifest, CephBlockPoolSpec, CephClusterSpec, CephFilesystemSpec,
    CephObjectStoreSpec,
};

const CEPH_GROUP_VERSION: &str = "ceph.rook.io/v1";

#[derive(Clone)]
pub struct RookClient {
    client: Client,
}

impl RookClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn ceph_collection_path(plural: &str, namespace: &str) -> Result<String> {
        validate_k8s_name("namespace", namespace)?;
        Ok(format!(
            "/apis/{CEPH_GROUP_VERSION}/namespaces/{namespace}/{plural}"
        ))
    }

    fn ceph_object_path(plural: &str, namespace: &str, name: &str) -> Result<String> {
        validate_k8s_name("namespace", namespace)?;
        validate_k8s_name("name", name)?;
        Ok(format!(
            "/apis/{CEPH_GROUP_VERSION}/namespaces/{namespace}/{plural}/{name}"
        ))
    }

    async fn http_get(&self, path: &str) -> kube::Result<Value> {
        let req = http::Request::builder()
            .method(http::Method::GET)
            .uri(path)
            .body(Vec::new())
            .expect("valid GET request");
        self.client.request::<Value>(req).await
    }

    async fn http_post(&self, path: &str, body: &Value) -> kube::Result<Value> {
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).expect("manifest serializes"))
            .expect("valid POST request");
        self.client.request::<Value>(req).await
    }

    async fn http_patch_merge(&self, path: &str, body: &Value) -> kube::Result<Value> {
        let req = http::Request::builder()
            .method(http::Method::PATCH)
            .uri(path)
            .header(http::header::CONTENT_TYPE, "application/merge-patch+json")
            .body(serde_json::to_vec(body).expect("manifest serializes"))
            .expect("valid PATCH request");
        self.client.request::<Value>(req).await
    }

    async fn http_delete(&self, path: &str) -> kube::Result<()> {
        let req = http::Request::builder()
            .method(http::Method::DELETE)
            .uri(path)
            .body(Vec::new())
            .expect("valid DELETE request");
        match self.client.request::<Value>(req).await {
            Ok(_) => Ok(()),
            Err(kube::Error::SerdeError(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Create-or-update a namespaced Ceph CRD object: POST to the collection,
    /// falling back to a merge PATCH on the existing object if it already
    /// exists (409 Conflict) — idempotent, safe to call repeatedly.
    async fn apply_ceph_crd(
        &self,
        plural: &str,
        namespace: &str,
        name: &str,
        manifest: &Value,
    ) -> Result<Value> {
        let collection = Self::ceph_collection_path(plural, namespace)?;
        match self.http_post(&collection, manifest).await {
            Ok(v) => Ok(v),
            Err(kube::Error::Api(ae)) if ae.code == 409 => {
                let object = Self::ceph_object_path(plural, namespace, name)?;
                Ok(self.http_patch_merge(&object, manifest).await?)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn get_ceph_crd(&self, plural: &str, namespace: &str, name: &str) -> Result<Value> {
        let path = Self::ceph_object_path(plural, namespace, name)?;
        Ok(self.http_get(&path).await?)
    }

    async fn get_ceph_crd_opt(
        &self,
        plural: &str,
        namespace: &str,
        name: &str,
    ) -> Result<Option<Value>> {
        let path = Self::ceph_object_path(plural, namespace, name)?;
        match self.http_get(&path).await {
            Ok(v) => Ok(Some(v)),
            Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn list_ceph_crd(&self, plural: &str, namespace: &str) -> Result<Vec<Value>> {
        let path = Self::ceph_collection_path(plural, namespace)?;
        let list = self.http_get(&path).await?;
        Ok(list
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default())
    }

    async fn delete_ceph_crd(&self, plural: &str, namespace: &str, name: &str) -> Result<()> {
        let path = Self::ceph_object_path(plural, namespace, name)?;
        Ok(self.http_delete(&path).await?)
    }

    // ── CephCluster ──

    pub async fn create_or_update_ceph_cluster(&self, spec: &CephClusterSpec) -> Result<Value> {
        let manifest = ceph_cluster_manifest(spec)?;
        self.apply_ceph_crd("cephclusters", &spec.namespace, &spec.name, &manifest)
            .await
    }

    pub async fn get_ceph_cluster(&self, namespace: &str, name: &str) -> Result<Value> {
        self.get_ceph_crd("cephclusters", namespace, name).await
    }

    pub async fn delete_ceph_cluster(&self, namespace: &str, name: &str) -> Result<()> {
        self.delete_ceph_crd("cephclusters", namespace, name).await
    }

    pub async fn cluster_health(&self, namespace: &str, name: &str) -> Result<CephHealthSummary> {
        let obj = self
            .get_ceph_crd_opt("cephclusters", namespace, name)
            .await?;
        Ok(summarize_ceph_cluster_status(obj.as_ref()))
    }

    // ── CephBlockPool ──

    pub async fn create_block_pool(&self, spec: &CephBlockPoolSpec) -> Result<Value> {
        let manifest = ceph_block_pool_manifest(spec)?;
        self.apply_ceph_crd("cephblockpools", &spec.namespace, &spec.name, &manifest)
            .await
    }

    pub async fn list_block_pools(&self, namespace: &str) -> Result<Vec<Value>> {
        self.list_ceph_crd("cephblockpools", namespace).await
    }

    pub async fn delete_block_pool(&self, namespace: &str, name: &str) -> Result<()> {
        self.delete_ceph_crd("cephblockpools", namespace, name)
            .await
    }

    // ── CephFilesystem ──

    pub async fn create_filesystem(&self, spec: &CephFilesystemSpec) -> Result<Value> {
        let manifest = ceph_filesystem_manifest(spec)?;
        self.apply_ceph_crd("cephfilesystems", &spec.namespace, &spec.name, &manifest)
            .await
    }

    pub async fn list_filesystems(&self, namespace: &str) -> Result<Vec<Value>> {
        self.list_ceph_crd("cephfilesystems", namespace).await
    }

    pub async fn delete_filesystem(&self, namespace: &str, name: &str) -> Result<()> {
        self.delete_ceph_crd("cephfilesystems", namespace, name)
            .await
    }

    // ── CephObjectStore ──

    pub async fn create_object_store(&self, spec: &CephObjectStoreSpec) -> Result<Value> {
        let manifest = ceph_object_store_manifest(spec)?;
        self.apply_ceph_crd("cephobjectstores", &spec.namespace, &spec.name, &manifest)
            .await
    }

    pub async fn list_object_stores(&self, namespace: &str) -> Result<Vec<Value>> {
        self.list_ceph_crd("cephobjectstores", namespace).await
    }

    pub async fn delete_object_store(&self, namespace: &str, name: &str) -> Result<()> {
        self.delete_ceph_crd("cephobjectstores", namespace, name)
            .await
    }

    // ── Cluster-scoped provisioning objects (StorageClass / VolumeSnapshotClass) ──

    pub async fn apply_storage_class(&self, manifest: &Value) -> Result<Value> {
        let name = manifest["metadata"]["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest missing metadata.name"))?;
        validate_k8s_name("StorageClass name", name)?;
        match self
            .http_post("/apis/storage.k8s.io/v1/storageclasses", manifest)
            .await
        {
            Ok(v) => Ok(v),
            Err(kube::Error::Api(ae)) if ae.code == 409 => Ok(self
                .http_patch_merge(
                    &format!("/apis/storage.k8s.io/v1/storageclasses/{name}"),
                    manifest,
                )
                .await?),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn apply_volume_snapshot_class(&self, manifest: &Value) -> Result<Value> {
        let name = manifest["metadata"]["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest missing metadata.name"))?;
        validate_k8s_name("VolumeSnapshotClass name", name)?;
        let collection = "/apis/snapshot.storage.k8s.io/v1/volumesnapshotclasses";
        match self.http_post(collection, manifest).await {
            Ok(v) => Ok(v),
            Err(kube::Error::Api(ae)) if ae.code == 409 => Ok(self
                .http_patch_merge(&format!("{collection}/{name}"), manifest)
                .await?),
            Err(e) => Err(e.into()),
        }
    }
}

/// Maps a bootstrap manifest object's (apiVersion, kind) to its REST
/// resource path, covering the object kinds present in Rook's own
/// `crds.yaml`/`common.yaml`/`operator.yaml`. Unknown kinds are reported
/// back to the caller rather than silently skipped.
#[cfg(feature = "web")]
fn bootstrap_object_path(
    api_version: &str,
    kind: &str,
    namespace: &str,
    name: &str,
) -> Option<(String, bool)> {
    let (group, version) = match api_version.split_once('/') {
        Some((g, v)) => (Some(g), v),
        None => (None, api_version),
    };
    let (plural, namespaced): (&str, bool) = match kind {
        "Namespace" => ("namespaces", false),
        "ServiceAccount" => ("serviceaccounts", true),
        "ConfigMap" => ("configmaps", true),
        "ClusterRole" => ("clusterroles", false),
        "ClusterRoleBinding" => ("clusterrolebindings", false),
        "Role" => ("roles", true),
        "RoleBinding" => ("rolebindings", true),
        "Deployment" => ("deployments", true),
        "DaemonSet" => ("daemonsets", true),
        "CustomResourceDefinition" => ("customresourcedefinitions", false),
        "PriorityClass" => ("priorityclasses", false),
        "Service" => ("services", true),
        _ => return None,
    };
    let base = match group {
        Some(g) => format!("/apis/{g}/{version}"),
        None => format!("/api/{version}"),
    };
    let path = if namespaced {
        format!("{base}/namespaces/{namespace}/{plural}/{name}")
    } else {
        format!("{base}/{plural}/{name}")
    };
    Some((path, namespaced))
}

#[derive(Debug, Clone)]
pub struct BootstrapOptions {
    /// Rook release tag to bootstrap (e.g. "v1.15.7"). Pinned, not "latest",
    /// so a bootstrap run is reproducible.
    pub version: String,
    /// Namespace the Rook operator and its CephClusters live in.
    pub namespace: String,
    /// Override for the manifest source (defaults to Rook's GitHub raw
    /// content) — set for air-gapped/mirrored environments via
    /// `ROOK_MANIFEST_BASE_URL`.
    pub manifest_base_url: Option<String>,
}

impl Default for BootstrapOptions {
    fn default() -> Self {
        Self {
            version: "v1.15.7".into(),
            namespace: "rook-ceph".into(),
            manifest_base_url: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BootstrapReport {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<(String, String)>,
}

#[cfg(feature = "web")]
const BOOTSTRAP_MANIFESTS: &[&str] = &["crds.yaml", "common.yaml", "operator.yaml"];

#[cfg(feature = "web")]
impl RookClient {
    /// Fetch Rook's pinned-version quickstart manifests from GitHub (or a
    /// configured mirror) and apply every object they contain. Best-effort:
    /// keeps applying after a per-object failure and reports what happened,
    /// since bootstrap is commonly re-run to pick up objects that failed the
    /// first time (e.g. a CRD not yet Established when a CR referencing it
    /// was attempted).
    pub async fn bootstrap_operator(&self, opts: &BootstrapOptions) -> Result<BootstrapReport> {
        validate_k8s_name("namespace", &opts.namespace)?;
        if opts.version.trim().is_empty() {
            bail!("bootstrap version must not be empty");
        }
        let base = opts
            .manifest_base_url
            .clone()
            .unwrap_or_else(|| "https://raw.githubusercontent.com".to_string());
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let mut report = BootstrapReport {
            applied: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        };

        for file in BOOTSTRAP_MANIFESTS {
            let url = format!("{base}/rook/rook/{}/deploy/examples/{file}", opts.version);
            let text = match http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => resp.text().await?,
                Ok(resp) => {
                    report.failed.push((
                        file.to_string(),
                        format!("HTTP {} fetching {url}", resp.status()),
                    ));
                    continue;
                }
                Err(e) => {
                    report
                        .failed
                        .push((file.to_string(), format!("fetch failed: {e}")));
                    continue;
                }
            };
            self.apply_manifest_documents(&text, &opts.namespace, &mut report)
                .await;
        }

        Ok(report)
    }

    async fn apply_manifest_documents(
        &self,
        yaml_text: &str,
        namespace: &str,
        report: &mut BootstrapReport,
    ) {
        // Parse every document into an owned Value up front: serde_yaml's
        // Deserializer wraps non-Send libyaml state, so it can't be held
        // across an `.await` inside the loop below.
        let mut documents = Vec::new();
        for document in serde_yaml::Deserializer::from_str(yaml_text) {
            match Value::deserialize(document) {
                Ok(v) => documents.push(v),
                Err(e) => report
                    .failed
                    .push(("<document>".into(), format!("YAML parse error: {e}"))),
            }
        }

        for value in documents {
            if value.is_null() {
                continue;
            }
            let (Some(api_version), Some(kind)) = (
                value.get("apiVersion").and_then(|v| v.as_str()),
                value.get("kind").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let Some(name) = value
                .get("metadata")
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
            else {
                continue;
            };
            let label = format!("{kind}/{name}");

            let obj_namespace = value
                .get("metadata")
                .and_then(|m| m.get("namespace"))
                .and_then(|n| n.as_str())
                .unwrap_or(namespace);

            let Some((path, namespaced)) =
                bootstrap_object_path(api_version, kind, obj_namespace, name)
            else {
                report.failed.push((
                    label,
                    format!("unrecognized kind '{kind}' — apply manually"),
                ));
                continue;
            };

            // Namespace every namespaced object that doesn't already specify
            // one, so Rook's manifests land in the requested namespace even
            // when authored against the default "rook-ceph".
            let mut manifest = value.clone();
            if namespaced {
                if let Some(meta) = manifest.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                    meta.entry("namespace")
                        .or_insert_with(|| Value::String(obj_namespace.to_string()));
                }
            }

            let collection_path = path.rsplit_once('/').map(|(base, _)| base).unwrap_or(&path);
            match self.http_post(collection_path, &manifest).await {
                Ok(_) => report.applied.push(label),
                Err(kube::Error::Api(ae)) if ae.code == 409 => {
                    match self.http_patch_merge(&path, &manifest).await {
                        Ok(_) => report.applied.push(label),
                        Err(e) => report.failed.push((label, e.to_string())),
                    }
                }
                Err(kube::Error::Api(ae)) if ae.code == 404 => {
                    // Referenced CRD/namespace not Established yet — common on
                    // a first pass; caller is expected to retry bootstrap.
                    report.skipped.push(label);
                }
                Err(e) => report.failed.push((label, e.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "web")]
    fn resolves_known_bootstrap_kinds() {
        let (path, namespaced) =
            bootstrap_object_path("v1", "Namespace", "rook-ceph", "rook-ceph").unwrap();
        assert_eq!(path, "/api/v1/namespaces/rook-ceph");
        assert!(!namespaced);

        let (path, namespaced) =
            bootstrap_object_path("apps/v1", "Deployment", "rook-ceph", "rook-ceph-operator")
                .unwrap();
        assert_eq!(
            path,
            "/apis/apps/v1/namespaces/rook-ceph/deployments/rook-ceph-operator"
        );
        assert!(namespaced);

        let (path, namespaced) = bootstrap_object_path(
            "apiextensions.k8s.io/v1",
            "CustomResourceDefinition",
            "",
            "cephclusters.ceph.rook.io",
        )
        .unwrap();
        assert_eq!(
            path,
            "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/cephclusters.ceph.rook.io"
        );
        assert!(!namespaced);
    }

    #[test]
    #[cfg(feature = "web")]
    fn unknown_kind_returns_none() {
        assert!(bootstrap_object_path("v1", "Frobnicator", "ns", "x").is_none());
    }

    #[test]
    fn ceph_collection_and_object_paths() {
        let collection = RookClient::ceph_collection_path("cephblockpools", "rook-ceph").unwrap();
        assert_eq!(
            collection,
            "/apis/ceph.rook.io/v1/namespaces/rook-ceph/cephblockpools"
        );
        let object =
            RookClient::ceph_object_path("cephblockpools", "rook-ceph", "fast-ssd").unwrap();
        assert_eq!(
            object,
            "/apis/ceph.rook.io/v1/namespaces/rook-ceph/cephblockpools/fast-ssd"
        );
    }

    #[test]
    fn rejects_invalid_namespace_in_path_builders() {
        assert!(RookClient::ceph_collection_path("cephblockpools", "Not_Valid").is_err());
    }
}
