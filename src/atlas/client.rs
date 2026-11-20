// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use super::models::*;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub token: Option<String>,
    pub default_tenant_id: Option<String>,
    pub timeout: Duration,
    pub allow_invalid_tls: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let base_url = match std::env::var("ATLAS_URL") {
            Ok(value) if !value.trim().is_empty() => value.trim().trim_end_matches('/').to_string(),
            _ => return Ok(None),
        };
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|e| anyhow::anyhow!("invalid ATLAS_URL: {e}"))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            anyhow::bail!("ATLAS_URL must use http:// or https://");
        }
        let timeout_secs = std::env::var("ATLAS_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0 && *v <= 300)
            .unwrap_or(30);
        let allow_invalid_tls = std::env::var("ATLAS_TLS_INSECURE")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        Ok(Some(Self {
            base_url,
            token: std::env::var("ATLAS_TOKEN").ok().filter(|v| !v.is_empty()),
            default_tenant_id: std::env::var("ATLAS_TENANT_ID")
                .ok()
                .filter(|v| !v.is_empty()),
            timeout: Duration::from_secs(timeout_secs),
            allow_invalid_tls,
        }))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Atlas request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Atlas returned HTTP {status}: {message}")]
    Upstream {
        status: u16,
        code: Option<String>,
        message: String,
    },
    #[error("Atlas tenant is required; set ATLAS_TENANT_ID or pass a tenant_id")]
    MissingTenant,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamEnvelope {
    error: Option<UpstreamError>,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamError {
    code: Option<String>,
    message: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    config: Config,
    http: reqwest::Client,
}

impl Client {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        Config::from_env()?.map(Self::new).transpose()
    }

    pub fn new(config: Config) -> anyhow::Result<Self> {
        if config.allow_invalid_tls {
            log::warn!(
                "ATLAS_TLS_INSECURE is enabled; Atlas TLS certificates will not be verified"
            );
        }
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .danger_accept_invalid_certs(config.allow_invalid_tls)
            .user_agent(format!(
                "zorvia/{}/atlas-adapter",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self { config, http })
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    pub fn configured_tenant_id(&self) -> Option<&str> {
        self.config.default_tenant_id.as_deref()
    }

    fn tenant_id<'a>(&'a self, tenant_id: Option<&'a str>) -> Result<&'a str, Error> {
        tenant_id
            .filter(|v| !v.trim().is_empty())
            .or(self.config.default_tenant_id.as_deref())
            .ok_or(Error::MissingTenant)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.config.base_url, path.trim_start_matches('/'))
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.request(method, self.endpoint(path));
        if let Some(token) = &self.config.token {
            req = req.bearer_auth(token);
        }
        req
    }

    async fn decode<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, Error> {
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response.json::<T>().await?);
        }
        Err(Self::decode_error(
            status,
            response.text().await.unwrap_or_default(),
        ))
    }

    fn decode_error(status: StatusCode, body: String) -> Error {
        let parsed = serde_json::from_str::<UpstreamEnvelope>(&body)
            .ok()
            .and_then(|v| v.error);
        let message = parsed
            .as_ref()
            .and_then(|e| e.message.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                let trimmed = body.trim();
                if trimmed.is_empty() {
                    status
                        .canonical_reason()
                        .unwrap_or("upstream error")
                        .to_string()
                } else {
                    trimmed.chars().take(512).collect()
                }
            });
        Error::Upstream {
            status: status.as_u16(),
            code: parsed.and_then(|e| e.code),
            message,
        }
    }

    // ── Unauthenticated (root, not under /api/atlas/v1) ──

    pub async fn health(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/health")).await
    }

    pub async fn version(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/version")).await
    }

    // ── Bearer-authenticated, /api/atlas/v1/* ──

    pub async fn list_backends(&self) -> Result<Vec<StorageBackend>, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/backends"))
            .await
    }

    pub async fn backends_summary(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/backends/summary"))
            .await
    }

    /// Registers a backend row -- synchronous, not an async job (unlike the
    /// volume writes). Only `nfs`/`zfs` `backend_type`s get instantiated
    /// live; others land as a `pending` catalog row.
    pub async fn create_backend(
        &self,
        request: CreateBackendRequest,
    ) -> Result<StorageBackend, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/backends")
                .json(&request),
        )
        .await
    }

    /// Refused (`400`) while volumes still reference the backend unless
    /// `purge` is set, which first drops the backend's orphaned inventory
    /// rows (no real storage is touched by purge itself).
    pub async fn delete_backend(&self, id: &str, purge: bool) -> Result<serde_json::Value, Error> {
        let path = format!("/api/atlas/v1/backends/{}", urlencoding::encode(id));
        let mut req = self.request(Method::DELETE, &path);
        if purge {
            req = req.query(&[("purge", "true")]);
        }
        self.decode(req).await
    }

    pub async fn discover_backend(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/backends/{}/discover",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn cordon_backend(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!("/api/atlas/v1/backends/{}/cordon", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn uncordon_backend(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/backends/{}/uncordon",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Whether Atlas's job engine is paused. Paused jobs stay `queued`
    /// until resumed.
    pub async fn get_maintenance(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/maintenance"))
            .await
    }

    pub async fn set_maintenance(&self, paused: bool) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/maintenance")
                .json(&serde_json::json!({ "paused": paused })),
        )
        .await
    }

    pub async fn list_orphans(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/maintenance/orphans"))
            .await
    }

    /// Report-only (always `200`); `ready`/`blockers` in the response is the
    /// actual gate -- aggregates cluster health, open critical alerts,
    /// in-flight jobs, and CDC lag.
    pub async fn upgrade_preflight(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/upgrade/preflight"))
            .await
    }

    pub async fn list_osds(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/osds"))
            .await
    }

    /// Drain an OSD (`ceph osd out`) -- async job, admin-role.
    pub async fn osd_out(&self, osd_id: i64) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::POST, &format!("/api/atlas/v1/osds/{osd_id}/out")))
            .await
    }

    /// Return a drained OSD to service (`ceph osd in`) -- async job, admin-role.
    pub async fn osd_in(&self, osd_id: i64) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::POST, &format!("/api/atlas/v1/osds/{osd_id}/in")))
            .await
    }

    /// Reweight an OSD (`ceph osd reweight`, `weight` in `[0.0, 1.0]`) --
    /// async job, admin-role.
    pub async fn osd_reweight(&self, osd_id: i64, weight: f64) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("/api/atlas/v1/osds/{osd_id}/reweight"),
            )
            .query(&[("weight", weight.to_string())]),
        )
        .await
    }

    pub async fn list_clusters(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/clusters"))
            .await
    }

    pub async fn cluster_health(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/clusters/{}/health", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn list_pools(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/pools"))
            .await
    }

    pub async fn ceph_status(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/ceph/status"))
            .await
    }

    pub async fn ceph_df(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/ceph/df"))
            .await
    }

    pub async fn list_storage_classes(&self) -> Result<Vec<StorageClassInfo>, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/storage-classes"))
            .await
    }

    pub async fn list_volumes(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/volumes"))
            .await
    }

    pub async fn create_volume(
        &self,
        mut request: CreateVolumeRequest,
    ) -> Result<serde_json::Value, Error> {
        if request.tenant_id.trim().is_empty() {
            request.tenant_id = self.tenant_id(None)?.to_string();
        }
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/volumes")
                .json(&request),
        )
        .await
    }

    /// Grows a volume. Atlas rejects `new_size_bytes <= current size` itself
    /// (`AppError::Validation`), so this doesn't duplicate that check.
    pub async fn expand_volume(
        &self,
        id: &str,
        new_size_bytes: i64,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("/api/atlas/v1/volumes/{}/expand", urlencoding::encode(id)),
            )
            .json(&serde_json::json!({ "new_size_bytes": new_size_bytes })),
        )
        .await
    }

    /// Deletes a volume. `confirm` must be `true` for production/protected-class
    /// volumes (Atlas's own `volume_requires_delete_confirm` check) -- Atlas
    /// returns a `400 Validation` error naming that requirement if it's needed
    /// and missing, rather than Zorvia silently guessing when to set it.
    pub async fn delete_volume(&self, id: &str, confirm: bool) -> Result<serde_json::Value, Error> {
        let path = format!("/api/atlas/v1/volumes/{}", urlencoding::encode(id));
        let mut req = self.request(Method::DELETE, &path);
        if confirm {
            req = req.query(&[("confirm", "true")]);
        }
        self.decode(req).await
    }

    /// Every Atlas write (including volume create/expand/delete) returns a
    /// job id -- this is how to find out what actually happened to it.
    /// Atlas caps this list at 100 most-recent jobs server-side; there's no
    /// pagination to request more.
    pub async fn list_jobs(&self) -> Result<Vec<JobRecord>, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/jobs"))
            .await
    }

    pub async fn get_job(&self, id: &str) -> Result<JobRecord, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/jobs/{}", urlencoding::encode(id)),
        ))
        .await
    }

    /// Atlas requires admin-role on the token for this; returns `409` if the
    /// job is already terminal (succeeded/failed) -- surfaced to the caller
    /// as a normal `Error::Upstream`, not treated as a client-level error.
    pub async fn cancel_job(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!("/api/atlas/v1/jobs/{}/cancel", urlencoding::encode(id)),
        ))
        .await
    }

    // ── RBD images -- a separate identity space (rbd:<pool>/<image>) from
    // the StorageVolume abstraction the volume routes above use. All writes
    // here are async jobs (Atlas's `accepted()` envelope), same shape as
    // the volume writes, except `refresh_rbd_usage` which is synchronous. ──

    fn rbd_path(pool: &str, image: &str) -> String {
        format!(
            "/api/atlas/v1/rbd-images/{}/{}",
            urlencoding::encode(pool),
            urlencoding::encode(image)
        )
    }

    pub async fn list_rbd_images(&self, pool: Option<&str>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/rbd-images");
        if let Some(pool) = pool {
            req = req.query(&[("pool", pool)]);
        }
        self.decode(req).await
    }

    pub async fn create_rbd_image(
        &self,
        request: CreateRbdImageRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/rbd-images")
                .json(&request),
        )
        .await
    }

    pub async fn delete_rbd_image(
        &self,
        pool: &str,
        image: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::DELETE, &Self::rbd_path(pool, image)))
            .await
    }

    /// Snapshot+protect the parent and create a COW clone.
    pub async fn clone_rbd_image(
        &self,
        pool: &str,
        image: &str,
        request: CloneRbdImageRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/clone", Self::rbd_path(pool, image)),
            )
            .json(&request),
        )
        .await
    }

    /// Grows by default; `allow_shrink: true` permits a shrink (Atlas warns
    /// this can lose data past the new size).
    pub async fn resize_rbd_image(
        &self,
        pool: &str,
        image: &str,
        size_bytes: i64,
        allow_shrink: bool,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/resize", Self::rbd_path(pool, image)),
            )
            .json(&serde_json::json!({ "size_bytes": size_bytes, "allow_shrink": allow_shrink })),
        )
        .await
    }

    /// Live-migrates to another pool (`rbd migration prepare -> execute -> commit`).
    pub async fn migrate_rbd_image(
        &self,
        pool: &str,
        image: &str,
        dest_pool: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/migrate", Self::rbd_path(pool, image)),
            )
            .query(&[("dest_pool", dest_pool)]),
        )
        .await
    }

    pub async fn flatten_rbd_image(
        &self,
        pool: &str,
        image: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!("{}/flatten", Self::rbd_path(pool, image)),
        ))
        .await
    }

    /// Caps IOPS/bandwidth; `0` clears a cap. At least one of `iops`/`bps`
    /// is required -- Atlas rejects the request with `400` otherwise.
    pub async fn qos_rbd_image(
        &self,
        pool: &str,
        image: &str,
        iops: Option<i64>,
        bps: Option<i64>,
    ) -> Result<serde_json::Value, Error> {
        let mut query = Vec::new();
        if let Some(iops) = iops {
            query.push(("iops".to_string(), iops.to_string()));
        }
        if let Some(bps) = bps {
            query.push(("bps".to_string(), bps.to_string()));
        }
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/qos", Self::rbd_path(pool, image)),
            )
            .query(&query),
        )
        .await
    }

    pub async fn list_rbd_snapshots(
        &self,
        pool: &str,
        image: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("{}/snapshots", Self::rbd_path(pool, image)),
        ))
        .await
    }

    pub async fn create_rbd_snapshot(
        &self,
        pool: &str,
        image: &str,
        name: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/snapshots", Self::rbd_path(pool, image)),
            )
            .json(&serde_json::json!({ "name": name })),
        )
        .await
    }

    /// Rolls the image back to the named snapshot (destructive; admin-role
    /// on Atlas's side).
    pub async fn rollback_rbd_image(
        &self,
        pool: &str,
        image: &str,
        snapshot_name: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/rollback", Self::rbd_path(pool, image)),
            )
            .json(&serde_json::json!({ "name": snapshot_name })),
        )
        .await
    }

    pub async fn delete_rbd_snapshot(
        &self,
        pool: &str,
        image: &str,
        snapshot_name: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!(
                "{}/snapshots/{}",
                Self::rbd_path(pool, image),
                urlencoding::encode(snapshot_name)
            ),
        ))
        .await
    }

    /// Recomputes `used_bytes` for every RBD-backed volume (`rbd du`) --
    /// synchronous, not an async job, unlike every other write in this
    /// section.
    pub async fn refresh_rbd_usage(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::POST, "/api/atlas/v1/rbd-usage/refresh"))
            .await
    }

    // ── Object-store buckets + backups/restores ──

    pub async fn list_buckets(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/buckets"))
            .await
    }

    pub async fn get_bucket(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/buckets/{}", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn create_bucket(
        &self,
        request: CreateBucketRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/buckets")
                .json(&request),
        )
        .await
    }

    /// Refused (`409`) while the bucket still holds backups, unless `force`.
    pub async fn delete_bucket(&self, id: &str, force: bool) -> Result<serde_json::Value, Error> {
        let path = format!("/api/atlas/v1/buckets/{}", urlencoding::encode(id));
        let mut req = self.request(Method::DELETE, &path);
        if force {
            req = req.query(&[("force", "true")]);
        }
        self.decode(req).await
    }

    /// Soft-fails (a normal `200`, not an error) on driver timeout so a
    /// console can show "unavailable" instead of hanging -- fake mode
    /// reports a zeroed-but-available stub (no `radosgw-admin` binary to
    /// shell out to, and unlike RBD, buckets don't cache stats in
    /// inventory).
    pub async fn bucket_stats(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/buckets/{}/stats", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn list_bucket_objects(
        &self,
        id: &str,
        prefix: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let path = format!("/api/atlas/v1/buckets/{}/objects", urlencoding::encode(id));
        let mut req = self.request(Method::GET, &path);
        if let Some(prefix) = prefix {
            req = req.query(&[("prefix", prefix)]);
        }
        self.decode(req).await
    }

    pub async fn delete_bucket_object(
        &self,
        id: &str,
        key: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::DELETE,
                &format!("/api/atlas/v1/buckets/{}/objects", urlencoding::encode(id)),
            )
            .query(&[("key", key)]),
        )
        .await
    }

    /// Mints a presigned S3 PUT URL so the browser uploads straight to RGW
    /// -- Zorvia never sees the object bytes. `versioned: true` stores each
    /// upload at `<key>.<UTC timestamp>` instead of overwriting, so
    /// `prune_bucket_objects` can enforce a keep-N retention afterward.
    pub async fn bucket_object_upload_url(
        &self,
        id: &str,
        key: &str,
        ttl_secs: Option<u64>,
        versioned: bool,
    ) -> Result<serde_json::Value, Error> {
        let mut body = serde_json::json!({ "key": key, "versioned": versioned });
        if let Some(ttl) = ttl_secs {
            body["ttl_secs"] = serde_json::json!(ttl);
        }
        self.decode(
            self.request(
                Method::POST,
                &format!(
                    "/api/atlas/v1/buckets/{}/objects/upload-url",
                    urlencoding::encode(id)
                ),
            )
            .json(&body),
        )
        .await
    }

    pub async fn bucket_object_download_url(
        &self,
        id: &str,
        key: &str,
        ttl_secs: Option<u64>,
    ) -> Result<serde_json::Value, Error> {
        let path = format!(
            "/api/atlas/v1/buckets/{}/objects/download-url",
            urlencoding::encode(id)
        );
        let mut query = vec![("key".to_string(), key.to_string())];
        if let Some(ttl) = ttl_secs {
            query.push(("ttl_secs".to_string(), ttl.to_string()));
        }
        self.decode(self.request(Method::GET, &path).query(&query))
            .await
    }

    /// Retention for versioned uploads: keeps the newest `keep` objects
    /// under `prefix` (version suffixes are sortable UTC timestamps), deletes
    /// the rest.
    pub async fn prune_bucket_objects(
        &self,
        id: &str,
        prefix: &str,
        keep: i64,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!(
                    "/api/atlas/v1/buckets/{}/objects/prune",
                    urlencoding::encode(id)
                ),
            )
            .json(&serde_json::json!({ "prefix": prefix, "keep": keep })),
        )
        .await
    }

    pub async fn list_backups(&self, volume_id: Option<&str>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/backups");
        if let Some(volume_id) = volume_id {
            req = req.query(&[("volume_id", volume_id)]);
        }
        self.decode(req).await
    }

    pub async fn get_backup(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/backups/{}", urlencoding::encode(id)),
        ))
        .await
    }

    /// Removes the backup's S3 objects + RBD snapshot + inventory row.
    pub async fn delete_backup(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!("/api/atlas/v1/backups/{}", urlencoding::encode(id)),
        ))
        .await
    }

    /// A time-limited presigned S3 URL for the backup object -- the client
    /// downloads straight from RGW, no proxy. `what`: `"manifest"`
    /// (default) or `"data"` (the `.rbd-diff` object).
    pub async fn download_backup(
        &self,
        id: &str,
        what: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let path = format!("/api/atlas/v1/backups/{}/download", urlencoding::encode(id));
        let mut req = self.request(Method::GET, &path);
        if let Some(what) = what {
            req = req.query(&[("what", what)]);
        }
        self.decode(req).await
    }

    /// Snapshots a volume and writes a backup manifest to an RGW bucket.
    pub async fn create_backup(
        &self,
        request: CreateBackupRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/backup-jobs")
                .json(&request),
        )
        .await
    }

    /// Verifies the backup manifest in RGW, then provisions a new PVC from
    /// the backup's snapshot.
    pub async fn create_restore(
        &self,
        request: CreateRestoreRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/restore-jobs")
                .json(&request),
        )
        .await
    }

    // ── Disaster recovery (RBD mirroring) -- Atlas's own source labels this
    // "scaffolding, real ops UNVERIFIED without a 2nd cluster"; see
    // docs/ATLAS_INTEGRATION.md's Disaster recovery section. ──

    pub async fn list_dr_peers(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/dr/peers"))
            .await
    }

    /// Registers a mirroring peer cluster (admin). `secret_ref` should name a
    /// k8s Secret holding the peer bootstrap token -- never the token itself.
    pub async fn register_dr_peer(
        &self,
        request: RegisterDrPeerRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/dr/peers")
                .json(&request),
        )
        .await
    }

    /// Removes a mirroring peer and any mirrors that referenced it (admin).
    pub async fn delete_dr_peer(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!("/api/atlas/v1/dr/peers/{}", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn list_dr_mirrors(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/dr/mirrors"))
            .await
    }

    /// DR posture: mirror counts by role/state + worst observed RPO.
    pub async fn dr_status(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/dr/status"))
            .await
    }

    /// Control-plane checklist before a failover drill -- `ready`/`blockers`
    /// in the response is the real gate, this call always returns `200`.
    pub async fn dr_preflight(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/dr/preflight"))
            .await
    }

    /// Enables RBD mirroring for a volume (admin, async job). `mode` must be
    /// `snapshot` or `journal` (Atlas default: `snapshot`); `peer` picks a
    /// registered DR peer, defaulting to the only one if exactly one exists.
    pub async fn enable_mirror(
        &self,
        volume_id: &str,
        mode: Option<&str>,
        peer: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/volumes/{}/mirror",
                urlencoding::encode(volume_id)
            ),
        );
        let mut query = Vec::new();
        if let Some(mode) = mode {
            query.push(("mode", mode));
        }
        if let Some(peer) = peer {
            query.push(("peer", peer));
        }
        if !query.is_empty() {
            req = req.query(&query);
        }
        self.decode(req).await
    }

    /// Disables RBD mirroring for a volume (admin, async job).
    pub async fn disable_mirror(&self, volume_id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!(
                "/api/atlas/v1/volumes/{}/mirror",
                urlencoding::encode(volume_id)
            ),
        ))
        .await
    }

    /// Failover: promote this cluster's copy to primary (admin, async job).
    /// `force` is for split-brain / non-clean failover only (`rbd mirror
    /// image promote --force`) -- without it, promote requires the mirror to
    /// currently be `role=secondary`.
    pub async fn promote_mirror(&self, id: &str, force: bool) -> Result<serde_json::Value, Error> {
        let mut req = self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/dr/mirrors/{}/promote",
                urlencoding::encode(id)
            ),
        );
        if force {
            req = req.query(&[("force", "true")]);
        }
        self.decode(req).await
    }

    /// Demote this cluster's copy to secondary (admin, async job).
    pub async fn demote_mirror(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/dr/mirrors/{}/demote",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Records an observed RPO sample for a mirror (operator).
    pub async fn set_mirror_rpo(
        &self,
        id: &str,
        rpo_seconds: Option<i64>,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!("/api/atlas/v1/dr/mirrors/{}/rpo", urlencoding::encode(id)),
            )
            .json(&serde_json::json!({ "rpo_seconds": rpo_seconds })),
        )
        .await
    }

    /// One-click failover runbook: runs preflight, then promotes `mirror_id`
    /// (admin, async job). `confirm: true` is required -- Atlas rejects the
    /// request with a `400` otherwise since this is destructive. If
    /// preflight isn't `ready`, Atlas rejects with a `409` naming the
    /// blockers unless `force` is also set.
    pub async fn dr_failover(
        &self,
        request: DrFailoverRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/dr/failover")
                .json(&request),
        )
        .await
    }

    // ── AI-assisted insights -- compute-only (recommendation queries, not
    // state mutation) despite the POST verbs on advisor/what-if. The local
    // advisor is always available and never mutates storage; an optional
    // external provider can only rewrite the executive summary text.

    /// Risk-scored advisory with evidence and recommended (not executed)
    /// actions. `can_execute` in the response is always `false`.
    pub async fn ai_advisor(&self, request: AiAdvisorRequest) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/ai/advisor")
                .json(&request),
        )
        .await
    }

    /// Statistical anomaly detection over recent metric samples.
    /// `sensitivity` is a median-absolute-deviation multiplier (Atlas
    /// default: `3.5`); `minutes` is the lookback window (default `360`).
    pub async fn ai_anomalies(
        &self,
        minutes: Option<i64>,
        sensitivity: Option<f64>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/ai/anomalies");
        let mut query = Vec::new();
        if let Some(minutes) = minutes {
            query.push(("minutes".to_string(), minutes.to_string()));
        }
        if let Some(sensitivity) = sensitivity {
            query.push(("sensitivity".to_string(), sensitivity.to_string()));
        }
        if !query.is_empty() {
            req = req.query(&query);
        }
        self.decode(req).await
    }

    /// Correlated incidents (alerts + anomalies + failed jobs grouped by
    /// likely cause). `mode` defaults to `local` here (unlike the advisor's
    /// `auto` default) -- narration is strictly opt-in for this GET
    /// endpoint since `auto`/`llm` can trigger a real external-network call.
    pub async fn ai_incidents(&self, mode: Option<&str>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/ai/incidents");
        if let Some(mode) = mode {
            req = req.query(&[("mode", mode)]);
        }
        self.decode(req).await
    }

    /// Projects capacity/risk under a hypothetical -- never mutates
    /// anything, purely a forward projection over current inventory.
    pub async fn ai_what_if(&self, request: AiWhatIfRequest) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/ai/what-if")
                .json(&request),
        )
        .await
    }

    // ── Observability: metrics, alerts, audit, chargeback, policy drift,
    // events. All read-only except the alert lifecycle actions (evaluate,
    // ack, silence, resolve).

    pub async fn metrics_summary(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/metrics/summary"))
            .await
    }

    pub async fn metrics_ceph(&self, prefix: Option<&str>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/metrics/ceph");
        if let Some(prefix) = prefix {
            req = req.query(&[("prefix", prefix)]);
        }
        self.decode(req).await
    }

    /// Persisted capacity/IO/job time-series for trend charts. `minutes`
    /// clamped by Atlas to `[1, 2880]`, default `60`.
    pub async fn metrics_history(&self, minutes: Option<i64>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/metrics/history");
        if let Some(minutes) = minutes {
            req = req.query(&[("minutes", minutes.to_string())]);
        }
        self.decode(req).await
    }

    /// Least-squares projection of days-until-full. `minutes` clamped by
    /// Atlas to `[1, 20160]`, default `1440`.
    pub async fn metrics_forecast(&self, minutes: Option<i64>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/metrics/forecast");
        if let Some(minutes) = minutes {
            req = req.query(&[("minutes", minutes.to_string())]);
        }
        self.decode(req).await
    }

    pub async fn list_alerts(&self, state: Option<&str>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/alerts");
        if let Some(state) = state {
            req = req.query(&[("state", state)]);
        }
        self.decode(req).await
    }

    /// Runs the alert rules on demand (they also run on Atlas's own monitor
    /// interval) -- a genuine side effect (creates/updates alert rows), not
    /// a pure read, despite returning just a count.
    pub async fn evaluate_alerts(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::POST, "/api/atlas/v1/alerts/evaluate"))
            .await
    }

    /// Records who saw an alert (operator, not a resolve).
    pub async fn ack_alert(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!("/api/atlas/v1/alerts/{}/ack", urlencoding::encode(id)),
        ))
        .await
    }

    /// Suppresses webhook notification for a window (default 1h, capped at
    /// 30d) -- the condition keeps being tracked and still shows in
    /// `list_alerts`, it just won't re-notify until the window elapses.
    pub async fn silence_alert(
        &self,
        id: &str,
        secs: Option<i64>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(
            Method::POST,
            &format!("/api/atlas/v1/alerts/{}/silence", urlencoding::encode(id)),
        );
        if let Some(secs) = secs {
            req = req.query(&[("secs", secs.to_string())]);
        }
        self.decode(req).await
    }

    /// Operator override to resolve an open alert.
    pub async fn resolve_alert(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!("/api/atlas/v1/alerts/{}/resolve", urlencoding::encode(id)),
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_audit(
        &self,
        actor: Option<&str>,
        action: Option<&str>,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        limit: Option<i64>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/audit");
        let mut query = Vec::new();
        if let Some(actor) = actor {
            query.push(("actor", actor.to_string()));
        }
        if let Some(action) = action {
            query.push(("action", action.to_string()));
        }
        if let Some(resource_type) = resource_type {
            query.push(("resource_type", resource_type.to_string()));
        }
        if let Some(resource_id) = resource_id {
            query.push(("resource_id", resource_id.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        if !query.is_empty() {
            req = req.query(&query);
        }
        self.decode(req).await
    }

    /// Raw CSV, not JSON -- Zorvia relays it byte-for-byte with the same
    /// `text/csv` content type rather than wrapping it.
    pub async fn export_audit_csv(&self) -> Result<String, Error> {
        let response = self
            .request(Method::GET, "/api/atlas/v1/audit.csv")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(Self::decode_error(status, body))
        }
    }

    /// Per-tenant usage + optional cost (showback). Rate is configured on
    /// Atlas's side via `ATLAS_CHARGEBACK_USD_PER_GIB_MONTH` (`0` = usage
    /// only) -- a point-in-time snapshot, not a billing record.
    pub async fn chargeback(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/chargeback"))
            .await
    }

    /// Volumes whose applied StorageClass no longer matches their policy
    /// (or whose policy was deleted) -- configuration drift from intent.
    pub async fn policy_drift(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/policy-drift"))
            .await
    }

    /// Unified activity feed (jobs + audit + alerts), newest first.
    pub async fn list_events(&self, limit: Option<i64>) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/events");
        if let Some(limit) = limit {
            req = req.query(&[("limit", limit.to_string())]);
        }
        self.decode(req).await
    }

    // ── Governance: tenants, quotas, policy overrides, protection
    // schedules, volume labels/bindings. Deliberately excludes Atlas's own
    // console auth (`/auth/login`, `/auth/users*`, `/auth/tokens*`) -- those
    // manage Atlas's own accounts, not Zorvia's, and proxying them would
    // make a compromised or misconfigured Zorvia a pass-through admin
    // console for a different product's user base.

    /// Overview of every tenant with volumes or a quota (usage + limits).
    pub async fn list_tenants(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/tenants"))
            .await
    }

    /// The global policy templates (intent -> default storage-class
    /// placement), not tenant-specific.
    pub async fn list_policies(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/policies"))
            .await
    }

    /// A tenant's policy overrides (empty if it uses the global defaults).
    pub async fn list_tenant_policies(&self, tenant_id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/tenants/{}/policies",
                urlencoding::encode(tenant_id)
            ),
        ))
        .await
    }

    /// Overrides an intent's placement for one tenant (admin).
    pub async fn put_tenant_policy(
        &self,
        tenant_id: &str,
        intent: &str,
        request: TenantPolicyRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::PUT,
                &format!(
                    "/api/atlas/v1/tenants/{}/policies/{}",
                    urlencoding::encode(tenant_id),
                    urlencoding::encode(intent)
                ),
            )
            .json(&request),
        )
        .await
    }

    pub async fn delete_tenant_policy(
        &self,
        tenant_id: &str,
        intent: &str,
    ) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!(
                "/api/atlas/v1/tenants/{}/policies/{}",
                urlencoding::encode(tenant_id),
                urlencoding::encode(intent)
            ),
        ))
        .await
    }

    /// The tenant's quota + current usage (unlimited `0`/`0` if unset).
    pub async fn get_tenant_quota(&self, tenant_id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/tenants/{}/quota",
                urlencoding::encode(tenant_id)
            ),
        ))
        .await
    }

    pub async fn put_tenant_quota(
        &self,
        tenant_id: &str,
        request: TenantQuotaRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::PUT,
                &format!(
                    "/api/atlas/v1/tenants/{}/quota",
                    urlencoding::encode(tenant_id)
                ),
            )
            .json(&request),
        )
        .await
    }

    /// Creates a protection schedule for a volume (operator). Both
    /// `snapshot` and `backup` schedules dispatch a CSI VolumeSnapshot job
    /// under the hood, which needs a PVC-backed (k8s) volume -- Atlas
    /// rejects a schedule against a raw NFS/ZFS volume with a `400` rather
    /// than creating a permanently-no-op schedule.
    pub async fn create_schedule(
        &self,
        volume_id: &str,
        request: CreateScheduleRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::POST,
                &format!(
                    "/api/atlas/v1/volumes/{}/schedule",
                    urlencoding::encode(volume_id)
                ),
            )
            .json(&request),
        )
        .await
    }

    pub async fn list_schedules(
        &self,
        volume_id: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/schedules");
        if let Some(volume_id) = volume_id {
            req = req.query(&[("volume_id", volume_id)]);
        }
        self.decode(req).await
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!("/api/atlas/v1/schedules/{}", urlencoding::encode(id)),
        ))
        .await
    }

    pub async fn get_volume_labels(&self, volume_id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/volumes/{}/labels",
                urlencoding::encode(volume_id)
            ),
        ))
        .await
    }

    /// Merges labels into the volume (operator) -- `labels` is a flat JSON
    /// object of string key/value pairs; existing keys not present in the
    /// call are left untouched.
    pub async fn put_volume_labels(
        &self,
        volume_id: &str,
        labels: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(
                Method::PUT,
                &format!(
                    "/api/atlas/v1/volumes/{}/labels",
                    urlencoding::encode(volume_id)
                ),
            )
            .json(&labels),
        )
        .await
    }

    /// Sibling-product ownership bindings for a volume (the `owner` tag set
    /// at create time, e.g. `{product: "zorvia", resource_type: "vm", ...}`).
    pub async fn list_volume_bindings(&self, volume_id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/volumes/{}/bindings",
                urlencoding::encode(volume_id)
            ),
        ))
        .await
    }

    // ── DataBridge: cloud-to-edge DB migration. A genuinely different
    // product domain from VM/storage provisioning -- surfaced on its own
    // top-level page, not the Storage page.

    pub async fn db_list_sources(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/sources"))
            .await
    }

    pub async fn db_get_source(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/databridge/sources/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Registers a cloud/source database -- synchronous, not a job.
    pub async fn db_create_source(
        &self,
        request: CreateDataBridgeSourceRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/databridge/sources")
                .json(&request),
        )
        .await
    }

    /// Refused (`409`) while any migration plan still references this
    /// source -- `migration_plans.source_id` is `ON DELETE CASCADE`, so an
    /// unguarded delete would silently destroy a plan's entire migration
    /// history, including one already cut over to production.
    pub async fn db_delete_source(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!(
                "/api/atlas/v1/databridge/sources/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Discovers the source's schema -- async job.
    pub async fn db_discover_source(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/sources/{}/discover",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_list_plans(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/plans"))
            .await
    }

    pub async fn db_get_plan(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!("/api/atlas/v1/databridge/plans/{}", urlencoding::encode(id)),
        ))
        .await
    }

    /// Creates a migration plan for a source -- synchronous, not a job.
    pub async fn db_create_plan(
        &self,
        request: CreateMigrationPlanRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/databridge/plans")
                .json(&request),
        )
        .await
    }

    pub async fn db_delete_plan(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!("/api/atlas/v1/databridge/plans/{}", urlencoding::encode(id)),
        ))
        .await
    }

    /// Scores readiness from the source's discovered schema -- async job.
    /// Requires the source to already be `discovered`.
    pub async fn db_assess_plan(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/assess",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Provisions the edge DB cluster on Ceph -- async job.
    pub async fn db_provision_edge(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/provision",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_full_load(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/full-load",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_cdc_start(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/cdc/start",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_cdc_stop(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/cdc/stop",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Re-establishes a stalled/errored CDC stream (self-heal).
    pub async fn db_cdc_restart(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/cdc/restart",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_validate(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/validate",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Guarded (Atlas requires `ROLE_ADMIN`): the plan must be `validated`,
    /// its last validation must have `passed`, and (if a CDC stream is
    /// attached) lag must be under Atlas's 10s cutover threshold -- Atlas
    /// enforces all three itself and rejects with a real `409` naming which
    /// precondition failed, not just a generic error.
    pub async fn db_cutover(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/cutover",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Guarded (Atlas requires `ROLE_ADMIN`): only within the cutover's
    /// rollback window (set at plan creation, default 72h) -- Atlas
    /// rejects with a `409` once the deadline has passed, or if there was
    /// never a cutover to roll back.
    pub async fn db_rollback(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/plans/{}/rollback",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_list_edge_clusters(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/edge-clusters"))
            .await
    }

    pub async fn db_get_edge_cluster(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/databridge/edge-clusters/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Removes a stale/orphaned edge-cluster inventory row -- the operator
    /// CR, if any, is torn down separately.
    pub async fn db_delete_edge_cluster(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::DELETE,
            &format!(
                "/api/atlas/v1/databridge/edge-clusters/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_list_cdc_streams(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/cdc-streams"))
            .await
    }

    pub async fn db_get_cdc_stream(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/databridge/cdc-streams/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_list_object_migrations(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/object"))
            .await
    }

    pub async fn db_get_object_migration(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::GET,
            &format!(
                "/api/atlas/v1/databridge/object/{}",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    /// Registers an S3-protocol object-store migration -- synchronous; no
    /// copy happens until `db_start_object_migration`.
    pub async fn db_create_object_migration(
        &self,
        request: CreateObjectMigrationRequest,
    ) -> Result<serde_json::Value, Error> {
        self.decode(
            self.request(Method::POST, "/api/atlas/v1/databridge/object")
                .json(&request),
        )
        .await
    }

    /// Atlas returns `204 No Content` on success here, unlike every other
    /// delete route in this client -- there's no body to decode.
    pub async fn db_delete_object_migration(&self, id: &str) -> Result<(), Error> {
        let response = self
            .request(
                Method::DELETE,
                &format!(
                    "/api/atlas/v1/databridge/object/{}",
                    urlencoding::encode(id)
                ),
            )
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::decode_error(
                status,
                response.text().await.unwrap_or_default(),
            ))
        }
    }

    /// Enqueues the copy job -- async job.
    pub async fn db_start_object_migration(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(
            Method::POST,
            &format!(
                "/api/atlas/v1/databridge/object/{}/start",
                urlencoding::encode(id)
            ),
        ))
        .await
    }

    pub async fn db_list_validations(
        &self,
        plan_id: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let mut req = self.request(Method::GET, "/api/atlas/v1/databridge/validations");
        if let Some(plan_id) = plan_id {
            req = req.query(&[("plan_id", plan_id)]);
        }
        self.decode(req).await
    }

    pub async fn db_list_cutovers(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/api/atlas/v1/databridge/cutovers"))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalizes_slashes() {
        let client = Client::new(Config {
            base_url: "http://127.0.0.1:9090".into(),
            token: None,
            default_tenant_id: Some("global".into()),
            timeout: Duration::from_secs(1),
            allow_invalid_tls: false,
        })
        .unwrap();
        assert_eq!(
            client.endpoint("/api/atlas/v1/backends"),
            "http://127.0.0.1:9090/api/atlas/v1/backends"
        );
    }

    #[test]
    fn atlas_error_envelope_is_preserved() {
        let err = Client::decode_error(
            StatusCode::CONFLICT,
            r#"{"error":{"code":"CONFLICT","message":"tenant quota exceeded"}}"#.into(),
        );
        match err {
            Error::Upstream {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 409);
                assert_eq!(code.as_deref(), Some("CONFLICT"));
                assert_eq!(message, "tenant quota exceeded");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn missing_tenant_is_rejected_without_a_default() {
        let client = Client::new(Config {
            base_url: "http://127.0.0.1:9090".into(),
            token: None,
            default_tenant_id: None,
            timeout: Duration::from_secs(1),
            allow_invalid_tls: false,
        })
        .unwrap();
        assert!(matches!(client.tenant_id(None), Err(Error::MissingTenant)));
        assert_eq!(client.tenant_id(Some("acme")).unwrap(), "acme");
    }

    // ── Live-instance checks -- not run by default, no Atlas instance in CI.
    // Start one first: `cd ../atlas && cargo run -p atlas-gateway` (dev config,
    // fake Ceph driver, no auth required, listens on 127.0.0.1:5110 by default
    // -- see `../atlas/README.md`'s Quickstart), then:
    //   cargo test --features web -p zorvia atlas::client::tests::live -- --ignored --nocapture

    fn live_client() -> Client {
        Client::new(Config {
            base_url: "http://127.0.0.1:5110".into(),
            token: None,
            default_tenant_id: Some("global".into()),
            timeout: Duration::from_secs(5),
            allow_invalid_tls: false,
        })
        .unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn live_health_and_version() {
        let c = live_client();
        let health = c.health().await.unwrap();
        assert_eq!(health["status"], "ok");
        c.version().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_list_backends_matches_real_contract() {
        let c = live_client();
        let backends = c.list_backends().await.unwrap();
        assert!(!backends.is_empty(), "fake driver seeds one backend");
        assert_eq!(backends[0].backend_type, BackendType::Ceph);
    }

    #[tokio::test]
    #[ignore]
    async fn live_list_clusters_pools_volumes() {
        let c = live_client();
        let clusters = c.list_clusters().await.unwrap();
        assert!(clusters.as_array().is_some_and(|a| !a.is_empty()));
        let pools = c.list_pools().await.unwrap();
        assert!(pools.as_array().is_some_and(|a| !a.is_empty()));
        // Fake driver seeds volumes too; this just confirms the route/shape,
        // not a specific count.
        c.list_volumes().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_create_volume_round_trips() {
        let c = live_client();
        let name = format!("zorvia-live-test-{}", chrono::Utc::now().timestamp_millis());
        let result = c
            .create_volume(CreateVolumeRequest {
                tenant_id: String::new(), // filled from default_tenant_id
                name: name.clone(),
                size_bytes: 1024 * 1024 * 1024,
                kind: VolumeKind::Block,
                policy: None,
                pool: None,
                owner: Some(Owner::for_vm("zorvia-live-test-vm")),
                kubernetes: None,
            })
            .await
            .unwrap();
        // create_volume's handler returns (StatusCode, Json<Value>) -- assert
        // it's a real object, not an error envelope masquerading as 200.
        assert!(
            result.get("error").is_none(),
            "unexpected error in response: {result:?}"
        );
    }

    // The fake driver seeds `vol_rbd_nvme_prod_web-01-root` and
    // `vol_rbd_nvme_prod_billing-db-01-root` on every fresh `atlas.db` -- see
    // `../atlas/crates/atlas-discovery`'s fake-mode seed data. Used here so
    // expand/delete have a real volume to act on without depending on
    // create_volume's own job (which fails in this dev setup -- no
    // ATLAS_KUBECONFIG attached, see the module doc above).

    #[tokio::test]
    #[ignore]
    async fn live_expand_volume_round_trips() {
        let c = live_client();
        let result = c
            .expand_volume("vol_rbd_nvme_prod_web-01-root", 50 * 1024 * 1024 * 1024)
            .await
            .unwrap();
        assert!(
            result.get("error").is_none(),
            "unexpected error in response: {result:?}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_delete_volume_requires_confirm_for_protected_class() {
        let c = live_client();
        // Without confirm: Atlas's own protected-class check should reject this
        // with a real, specific error -- not silently succeed.
        let err = c
            .delete_volume("vol_rbd_nvme_prod_billing-db-01-root", false)
            .await
            .unwrap_err();
        match err {
            Error::Upstream {
                status, message, ..
            } => {
                assert_eq!(status, 400);
                assert!(
                    message.contains("confirm=true"),
                    "expected the confirm-required message, got: {message}"
                );
            }
            other => panic!("expected an Upstream 400, got: {other:?}"),
        }

        // With confirm: accepted.
        let result = c
            .delete_volume("vol_rbd_nvme_prod_billing-db-01-root", true)
            .await
            .unwrap();
        assert!(
            result.get("error").is_none(),
            "unexpected error in response: {result:?}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_job_list_get_and_cancel_round_trip() {
        let c = live_client();

        // Create a volume to get a real job id -- exercises list/get end to
        // end against an actual job this instance just created, not a
        // fabricated id.
        let name = format!(
            "zorvia-live-job-test-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let create_result = c
            .create_volume(CreateVolumeRequest {
                tenant_id: String::new(),
                name,
                size_bytes: 1024 * 1024 * 1024,
                kind: VolumeKind::Block,
                policy: None,
                pool: None,
                owner: None,
                kubernetes: None,
            })
            .await
            .unwrap();
        let job_id = create_result["job_id"]
            .as_str()
            .expect("create_volume response has a job_id")
            .to_string();

        let jobs = c.list_jobs().await.unwrap();
        assert!(
            jobs.iter().any(|j| j.id == job_id),
            "expected job {job_id} in list_jobs()"
        );

        let job = c.get_job(&job_id).await.unwrap();
        assert_eq!(job.id, job_id);
        assert_eq!(job.job_type, "volume.create");

        // This local instance has no Kubernetes cluster attached (see the
        // create_volume test above), so the job fails fast rather than
        // staying queued/running -- cancel on an already-terminal job
        // should get Atlas's real 409, not silently succeed.
        if job.is_terminal() {
            let err = c.cancel_job(&job_id).await.unwrap_err();
            match err {
                Error::Upstream { status, .. } => assert_eq!(status, 409),
                other => panic!("expected an Upstream 409, got: {other:?}"),
            }
        } else {
            let result = c.cancel_job(&job_id).await.unwrap();
            assert_eq!(result["cancelled"], true);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_maintenance_reads_and_preflight() {
        let c = live_client();
        let maint = c.get_maintenance().await.unwrap();
        assert!(maint.get("paused").is_some());
        c.list_orphans().await.unwrap();
        let preflight = c.upgrade_preflight().await.unwrap();
        // Report-only: always 200, `ready` is the real gate -- just confirm
        // the shape, not a particular verdict.
        assert!(preflight.get("ready").is_some() || preflight.get("blockers").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_set_maintenance_round_trips() {
        let c = live_client();
        let paused = c.set_maintenance(true).await.unwrap();
        assert_eq!(paused["paused"], true);
        // Always resume -- leaving the fake driver's job engine paused would
        // silently wedge every other live test run against this instance.
        let resumed = c.set_maintenance(false).await.unwrap();
        assert_eq!(resumed["paused"], false);
    }

    #[tokio::test]
    #[ignore]
    async fn live_backend_create_cordon_uncordon_delete_round_trip() {
        let c = live_client();
        let backend = c
            .create_backend(CreateBackendRequest {
                name: format!(
                    "zorvia-live-test-backend-{}",
                    chrono::Utc::now().timestamp_millis()
                ),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(backend.status, "pending"); // no backend_type given -> defaults to Ceph -> "pending" row, no live driver
        assert!(!backend.cordoned);

        let cordoned = c.cordon_backend(&backend.id).await.unwrap();
        assert_eq!(cordoned["cordoned"], true);
        let uncordoned = c.uncordon_backend(&backend.id).await.unwrap();
        assert_eq!(uncordoned["cordoned"], false);

        let deleted = c.delete_backend(&backend.id, false).await.unwrap();
        assert_eq!(deleted["deleted"], true);
    }

    #[tokio::test]
    #[ignore]
    async fn live_osd_list_and_out_in_round_trip() {
        let c = live_client();
        let osds = c.list_osds().await.unwrap();
        let osd_id = osds
            .as_array()
            .and_then(|a| a.first())
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_i64())
            .expect("fake driver seeds at least one OSD");

        let out_result = c.osd_out(osd_id).await.unwrap();
        assert!(out_result.get("error").is_none());
        let in_result = c.osd_in(osd_id).await.unwrap();
        assert!(in_result.get("error").is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_rbd_image_full_lifecycle() {
        let c = live_client();
        let pool = "rbd-nvme-prod"; // seeded by the fake driver, see live_list_backends... fixtures
        let name = format!("zorvia-live-rbd-{}", chrono::Utc::now().timestamp_millis());

        let created = c
            .create_rbd_image(CreateRbdImageRequest {
                name: name.clone(),
                size_bytes: 1024 * 1024 * 1024,
                pool: Some(pool.to_string()),
                tenant_id: None,
            })
            .await
            .unwrap();
        assert!(created.get("error").is_none(), "create failed: {created:?}");

        let resized = c
            .resize_rbd_image(pool, &name, 2 * 1024 * 1024 * 1024, false)
            .await
            .unwrap();
        assert!(resized.get("error").is_none());

        let qos = c.qos_rbd_image(pool, &name, Some(500), None).await.unwrap();
        assert!(qos.get("error").is_none());

        let snap_name = "zorvia-live-snap";
        let snap = c.create_rbd_snapshot(pool, &name, snap_name).await.unwrap();
        assert!(snap.get("error").is_none());

        let snaps = c.list_rbd_snapshots(pool, &name).await.unwrap();
        assert!(snaps.get("error").is_none());

        let rolled_back = c.rollback_rbd_image(pool, &name, snap_name).await.unwrap();
        assert!(rolled_back.get("error").is_none());

        let images = c.list_rbd_images(Some(pool)).await.unwrap();
        assert!(images.get("error").is_none());

        let usage = c.refresh_rbd_usage().await.unwrap();
        assert!(usage.get("updated").is_some());

        // Clean up: snapshot first, then the image, same order Atlas itself
        // requires (a protected snapshot backing a clone would refuse
        // image delete, though this test never cloned).
        let snap_deleted = c.delete_rbd_snapshot(pool, &name, snap_name).await.unwrap();
        assert!(snap_deleted.get("error").is_none());
        let deleted = c.delete_rbd_image(pool, &name).await.unwrap();
        assert!(deleted.get("error").is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_rbd_clone_and_flatten() {
        let c = live_client();
        let pool = "rbd-nvme-prod";
        let parent = format!(
            "zorvia-live-rbd-parent-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let clone_name = format!("{parent}-clone");

        c.create_rbd_image(CreateRbdImageRequest {
            name: parent.clone(),
            size_bytes: 1024 * 1024 * 1024,
            pool: Some(pool.to_string()),
            tenant_id: None,
        })
        .await
        .unwrap();

        let cloned = c
            .clone_rbd_image(
                pool,
                &parent,
                CloneRbdImageRequest {
                    name: clone_name.clone(),
                    snap: None,
                    tenant_id: None,
                },
            )
            .await
            .unwrap();
        assert!(cloned.get("error").is_none(), "clone failed: {cloned:?}");

        let flattened = c.flatten_rbd_image(pool, &clone_name).await.unwrap();
        assert!(flattened.get("error").is_none());

        // Clean up both images (clone first -- it may still reference the
        // parent's auto-created `<clone>-base` snapshot).
        c.delete_rbd_image(pool, &clone_name).await.unwrap();
        c.delete_rbd_image(pool, &parent).await.unwrap();
    }

    // Bucket/backup *creation* needs a real Kubernetes cluster attached to
    // Atlas (ObjectBucketClaim provisioning) -- same constraint as volume
    // create, confirmed live: the create call itself is accepted (202 +
    // job envelope) but the job then fails with "no Kubernetes cluster is
    // attached" against this no-kubeconfig local instance, and (confirmed
    // live) leaves no orphan inventory row behind. So these tests verify
    // the request/response wiring -- real shapes, real error codes -- not
    // a full happy path, which needs a real cluster to exercise.

    #[tokio::test]
    #[ignore]
    async fn live_bucket_create_returns_job_envelope() {
        let c = live_client();
        let created = c
            .create_bucket(CreateBucketRequest {
                name: format!("zorvia-live-bkt-{}", chrono::Utc::now().timestamp_millis()),
                namespace: None,
                storage_class: None,
                max_objects: None,
                max_size: None,
            })
            .await
            .unwrap();
        assert!(created.get("job_id").is_some());
        assert_eq!(created["state"], "queued");
    }

    #[tokio::test]
    #[ignore]
    async fn live_bucket_and_backup_reads() {
        let c = live_client();
        c.list_buckets().await.unwrap();
        c.list_backups(None).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_bucket_and_backup_404_on_missing_id() {
        let c = live_client();
        let err = c
            .delete_bucket("bkt_does-not-exist", false)
            .await
            .unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 404),
            other => panic!("expected an Upstream 404, got: {other:?}"),
        }

        let err = c
            .create_backup(CreateBackupRequest {
                volume_id: "vol_does-not-exist".into(),
                bucket_id: "bkt_does-not-exist".into(),
                mode: None,
                keep: None,
                max_age_secs: None,
            })
            .await
            .unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 404),
            other => panic!("expected an Upstream 404, got: {other:?}"),
        }
    }

    // ── Disaster recovery (RBD mirroring) -- scaffolding on Atlas's own side,
    // "real ops UNVERIFIED without a 2nd cluster" per its source comment.
    // These live tests exercise the control-plane catalog (peers/mirrors/
    // preflight bookkeeping, job envelopes, role-transition guards) against
    // the fake driver, which is everything verifiable without a second real
    // Ceph cluster -- not a live two-site mirror drill.

    #[tokio::test]
    #[ignore]
    async fn live_dr_peer_and_mirror_lifecycle() {
        let c = live_client();
        let peer = c
            .register_dr_peer(RegisterDrPeerRequest {
                name: format!("zorvia-live-peer-{}", chrono::Utc::now().timestamp_millis()),
                cluster_fsid: None,
                direction: Some("rx-tx".into()),
                secret_ref: Some("secret/dr-peer-token".into()),
            })
            .await
            .unwrap();
        let peer_id = peer["id"].as_str().unwrap().to_string();

        // The fake driver seeds `vol_rbd_nvme_prod_web-01-root` -- see the
        // comment above `live_expand_volume_round_trips`.
        let enabled = c
            .enable_mirror("vol_rbd_nvme_prod_web-01-root", Some("snapshot"), None)
            .await
            .unwrap();
        assert!(enabled.get("job_id").is_some());
        let mirror_id = enabled["resource"]["mirror_id"]
            .as_str()
            .unwrap()
            .to_string();

        let mirrors = c.list_dr_mirrors().await.unwrap();
        assert!(mirrors
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == mirror_id && m["role"] == "primary" && m["state"] == "enabled"));

        // promote while already primary and not forced -- Atlas's own
        // conflict guard, not a client-side check.
        let err = c.promote_mirror(&mirror_id, false).await.unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 409),
            other => panic!("expected an Upstream 409, got: {other:?}"),
        }

        let demoted = c.demote_mirror(&mirror_id).await.unwrap();
        assert!(demoted.get("job_id").is_some());
        let promoted = c.promote_mirror(&mirror_id, false).await.unwrap();
        assert!(promoted.get("job_id").is_some());

        let rpo = c.set_mirror_rpo(&mirror_id, Some(45)).await.unwrap();
        assert_eq!(rpo["rpo_seconds"], 45);

        let disabled = c
            .disable_mirror("vol_rbd_nvme_prod_web-01-root")
            .await
            .unwrap();
        assert!(disabled.get("job_id").is_some());

        // Atlas's delete_peer is unconditional (no rows-affected check), so
        // this always returns 200 -- even for an id that never existed --
        // unlike backend/bucket/backup delete, which do 404. Documented in
        // docs/ATLAS_INTEGRATION.md rather than assumed to match the others.
        let deleted = c.delete_dr_peer(&peer_id).await.unwrap();
        assert_eq!(deleted["deleted"], true);
    }

    #[tokio::test]
    #[ignore]
    async fn live_dr_status_and_preflight_reflect_real_state() {
        let c = live_client();
        let status = c.dr_status().await.unwrap();
        assert!(status.get("peers").is_some());
        let preflight = c.dr_preflight().await.unwrap();
        assert!(preflight.get("ready").is_some());
        assert!(preflight.get("blockers").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_dr_failover_requires_confirm() {
        let c = live_client();
        let err = c
            .dr_failover(DrFailoverRequest {
                mirror_id: "drm_does-not-exist".into(),
                confirm: false,
                force: false,
            })
            .await
            .unwrap_err();
        match err {
            Error::Upstream {
                status, message, ..
            } => {
                assert_eq!(status, 400);
                assert!(
                    message.contains("confirm=true"),
                    "expected the confirm-required message, got: {message}"
                );
            }
            other => panic!("expected an Upstream 400, got: {other:?}"),
        }
    }

    // ── AI-assisted insights -- local deterministic advisor, no external
    // provider configured in this dev setup, so `mode` stays "local".

    #[tokio::test]
    #[ignore]
    async fn live_ai_advisor_returns_real_evidence() {
        let c = live_client();
        let result = c
            .ai_advisor(AiAdvisorRequest {
                question: "is storage healthy?".into(),
                mode: Some("local".into()),
            })
            .await
            .unwrap();
        assert_eq!(result["mode"], "local");
        assert_eq!(result["can_execute"], false);
        assert!(result.get("evidence").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_ai_anomalies_and_incidents_shapes() {
        let c = live_client();
        let anomalies = c.ai_anomalies(Some(360), Some(3.5)).await.unwrap();
        assert_eq!(anomalies["window_minutes"], 360);
        assert!(anomalies.get("anomalies").is_some());

        let incidents = c.ai_incidents(Some("local")).await.unwrap();
        assert!(incidents.get("incidents").is_some());
        assert_eq!(incidents["can_execute"], false);
    }

    #[tokio::test]
    #[ignore]
    async fn live_ai_what_if_projects_without_mutating() {
        let c = live_client();
        let result = c
            .ai_what_if(AiWhatIfRequest {
                add_capacity_bytes: 2 * 1024 * 1024 * 1024 * 1024,
                horizon_days: Some(30),
                projected_growth_bytes_per_day: None,
                assume_alerts_resolved: true,
                assume_recovery_complete: false,
            })
            .await
            .unwrap();
        assert!(result.get("baseline").is_some());
        assert!(result.get("projected").is_some());
        assert_eq!(result["can_execute"], false);
    }

    // ── Observability: metrics, alerts, audit, chargeback, policy drift,
    // events.

    #[tokio::test]
    #[ignore]
    async fn live_metrics_endpoints_return_real_shapes() {
        let c = live_client();
        let summary = c.metrics_summary().await.unwrap();
        assert!(summary.get("used_capacity_bytes").is_some());
        c.metrics_ceph(None).await.unwrap();
        let history = c.metrics_history(Some(60)).await.unwrap();
        assert!(history.as_array().is_some());
        let forecast = c.metrics_forecast(Some(1440)).await.unwrap();
        assert!(forecast.get("days_to_full").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_alert_lifecycle_ack_silence_resolve() {
        let c = live_client();
        // Fake driver seeds an OSD-down + cluster-unhealthy alert only after
        // the monitor rules run once -- evaluate on demand rather than
        // waiting on its interval.
        c.evaluate_alerts().await.unwrap();
        let alerts = c.list_alerts(Some("open")).await.unwrap();
        let alerts = alerts.as_array().unwrap();
        assert!(
            !alerts.is_empty(),
            "fake driver should seed at least one open alert"
        );
        let id = alerts[0]["id"].as_str().unwrap().to_string();

        let acked = c.ack_alert(&id).await.unwrap();
        assert_eq!(acked["id"], id);
        let silenced = c.silence_alert(&id, Some(60)).await.unwrap();
        assert_eq!(silenced["silenced_secs"], 60);
        let resolved = c.resolve_alert(&id).await.unwrap();
        assert_eq!(resolved["state"], "resolved");

        // Resolving an already-resolved alert is a real 404, not silently ok.
        let err = c.resolve_alert(&id).await.unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 404),
            other => panic!("expected an Upstream 404, got: {other:?}"),
        }

        let audit = c
            .list_audit(None, None, None, None, Some(10))
            .await
            .unwrap();
        let audit = audit.as_array().unwrap();
        assert!(audit.iter().any(|a| a["action"] == "alert.resolve"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_audit_csv_chargeback_policy_drift_events() {
        let c = live_client();
        let csv = c.export_audit_csv().await.unwrap();
        assert!(csv.starts_with("created_at,actor_id,action,resource_type,resource_id,status"));

        let chargeback = c.chargeback().await.unwrap();
        assert!(chargeback.get("tenants").is_some());

        let drift = c.policy_drift().await.unwrap();
        assert!(drift.get("drift").is_some());

        let events = c.list_events(Some(5)).await.unwrap();
        assert!(events.as_array().is_some());
    }

    // ── Governance: tenants, quotas, policy overrides, protection
    // schedules, volume labels/bindings.

    #[tokio::test]
    #[ignore]
    async fn live_tenant_quota_and_policy_override_round_trip() {
        let c = live_client();
        let tenants = c.list_tenants().await.unwrap();
        assert!(tenants.as_array().is_some_and(|a| !a.is_empty()));

        let policies = c.list_policies().await.unwrap();
        assert!(policies.as_array().is_some_and(|a| !a.is_empty()));

        let quota = c
            .put_tenant_quota(
                "global",
                TenantQuotaRequest {
                    max_bytes: 10 * 1024 * 1024 * 1024 * 1024,
                    max_volumes: 50,
                },
            )
            .await
            .unwrap();
        assert_eq!(quota["max_volumes"], 50);
        let fetched = c.get_tenant_quota("global").await.unwrap();
        assert_eq!(fetched["max_volumes"], 50);

        let overridden = c
            .put_tenant_policy(
                "global",
                "production",
                TenantPolicyRequest {
                    storage_class: "zyvor-rbd-prod-fast".into(),
                    access_mode: None,
                    volume_mode: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(overridden["storage_class"], "zyvor-rbd-prod-fast");
        let listed = c.list_tenant_policies("global").await.unwrap();
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["intent"] == "production"));

        c.delete_tenant_policy("global", "production")
            .await
            .unwrap();
        let err = c
            .delete_tenant_policy("global", "production")
            .await
            .unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 404),
            other => panic!("expected an Upstream 404, got: {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_schedule_and_volume_labels_bindings_round_trip() {
        let c = live_client();
        // Fake driver seeds this PVC-backed volume -- see the comment above
        // `live_expand_volume_round_trips`.
        let volume_id = "vol_rbd_nvme_prod_web-01-root";

        let created = c
            .create_schedule(
                volume_id,
                CreateScheduleRequest {
                    interval_secs: 3600,
                    keep: 5,
                    kind: Some("snapshot".into()),
                    bucket_id: None,
                    mode: None,
                },
            )
            .await
            .unwrap();
        let schedule_id = created["id"].as_str().unwrap().to_string();

        let listed = c.list_schedules(None).await.unwrap();
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == schedule_id));

        c.delete_schedule(&schedule_id).await.unwrap();
        let err = c.delete_schedule(&schedule_id).await.unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 404),
            other => panic!("expected an Upstream 404, got: {other:?}"),
        }

        let merged = c
            .put_volume_labels(volume_id, serde_json::json!({ "env": "prod" }))
            .await
            .unwrap();
        assert_eq!(merged["env"], "prod");
        let fetched = c.get_volume_labels(volume_id).await.unwrap();
        assert_eq!(fetched["env"], "prod");

        c.list_volume_bindings(volume_id).await.unwrap();
    }

    // ── DataBridge: cloud-to-edge DB migration.

    #[tokio::test]
    #[ignore]
    async fn live_databridge_plan_lifecycle_through_cutover_and_rollback() {
        let c = live_client();
        let suffix = chrono::Utc::now().timestamp_millis();

        let source = c
            .db_create_source(CreateDataBridgeSourceRequest {
                name: format!("zorvia-live-src-{suffix}"),
                kind: "postgres".into(),
                cloud: Some("rds".into()),
                endpoint: Some("prod-pg.rds.amazonaws.com".into()),
                port: Some(5432),
                database: Some("appdb".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let source_id = source["id"].as_str().unwrap().to_string();
        assert_eq!(source["state"], "registered");

        let discovered = c.db_discover_source(&source_id).await.unwrap();
        assert!(discovered.get("job_id").is_some());
        let source_after = poll_until(
            || async { c.db_get_source(&source_id).await.unwrap() },
            |s| s["state"] == "discovered",
        )
        .await;
        assert!(source_after["discovered"].get("tables").is_some());

        let plan = c
            .db_create_plan(CreateMigrationPlanRequest {
                name: format!("zorvia-live-plan-{suffix}"),
                source_id: source_id.clone(),
                rollback_window_secs: None,
            })
            .await
            .unwrap();
        let plan_id = plan["id"].as_str().unwrap().to_string();

        c.db_assess_plan(&plan_id).await.unwrap();
        let plan_after_assess = poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["state"] == "assessed",
        )
        .await;
        assert!(plan_after_assess["readiness_score"].as_i64().unwrap() > 0);

        c.db_provision_edge(&plan_id).await.unwrap();
        let plan_after_provision = poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["state"] == "provisioned",
        )
        .await;
        let edge_cluster_id = plan_after_provision["edge_cluster_id"]
            .as_str()
            .unwrap()
            .to_string();
        let edge_cluster = c.db_get_edge_cluster(&edge_cluster_id).await.unwrap();
        assert_eq!(edge_cluster["state"], "ready");

        c.db_full_load(&plan_id).await.unwrap();
        poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["state"] == "loaded",
        )
        .await;

        // Cutover before validate is refused with a real 409, not a client-side guess.
        let err = c.db_cutover(&plan_id).await.unwrap_err();
        match err {
            Error::Upstream { status, .. } => assert_eq!(status, 409),
            other => panic!("expected an Upstream 409, got: {other:?}"),
        }

        c.db_cdc_start(&plan_id).await.unwrap();
        let plan_after_cdc = poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["cdc_stream_id"].is_string(),
        )
        .await;
        let cdc_stream_id = plan_after_cdc["cdc_stream_id"]
            .as_str()
            .unwrap()
            .to_string();

        c.db_validate(&plan_id).await.unwrap();
        poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["state"] == "validated",
        )
        .await;
        let validations = c.db_list_validations(Some(&plan_id)).await.unwrap();
        assert!(validations
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["state"] == "passed"));

        // Cutover is also refused while CDC lag exceeds Atlas's 10s threshold. The fake driver's
        // reconciler decays lag by 0.6x per tick on a 15s interval (see atlas-databridge's
        // reconcile.rs) -- not instant, so this needs multiple reconcile ticks' worth of margin,
        // not a short poll.
        let drained = poll_until_timeout(
            || async { c.db_get_cdc_stream(&cdc_stream_id).await.unwrap() },
            |s| s["lag_seconds"].as_i64().unwrap_or(999) <= 10,
            Duration::from_secs(60),
        )
        .await;
        assert!(
            drained["lag_seconds"].as_i64().unwrap_or(999) <= 10,
            "CDC lag did not drain under the cutover threshold within 60s: {drained:?}"
        );

        let cutover = c.db_cutover(&plan_id).await.unwrap();
        assert!(cutover.get("job_id").is_some());
        poll_until(
            || async { c.db_get_plan(&plan_id).await.unwrap() },
            |p| p["state"] == "cutover_complete",
        )
        .await;
        let cutovers = c.db_list_cutovers().await.unwrap();
        assert!(cutovers
            .as_array()
            .unwrap()
            .iter()
            .any(|co| co["plan_id"] == plan_id));

        let rollback = c.db_rollback(&plan_id).await.unwrap();
        assert!(rollback.get("job_id").is_some());

        // Cleanup: plan first (source delete is guarded while a plan still references it).
        c.db_delete_plan(&plan_id).await.unwrap();
        c.db_delete_source(&source_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_databridge_source_delete_guarded_by_referencing_plan() {
        let c = live_client();
        let suffix = chrono::Utc::now().timestamp_millis();
        let source = c
            .db_create_source(CreateDataBridgeSourceRequest {
                name: format!("zorvia-live-src-guard-{suffix}"),
                kind: "postgres".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let source_id = source["id"].as_str().unwrap().to_string();
        let plan = c
            .db_create_plan(CreateMigrationPlanRequest {
                name: format!("zorvia-live-plan-guard-{suffix}"),
                source_id: source_id.clone(),
                rollback_window_secs: None,
            })
            .await
            .unwrap();
        let plan_id = plan["id"].as_str().unwrap().to_string();

        let err = c.db_delete_source(&source_id).await.unwrap_err();
        match err {
            Error::Upstream {
                status, message, ..
            } => {
                assert_eq!(status, 409);
                assert!(message.contains("migration plan"));
            }
            other => panic!("expected an Upstream 409, got: {other:?}"),
        }

        c.db_delete_plan(&plan_id).await.unwrap();
        c.db_delete_source(&source_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_databridge_object_migration_lifecycle() {
        let c = live_client();
        let created = c
            .db_create_object_migration(CreateObjectMigrationRequest {
                name: format!(
                    "zorvia-live-objmig-{}",
                    chrono::Utc::now().timestamp_millis()
                ),
                source_endpoint: "s3.amazonaws.com".into(),
                source_bucket: "my-cloud-bucket".into(),
                dest_endpoint: "rgw.zorvia.svc:80".into(),
                dest_bucket: "edge-bucket".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["state"], "created");

        let started = c.db_start_object_migration(&id).await.unwrap();
        assert!(started.get("job_id").is_some());

        c.db_delete_object_migration(&id).await.unwrap();
        // Atlas's delete is unconditional (no rows-affected check), so a second
        // delete of the same id still returns success -- not a 404.
        c.db_delete_object_migration(&id).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_databridge_read_only_lists() {
        let c = live_client();
        c.db_list_sources().await.unwrap();
        c.db_list_plans().await.unwrap();
        c.db_list_edge_clusters().await.unwrap();
        c.db_list_cdc_streams().await.unwrap();
        c.db_list_object_migrations().await.unwrap();
        c.db_list_validations(None).await.unwrap();
        c.db_list_cutovers().await.unwrap();
    }

    /// Polls `get` every 500ms (up to 20s) until `done` is true, returning the
    /// last-seen value -- DataBridge's async jobs (discover/assess/provision/
    /// full-load/cdc-start/validate/cutover) settle almost instantly against
    /// the fake driver, but this avoids a hardcoded sleep racing them.
    async fn poll_until<F, Fut, P>(get: F, done: P) -> serde_json::Value
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = serde_json::Value>,
        P: Fn(&serde_json::Value) -> bool,
    {
        poll_until_timeout(get, done, Duration::from_secs(20)).await
    }

    async fn poll_until_timeout<F, Fut, P>(
        mut get: F,
        done: P,
        timeout: Duration,
    ) -> serde_json::Value
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = serde_json::Value>,
        P: Fn(&serde_json::Value) -> bool,
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let v = get().await;
            if done(&v) || std::time::Instant::now() >= deadline {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
