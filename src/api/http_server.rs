// HTTP Server - Real web server using axum for serving API and dashboard
//
// This module is only compiled when the "web" feature is enabled.

#[cfg(feature = "web")]
pub mod web {
    use crate::api::leader::spawn_leader_election;
    use crate::api::webhooks::WebhookEvent;
    use crate::api::{ApiResponse, HttpMethod, RequestContext};
    use crate::kube::KubeClient;
    use crate::tui::state::VmInfo;
    use axum::{
        extract::{DefaultBodyLimit, Path, Query, State},
        http::{header, HeaderMap, StatusCode},
        middleware,
        response::{IntoResponse, Json},
        routing::{delete, get, post, put},
        Router,
    };
    use serde::{Deserialize, Serialize};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower_http::cors::{AllowOrigin, CorsLayer};
    use tower_http::services::{ServeDir, ServeFile};
    use tower_http::timeout::TimeoutLayer;

    #[path = "fabric_vm_handlers.rs"]
    mod fabric_vm_handlers;
    mod guest_agent_handlers;
    use fabric_vm_handlers::*;
    use guest_agent_handlers::*;

    #[path = "adopt_handlers.rs"]
    mod adopt_handlers;
    use adopt_handlers::*;

    #[path = "advanced_handlers.rs"]
    mod advanced_handlers;
    use advanced_handlers::*;

    #[path = "kryton_handlers.rs"]
    mod kryton_handlers;
    use kryton_handlers::*;

    #[path = "atlas_handlers.rs"]
    mod atlas_handlers;
    use atlas_handlers::*;

    #[path = "commercial_handlers.rs"]
    mod commercial_handlers;
    use commercial_handlers::*;

    #[path = "support_handlers.rs"]
    mod support_handlers;
    use support_handlers::*;

    #[path = "services_handlers.rs"]
    mod services_handlers;
    use services_handlers::*;

    #[path = "billing_handlers.rs"]
    mod billing_handlers;
    use billing_handlers::*;

    #[path = "ws_proxy_handlers.rs"]
    mod ws_proxy_handlers;
    use ws_proxy_handlers::{ws_console, ws_ssh, ws_vnc};

    #[path = "pod_handlers.rs"]
    mod pod_handlers;
    use pod_handlers::{
        delete_pod_handler, list_namespaces_handler, list_pods_handler, pod_capabilities_handler,
        pod_events_handler, pod_yaml_handler, restart_pod_handler, ws_pod_exec, ws_pod_logs,
    };

    #[path = "hotplug_handlers.rs"]
    mod hotplug_handlers;
    use hotplug_handlers::*;

    #[path = "migration_handlers.rs"]
    mod migration_handlers;
    use migration_handlers::*;

    #[path = "rook_handlers.rs"]
    mod rook_handlers;
    use rook_handlers::*;

    #[path = "disk_network_handlers.rs"]
    mod disk_network_handlers;
    use disk_network_handlers::*;

    #[path = "vm_import_handlers.rs"]
    mod vm_import_handlers;
    use vm_import_handlers::{
        create_import_handler, get_wave_handler, list_imports_handler, preflight_import_handler,
    };

    #[path = "backup_offcluster_handlers.rs"]
    mod backup_offcluster_handlers;
    use backup_offcluster_handlers::{
        create_offcluster_backup_handler, drill_offcluster_handler,
        list_offcluster_backups_handler, restore_offcluster_handler, verify_key_offcluster_handler,
    };

    #[path = "operations_handlers.rs"]
    mod operations_handlers;
    use operations_handlers::{
        cancel_operation_handler, get_operation_handler, list_operations_handler,
        spawn_operations_reconciler,
    };

    #[path = "rescue_handlers.rs"]
    mod rescue_handlers;
    use rescue_handlers::{delete_rescue_job_handler, get_rescue_job_handler, rescue_vm_handler};

    #[path = "drift_handlers.rs"]
    mod drift_handlers;
    use drift_handlers::*;

    #[path = "quota_handlers.rs"]
    mod quota_handlers;
    use quota_handlers::*;

    #[path = "service_map_handlers.rs"]
    mod service_map_handlers;
    use service_map_handlers::*;

    #[path = "audit_handlers.rs"]
    mod audit_handlers;
    use audit_handlers::*;

    #[path = "backup_handlers.rs"]
    mod backup_handlers;
    use backup_handlers::*;

    #[path = "placement_handlers.rs"]
    mod placement_handlers;
    use placement_handlers::*;
    #[path = "devices_handlers.rs"]
    mod devices_handlers;
    use devices_handlers::{
        devices_inventory_handler, devices_permit_handler, devices_preflight_handler,
        devices_unpermit_handler,
    };
    #[path = "maintenance_handlers.rs"]
    mod maintenance_handlers;
    use maintenance_handlers::maintenance_plan_handler;

    #[path = "ha_handlers.rs"]
    mod ha_handlers;
    use ha_handlers::*;

    #[path = "backup_scheduler_handlers.rs"]
    mod backup_scheduler_handlers;
    use backup_scheduler_handlers::*;

    #[path = "storage_handlers.rs"]
    mod storage_handlers;
    use storage_handlers::*;

    #[path = "network_policy_handlers.rs"]
    mod network_policy_handlers;
    use network_policy_handlers::*;

    #[path = "compliance_handlers.rs"]
    mod compliance_handlers;
    use compliance_handlers::*;

    #[path = "security_dashboard_handlers.rs"]
    mod security_dashboard_handlers;
    use security_dashboard_handlers::*;

    #[path = "capacity_handlers.rs"]
    mod capacity_handlers;
    use capacity_handlers::*;

    #[path = "zone_handlers.rs"]
    mod zone_handlers;
    use zone_handlers::*;

    #[path = "analytics_handlers.rs"]
    mod analytics_handlers;
    use analytics_handlers::*;

    #[path = "resource_optimizer_handlers.rs"]
    mod resource_optimizer_handlers;
    use resource_optimizer_handlers::*;

    #[path = "template_handlers.rs"]
    mod template_handlers;
    use template_handlers::*;

    #[path = "power_schedule_handlers.rs"]
    mod power_schedule_handlers;
    use power_schedule_handlers::*;

    #[path = "webhook_handlers.rs"]
    mod webhook_handlers;
    use webhook_handlers::*;

    #[path = "alert_handlers.rs"]
    mod alert_handlers;
    use alert_handlers::*;

    #[path = "warm_pool_handlers.rs"]
    mod warm_pool_handlers;
    use warm_pool_handlers::*;

    /// Simple sliding-window rate limiter state.
    struct RateLimiterState {
        /// Number of requests in the current window
        count: AtomicU64,
        /// Start of the current window (unix timestamp seconds)
        window_start: AtomicU64,
        /// Maximum requests per window
        max_requests: u64,
        /// Window duration in seconds
        window_secs: u64,
    }

    impl RateLimiterState {
        fn new(max_requests: u64, window_secs: u64) -> Self {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Self {
                count: AtomicU64::new(0),
                window_start: AtomicU64::new(now),
                max_requests,
                window_secs,
            }
        }

        fn check_rate_limit(&self) -> bool {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let window_start = self.window_start.load(Ordering::Relaxed);

            // Reset window if expired
            if now - window_start >= self.window_secs {
                self.window_start.store(now, Ordering::Relaxed);
                self.count.store(1, Ordering::Relaxed);
                return true;
            }

            let count = self.count.fetch_add(1, Ordering::Relaxed);
            count < self.max_requests
        }
    }

    /// TLS configuration for the API server.
    #[derive(Clone, Debug)]
    pub struct TlsConfig {
        pub cert_path: String,
        pub key_path: String,
    }

    impl TlsConfig {
        /// Validate that the cert and key files exist on disk.
        pub fn validate(&self) -> anyhow::Result<()> {
            if !std::path::Path::new(&self.cert_path).exists() {
                return Err(anyhow::anyhow!(
                    "TLS certificate file not found: {}",
                    self.cert_path
                ));
            }
            if !std::path::Path::new(&self.key_path).exists() {
                return Err(anyhow::anyhow!("TLS key file not found: {}", self.key_path));
            }
            Ok(())
        }
    }

    pub type SharedAuditTrail = Arc<RwLock<crate::audit_trail::AuditTrail>>;

    pub struct WebState {
        pub namespace: String,
        pub kube_client: KubeClient,
        pub fleet: Arc<crate::multi_cluster::fleet::FleetService>,
        /// Present only when ZORVIA_LAB_MODE=1 and ZORVIA_API_KEY is set.
        pub lab_api_key: Option<String>,
        pub auth: crate::api::auth::SharedAuth,
        pub kryton: Option<crate::kryton::Client>,
        pub atlas: Option<crate::atlas::Client>,
        pub audit: SharedAuditTrail,
        rate_limiter: RateLimiterState,
    }

    impl WebState {
        pub async fn new(namespace: String, rate_limit_per_minute: u64) -> anyhow::Result<Self> {
            let auth = Arc::new(crate::api::auth::AuthState::from_env()?);
            let lab_api_key = auth.lab_api_key.clone();
            if crate::api::auth::lab_mode() {
                log::warn!("ZORVIA_LAB_MODE=1: lab credentials and shared API key are permitted");
            }
            let kube_client = KubeClient::new().await?;
            let fleet = Arc::new(
                crate::multi_cluster::fleet::FleetService::from_env(
                    kube_client.client(),
                    namespace.clone(),
                )
                .await?,
            );
            let kryton = crate::kryton::Client::from_env()?;
            if let Some(ref client) = kryton {
                log::info!(
                    "Kryton integration enabled: {} (project={:?})",
                    client.base_url(),
                    client.configured_project()
                );
            }
            let atlas = crate::atlas::Client::from_env()?;
            if let Some(ref client) = atlas {
                log::info!(
                    "Atlas integration enabled: {} (tenant={:?})",
                    client.base_url(),
                    client.configured_tenant_id()
                );
            }
            Ok(Self {
                namespace,
                kube_client,
                fleet,
                lab_api_key,
                auth,
                kryton,
                atlas,
                audit: Arc::new(RwLock::new(crate::audit_trail::AuditTrail::from_env())),
                rate_limiter: RateLimiterState::new(rate_limit_per_minute, 60),
            })
        }

        pub fn client(&self) -> KubeClient {
            self.kube_client.clone()
        }
    }

    pub type SharedState = Arc<RwLock<WebState>>;

    // ── Kubernetes name validation (RFC 1123 DNS label) ──────────

    /// Validate that a string is a valid Kubernetes name (RFC 1123 DNS label).
    /// Must be at most 63 characters, consist of lowercase alphanumeric characters
    /// or '-', and must start and end with an alphanumeric character.
    fn is_valid_k8s_name(s: &str) -> bool {
        if s.is_empty() || s.len() > 63 {
            return false;
        }
        let bytes = s.as_bytes();
        // Must start and end with alphanumeric
        if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
            return false;
        }
        if !bytes[bytes.len() - 1].is_ascii_lowercase() && !bytes[bytes.len() - 1].is_ascii_digit()
        {
            return false;
        }
        // All characters must be lowercase alphanumeric or '-'
        s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }

    /// Return a 400 error if a path parameter is not a valid Kubernetes name.
    fn validate_k8s_params(
        params: &[(&str, &str)],
    ) -> Option<(StatusCode, Json<serde_json::Value>)> {
        for (label, value) in params {
            if !is_valid_k8s_name(value) {
                return Some(err_json(
                    400,
                    "INVALID_PARAMETER",
                    &format!("'{}' is not a valid Kubernetes name", label),
                ));
            }
        }
        None
    }

    // ── Auth middleware ──────────────────────────────────────────

    /// Authentication + RBAC middleware: JWT Bearer, scoped API token, or
    /// lab-only shared API key. Inserts `AuthIdentity` and enforces
    /// `required_permission` for mutating routes.
    async fn auth_middleware(
        State(state): State<SharedState>,
        headers: HeaderMap,
        mut request: axum::extract::Request,
        next: middleware::Next,
    ) -> impl IntoResponse {
        let path = request.uri().path().to_string();
        let method = request.method().clone();

        if is_public_path(&path) || method == axum::http::Method::OPTIONS {
            return next.run(request).await.into_response();
        }

        let s = state.read().await;
        let auth = s.auth.clone();
        drop(s);

        let credential = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .or_else(|| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer ").map(|s| s.to_string()))
            });

        let Some(credential) = credential else {
            let (status, json) = err_json(401, "UNAUTHORIZED", "Invalid or missing credentials");
            return (status, json).into_response();
        };

        // The store lookup is synchronous (and can wait on a database): keep it off the
        // async workers.
        let checked = {
            let auth = auth.clone();
            tokio::task::spawn_blocking(move || auth.check_credential(&credential)).await
        };
        let identity = match checked {
            Ok(crate::api::auth::handlers::CredentialCheck::Valid(id)) => id,
            Ok(crate::api::auth::handlers::CredentialCheck::StoreUnavailable) | Err(_) => {
                // Not a 401: the session may be fine, and clients must not sign the user out.
                let (status, json) = err_json(
                    503,
                    "AUTH_STORE_UNAVAILABLE",
                    "The user database is unavailable; retry shortly",
                );
                let mut resp = (status, json).into_response();
                resp.headers_mut().insert(
                    header::RETRY_AFTER,
                    axum::http::HeaderValue::from_static("5"),
                );
                return resp;
            }
            Ok(crate::api::auth::handlers::CredentialCheck::Invalid) => {
                let (status, json) =
                    err_json(401, "UNAUTHORIZED", "Invalid or missing credentials");
                return (status, json).into_response();
            }
        };

        // Path under /api nest: strip /api prefix for permission map
        let api_path = path.strip_prefix("/api").unwrap_or(&path);
        if let Some(required) =
            crate::api::auth::permissions::required_permission(method.as_str(), api_path)
        {
            if !identity.has_permission(required) {
                let (status, json) = err_json(
                    403,
                    "FORBIDDEN",
                    &format!("Missing permission: {}", required.as_str()),
                );
                return (status, json).into_response();
            }
        }

        if let Some(allowed) = identity.namespaces.as_deref() {
            let default_ns = state.read().await.namespace.clone();
            if let Err(msg) = crate::api::auth::tenancy::check_namespace_access(
                allowed,
                &default_ns,
                api_path,
                request.uri().query(),
            ) {
                let (status, json) = err_json(403, "FORBIDDEN", &msg);
                return (status, json).into_response();
            }
        }

        request.extensions_mut().insert(identity);
        next.run(request).await.into_response()
    }

    /// Requires `users.admin` (Admin role). Applied via `route_layer` to
    /// `/v1/users*` and `/v1/api-tokens*`.
    async fn require_admin_middleware(
        request: axum::extract::Request,
        next: middleware::Next,
    ) -> impl IntoResponse {
        let allowed = request
            .extensions()
            .get::<crate::api::auth::AuthIdentity>()
            .map(|id| id.has_permission(crate::api::auth::ApiPermission::UsersAdmin))
            .unwrap_or(false);
        if !allowed {
            let (status, json) = err_json(403, "FORBIDDEN", "Admin role required");
            return (status, json).into_response();
        }
        next.run(request).await.into_response()
    }

    fn is_public_path(path: &str) -> bool {
        // SPA assets and client routes are public; only /api/* is gated (except auth/health/instance).
        if !path.starts_with("/api/") {
            return true;
        }
        matches!(
            path,
            "/api/v1/health"
                | "/api/health"
                | "/api/readyz"
                | "/api/metrics"
                | "/api/v1/auth/login"
                | "/api/v1/auth/providers"
                | "/api/v1/features"
                | "/api/v1/commercial/catalog"
                | "/api/v1/instance"
                | "/api/instance"
        ) || path.starts_with("/api/v1/auth/oidc/")
    }

    // ── Rate limiting middleware ──────────────────────────────────

    /// Middleware that enforces a global request rate limit.
    async fn rate_limit_middleware(
        State(state): State<SharedState>,
        request: axum::extract::Request,
        next: middleware::Next,
    ) -> impl IntoResponse {
        let path = request.uri().path().to_string();
        // Never rate-limit SPA assets or public health; only gate API traffic.
        if !path.starts_with("/api/")
            || path == "/api/v1/health"
            || path == "/api/health"
            || path == "/api/readyz"
            || path == "/api/metrics"
        {
            return next.run(request).await.into_response();
        }

        let s = state.read().await;
        if !s.rate_limiter.check_rate_limit() {
            let (status, json) =
                err_json(429, "RATE_LIMITED", "Too many requests. Please slow down.");
            return (status, json).into_response();
        }
        drop(s);
        next.run(request).await.into_response()
    }

    // ── Security headers middleware ─────────────────────────────

    /// Middleware that adds security headers to every response.
    async fn security_headers_middleware(
        request: axum::extract::Request,
        next: middleware::Next,
    ) -> impl IntoResponse {
        let mut response = next.run(request).await;
        let headers = response.headers_mut();
        headers.insert("x-content-type-options", "nosniff".parse().unwrap());
        headers.insert("x-frame-options", "DENY".parse().unwrap());
        headers.insert("cache-control", "no-store".parse().unwrap());
        headers.insert("x-xss-protection", "0".parse().unwrap());
        response
    }

    // ── CORS configuration ──────────────────────────────────────

    /// Build a CORS layer. If `ZORVIA_CORS_ORIGINS` is set (comma-separated
    /// list of origins), allow those origins. Otherwise default to same-origin
    /// only (no extra origins allowed).
    fn build_cors_layer() -> CorsLayer {
        let origins = std::env::var("ZORVIA_CORS_ORIGINS").ok();

        let allow_origin = match origins {
            Some(ref raw) if !raw.is_empty() => {
                let parsed: Vec<_> = raw
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if parsed.is_empty() {
                    AllowOrigin::default()
                } else {
                    AllowOrigin::list(parsed)
                }
            }
            _ => AllowOrigin::default(), // same-origin: no Access-Control-Allow-Origin header
        };

        CorsLayer::new()
            .allow_origin(allow_origin)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
            ])
            .allow_headers([
                header::CONTENT_TYPE,
                header::AUTHORIZATION,
                "x-api-key".parse().unwrap(),
            ])
    }

    // ── Router ──────────────────────────────────────────────────

    fn resolve_web_dir() -> std::path::PathBuf {
        if let Ok(dir) = std::env::var("ZORVIA_WEB_DIR") {
            return std::path::PathBuf::from(dir);
        }
        let candidates = [
            std::path::PathBuf::from("/usr/share/zorvia/web"),
            std::path::PathBuf::from("web/dist"),
            std::path::PathBuf::from("./web/dist"),
        ];
        for c in candidates {
            if c.join("index.html").exists() {
                return c;
            }
        }
        std::path::PathBuf::from("/usr/share/zorvia/web")
    }

    pub fn build_router(state: SharedState) -> Router {
        let web_dir = resolve_web_dir();
        log::info!("Serving SPA from {}", web_dir.display());
        let index = web_dir.join("index.html");
        let spa = ServeDir::new(web_dir).fallback(ServeFile::new(index));

        // Admin-only: role-gated via route_layer rather than each handler
        // self-checking, so a future admin route just needs to join this
        // group to be protected. Kept in its own Router since route_layer
        // applies to every route already registered on *that* router --
        // calling it inline in the big flat `api` chain below would wrongly
        // cover the auth/login routes registered earlier too.
        let admin_only = Router::new()
            .route("/v1/users", get(users_list).post(users_create))
            .route("/v1/users/{id}", delete(users_delete))
            .route("/v1/users/{id}/role", put(users_update_role))
            .route("/v1/users/{id}/enabled", put(users_set_enabled))
            .route("/v1/users/{id}/totp", delete(users_reset_totp))
            .route(
                "/v1/users/{id}/namespaces",
                get(users_get_namespaces).put(users_set_namespaces),
            )
            .route(
                "/v1/api-tokens",
                get(api_tokens_list).post(api_tokens_create),
            )
            .route(
                "/v1/api-tokens/{id}",
                delete(api_tokens_delete).post(api_tokens_revoke),
            )
            .route_layer(middleware::from_fn(require_admin_middleware));

        let api = Router::new()
            // Auth (Zorvia)
            .route("/v1/auth/login", post(auth_login))
            .route("/v1/auth/me", get(auth_me))
            .route("/v1/auth/providers", get(auth_providers))
            .route("/v1/auth/totp/setup", post(auth_totp_setup))
            .route("/v1/auth/totp/verify", post(auth_totp_verify))
            .route("/v1/auth/totp/disable", post(auth_totp_disable))
            .route("/v1/auth/password", post(auth_change_password))
            .route("/v1/auth/logout", post(auth_logout))
            .route("/v1/auth/oidc/callback", get(auth_oidc_callback))
            .route("/v1/auth/oidc/{id}", get(auth_oidc_login))
            .merge(admin_only)
            .route("/v1/instance", get(instance_handler))
            .route("/instance", get(instance_handler))
            // Images (create wizard)
            .route("/images", get(fabric_list_images))
            .route("/images/cloud", get(fabric_list_cloud_images))
            .route("/images/downloads", get(fabric_list_downloads))
            .route("/images/cloud/download", post(fabric_start_download))
            .route(
                "/images/from-vm/{vm_name}",
                post(fabric_create_image_from_vm),
            )
            .route("/images/convert/{id}", get(fabric_get_convert_job))
            // VMware -> KubeVirt imports (h2kvm Jobs)
            .route(
                "/vm-imports",
                get(list_imports_handler).post(create_import_handler),
            )
            .route("/vm-imports/preflight", post(preflight_import_handler))
            .route("/vm-imports/waves/{wave_id}", get(get_wave_handler))
            // Off-cluster backups: catalog, restore, recovery drills
            .route(
                "/backups/offcluster",
                get(list_offcluster_backups_handler).post(create_offcluster_backup_handler),
            )
            .route(
                "/backups/offcluster/{op_id}/restore",
                post(restore_offcluster_handler),
            )
            .route(
                "/backups/offcluster/{op_id}/drill",
                post(drill_offcluster_handler),
            )
            .route(
                "/backups/offcluster/{op_id}/verify-key",
                post(verify_key_offcluster_handler),
            )
            // Durable operations
            .route("/operations", get(list_operations_handler))
            .route("/operations/{id}", get(get_operation_handler))
            .route("/operations/{id}/cancel", post(cancel_operation_handler))
            // Fabric-compat VM API (unwrapped JSON)
            .route("/vms", get(fabric_list_vms).post(fabric_create_vm))
            .route("/vms/{name}", get(fabric_get_vm).delete(fabric_delete_vm))
            .route("/vms/{name}/start", post(fabric_start_vm))
            .route("/vms/{name}/stop", post(fabric_stop_vm))
            .route("/vms/{name}/restart", post(fabric_restart_vm))
            .route("/vms/{name}/pause", post(fabric_pause_vm))
            .route("/vms/{name}/resume", post(fabric_resume_vm))
            .route("/vms/{name}/hotplug/cpu", post(fabric_hotplug_cpu))
            .route("/vms/{name}/hotplug/memory", post(fabric_hotplug_memory))
            .route("/vms/{name}/hotplug/disk", post(fabric_hotplug_disk))
            .route(
                "/vms/{name}/hotplug/disk/{device_id}",
                delete(fabric_hotunplug_disk),
            )
            .route("/vms/{name}/hotplug/nic", post(fabric_hotplug_nic))
            .route(
                "/vms/{name}/hotplug/nic/{device_id}",
                delete(fabric_hotunplug_nic),
            )
            .route("/vms/{name}/drift", post(fabric_vm_drift))
            .route("/vms/{name}/plan", post(fabric_vm_change_plan))
            .route("/vms/{name}/disks", get(fabric_list_disks))
            .route(
                "/vms/{name}/disks/{disk_name}/resize",
                post(fabric_resize_disk),
            )
            .route("/vms/{name}/interfaces", get(fabric_list_interfaces))
            .route("/vms/{name}/migrate", post(fabric_migrate_vm))
            .route("/vms/{name}/migrations", get(fabric_list_vm_migrations))
            .route("/migrations/{id}", get(fabric_get_migration))
            .route("/migrations/{id}/cancel", post(fabric_cancel_migration))
            .route("/migrations/readiness", get(migration_readiness_handler))
            // Rook-Ceph distributed storage
            .route("/storage/rook/bootstrap", post(rook_bootstrap))
            .route(
                "/storage/rook/cluster",
                get(rook_cluster_status)
                    .post(rook_create_cluster)
                    .delete(rook_delete_cluster),
            )
            .route(
                "/storage/rook/pools",
                get(rook_list_pools).post(rook_create_pool),
            )
            .route("/storage/rook/pools/{name}", delete(rook_delete_pool))
            .route(
                "/storage/rook/filesystems",
                get(rook_list_filesystems).post(rook_create_filesystem),
            )
            .route(
                "/storage/rook/filesystems/{name}",
                delete(rook_delete_filesystem),
            )
            .route(
                "/storage/rook/objectstores",
                get(rook_list_object_stores).post(rook_create_object_store),
            )
            .route(
                "/storage/rook/objectstores/{name}",
                delete(rook_delete_object_store),
            )
            .route(
                "/storage/rook/storage-classes",
                post(rook_create_storage_class),
            )
            .route(
                "/storage/rook/volume-snapshot-classes",
                post(rook_create_volume_snapshot_class),
            )
            .route("/vms/{name}/metrics", get(fabric_vm_metrics))
            .route("/vms/{name}/guest-insight", get(fabric_guest_insight))
            .route("/vms/{name}/wait-ready", post(fabric_wait_guest_ready))
            .route("/vms/{name}/guest/agent", get(guest_agent_info))
            .route("/vms/{name}/guest/inventory/{kind}", get(guest_inventory))
            .route("/vms/{name}/logs", get(fabric_vm_logs))
            .route("/datavolumes/{name}/wait", post(fabric_wait_data_volume))
            .route("/readyz", get(fabric_readyz))
            .route("/metrics", get(fabric_prom_metrics))
            .route("/vms/{name}/port-forwards", post(fabric_add_port_forward))
            .route(
                "/vms/{name}/port-forwards/{host_port}",
                delete(fabric_remove_port_forward),
            )
            .route("/vms/{name}/cloud-init", post(fabric_cloud_init))
            .route(
                "/vms/{name}/tags",
                put(fabric_set_tags).post(fabric_add_tag),
            )
            .route("/vms/{name}/tags/{tag}", delete(fabric_remove_tag))
            .route("/vms/{name}/clone", post(fabric_clone_vm))
            .route(
                "/vms/{name}/boot",
                get(fabric_get_boot).post(fabric_set_boot),
            )
            .route(
                "/vms/{name}/display",
                get(fabric_get_display).post(fabric_set_display),
            )
            .route(
                "/vms/{name}/cpu-model",
                get(fabric_get_cpu_model).post(fabric_set_cpu_model),
            )
            .route(
                "/vms/{name}/watchdog",
                get(fabric_get_watchdog).post(fabric_set_watchdog),
            )
            .route(
                "/vms/{name}/serials",
                get(fabric_get_serials).post(fabric_add_serial),
            )
            .route(
                "/vms/{name}/firmware/status",
                get(fabric_get_firmware_status),
            )
            .route("/vms/{name}/firmware/uefi", post(fabric_enable_uefi))
            .route(
                "/vms/{name}/firmware/secureboot",
                post(fabric_enable_secureboot).delete(fabric_disable_secureboot),
            )
            .route("/vms/{name}/firmware/reset", post(fabric_reset_nvram))
            .route("/vms/{name}/cpu/affinity", get(fabric_get_cpu_affinity))
            .route("/system/cpu-models", get(fabric_list_cpu_models))
            .route(
                "/system/firmware/capabilities",
                get(fabric_firmware_capabilities),
            )
            .route(
                "/vms/{name}/snapshots",
                get(fabric_list_vm_snapshots).post(fabric_create_snapshot),
            )
            .route("/vms/{name}/snapshots/{id}", delete(fabric_delete_snapshot))
            .route(
                "/vms/{name}/snapshots/{id}/revert",
                post(fabric_revert_snapshot),
            )
            .route("/snapshots", get(fabric_list_snapshots))
            .route("/services/map", get(list_service_map_handler))
            .route("/audit/logs", get(list_audit_logs_handler))
            .route("/audit/stats", get(audit_stats_handler))
            .route("/audit/export", get(export_audit_logs_handler))
            .route("/audit/logs/export", get(export_audit_logs_handler))
            .route(
                "/backups",
                get(list_backups_handler).post(create_backup_handler),
            )
            .route("/backups/{id}", delete(delete_backup_handler))
            .route("/backups/restore", post(restore_backup_handler))
            .route("/backups/jobs", get(list_backup_jobs_handler))
            .route("/backups/jobs/{id}", get(get_backup_job_handler))
            .route("/v1/maintenance/plan", get(maintenance_plan_handler))
            .route("/v1/devices", get(devices_inventory_handler))
            .route("/v1/devices/preflight", post(devices_preflight_handler))
            .route(
                "/v1/devices/permitted",
                post(devices_permit_handler).delete(devices_unpermit_handler),
            )
            .route("/placement/rebalance", get(placement_rebalance_handler))
            .route("/placement/{vm}", get(placement_recommend_handler))
            .route(
                "/vms/{name}/ha",
                get(get_ha_policy_handler).put(set_ha_policy_handler),
            )
            .route("/vms/{name}/rescue", post(rescue_vm_handler))
            .route(
                "/vms/{name}/rescue/{job_name}",
                get(get_rescue_job_handler).delete(delete_rescue_job_handler),
            )
            .route(
                "/backups/policies",
                get(list_backup_policies_handler).post(create_backup_policy_handler),
            )
            .route(
                "/backups/policies/{name}",
                delete(delete_backup_policy_handler),
            )
            .route(
                "/backups/policies/{name}/enable",
                post(enable_backup_policy_handler),
            )
            .route(
                "/backups/policies/{name}/disable",
                post(disable_backup_policy_handler),
            )
            .route("/storage/volumes", get(list_storage_volumes_handler))
            .route(
                "/network-policies",
                get(list_network_policies_handler).post(create_network_policy_handler),
            )
            .route(
                "/network-policies/{name}",
                delete(delete_network_policy_handler),
            )
            .route("/system/compliance", get(compliance_dashboard_handler))
            .route(
                "/system/compliance/scan",
                post(compliance_dashboard_handler),
            )
            .route("/system/security", get(security_dashboard_handler))
            .route("/capacity/overview", get(capacity_overview_handler))
            .route("/capacity/fit", get(capacity_fit_handler))
            .route("/zones", get(list_zones_handler))
            .route("/analytics/top-vms", get(analytics_top_vms_handler))
            .route(
                "/optimization/recommendations",
                get(resource_optimizer_handler),
            )
            .route("/templates", get(list_templates_handler))
            .route("/templates/{name}", get(get_template_handler))
            .route("/templates/{name}/deploy", post(deploy_template_handler))
            .route(
                "/schedules/power",
                get(list_power_schedules_handler).post(create_power_schedule_handler),
            )
            .route(
                "/schedules/power/{name}",
                delete(delete_power_schedule_handler),
            )
            .route(
                "/schedules/power/{name}/enable",
                post(enable_power_schedule_handler),
            )
            .route(
                "/schedules/power/{name}/disable",
                post(disable_power_schedule_handler),
            )
            .route(
                "/webhooks",
                get(list_webhooks_handler).post(create_webhook_handler),
            )
            .route("/webhooks/{id}", delete(delete_webhook_handler))
            .route("/webhooks/{id}/test", post(test_webhook_handler))
            .route("/alerts", get(list_alerts_handler))
            .route(
                "/alerts/rules",
                get(list_alert_rules_handler).post(create_alert_rule_handler),
            )
            .route("/alerts/rules/{id}", delete(delete_alert_rule_handler))
            .route("/alerts/{id}/resolve", post(resolve_alert_handler))
            .route("/alerts/{id}/silence", post(silence_alert_handler))
            .route(
                "/warm-pools",
                get(list_warm_pools_handler).post(create_warm_pool_handler),
            )
            .route("/warm-pools/{name}", delete(delete_warm_pool_handler))
            .route("/warm-pools/{name}/claim", post(claim_warm_pool_handler))
            .route("/events", get(fabric_list_events))
            .route("/events/stream", get(fabric_events_stream))
            .route("/capabilities", get(fabric_capabilities))
            .route("/health", get(fabric_health))
            .route("/dashboard/overview", get(fabric_overview))
            // Native Zorvia v1 API
            .route("/v1/vms", get(list_vms_handler))
            .route("/v1/vms/{ns}/{name}", get(get_vm_handler))
            .route("/v1/vms/{ns}/{name}", delete(delete_vm_handler))
            .route("/v1/vms/{ns}/{name}/start", post(start_vm_handler))
            .route("/v1/vms/{ns}/{name}/stop", post(stop_vm_handler))
            .route("/v1/vms/{ns}/{name}/restart", post(restart_vm_handler))
            .route("/v1/snapshots", get(list_snapshots_handler))
            .route("/v1/snapshots/{ns}/{vm}", get(list_vm_snapshots_handler))
            .route(
                "/v1/snapshots/{ns}/{name}/delete",
                post(delete_snapshot_handler),
            )
            .route("/v1/events", get(list_events_handler))
            .route("/v1/events/recent", get(recent_events_handler))
            .route("/v1/dashboard/overview", get(dashboard_overview_handler))
            .route(
                "/v1/quotas",
                get(list_quotas_handler).post(create_quota_handler),
            )
            .route("/v1/quotas/{ns}/{name}", delete(delete_quota_handler))
            .route("/v1/pods", get(list_pods_handler))
            .route("/v1/pods/capabilities", get(pod_capabilities_handler))
            .route("/v1/pods/{ns}/{name}", delete(delete_pod_handler))
            .route("/v1/pods/{ns}/{name}/restart", post(restart_pod_handler))
            .route("/v1/pods/{ns}/{name}/events", get(pod_events_handler))
            .route("/v1/pods/{ns}/{name}/yaml", get(pod_yaml_handler))
            .route("/v1/namespaces", get(list_namespaces_handler))
            // Kryton Windows control plane (server-side token; Zorvia auth at edge)
            .route("/v1/kryton/status", get(kryton_status))
            .route("/v1/kryton/capabilities", get(kryton_capabilities))
            .route("/v1/kryton/doctor", get(kryton_doctor))
            .route("/v1/kryton/images", get(kryton_images))
            .route("/v1/kryton/summary", get(kryton_summary))
            .route(
                "/v1/kryton/machines",
                get(kryton_list_machines).post(kryton_create_machine),
            )
            .route(
                "/v1/kryton/machines/{id}",
                get(kryton_get_machine).delete(kryton_delete_machine),
            )
            .route("/v1/kryton/machines/{id}/start", post(kryton_start_machine))
            .route("/v1/kryton/machines/{id}/stop", post(kryton_stop_machine))
            .route(
                "/v1/kryton/machines/{id}/snapshot",
                post(kryton_snapshot_machine),
            )
            .route(
                "/v1/kryton/machines/{id}/snapshots",
                get(kryton_list_snapshots),
            )
            .route(
                "/v1/kryton/machines/{id}/snapshots/{sid}/restore",
                post(kryton_restore_snapshot),
            )
            .route(
                "/v1/kryton/machines/{id}/snapshots/{sid}",
                delete(kryton_delete_snapshot),
            )
            .route(
                "/v1/kryton/golden",
                get(kryton_list_golden).post(kryton_start_golden),
            )
            .route("/v1/kryton/golden/{id}", get(kryton_get_golden))
            .route(
                "/v1/kryton/golden/{id}/passport",
                get(kryton_golden_passport),
            )
            .route(
                "/v1/kryton/golden/{id}/bootstrap",
                post(kryton_bootstrap_golden),
            )
            // Atlas storage control plane (server-side token; Zorvia auth at edge)
            .route("/v1/atlas/status", get(atlas_status))
            .route(
                "/v1/atlas/backends",
                get(atlas_list_backends).post(atlas_create_backend),
            )
            .route("/v1/atlas/backends/summary", get(atlas_backends_summary))
            .route("/v1/atlas/clusters", get(atlas_list_clusters))
            .route("/v1/atlas/clusters/{id}/health", get(atlas_cluster_health))
            .route("/v1/atlas/pools", get(atlas_list_pools))
            .route("/v1/atlas/ceph/status", get(atlas_ceph_status))
            .route("/v1/atlas/ceph/df", get(atlas_ceph_df))
            .route("/v1/atlas/storage-classes", get(atlas_list_storage_classes))
            .route(
                "/v1/atlas/volumes",
                get(atlas_list_volumes).post(atlas_create_volume),
            )
            .route("/v1/atlas/volumes/{id}", delete(atlas_delete_volume))
            .route("/v1/atlas/volumes/{id}/expand", post(atlas_expand_volume))
            .route("/v1/atlas/jobs", get(atlas_list_jobs))
            .route("/v1/atlas/jobs/{id}", get(atlas_get_job))
            .route("/v1/atlas/jobs/{id}/cancel", post(atlas_cancel_job))
            .route("/v1/atlas/backends/{id}", delete(atlas_delete_backend))
            .route(
                "/v1/atlas/backends/{id}/discover",
                post(atlas_discover_backend),
            )
            .route("/v1/atlas/backends/{id}/cordon", post(atlas_cordon_backend))
            .route(
                "/v1/atlas/backends/{id}/uncordon",
                post(atlas_uncordon_backend),
            )
            .route(
                "/v1/atlas/maintenance",
                get(atlas_get_maintenance).post(atlas_set_maintenance),
            )
            .route("/v1/atlas/maintenance/orphans", get(atlas_list_orphans))
            .route("/v1/atlas/upgrade/preflight", get(atlas_upgrade_preflight))
            .route("/v1/atlas/osds", get(atlas_list_osds))
            .route("/v1/atlas/osds/{id}/out", post(atlas_osd_out))
            .route("/v1/atlas/osds/{id}/in", post(atlas_osd_in))
            .route("/v1/atlas/osds/{id}/reweight", post(atlas_osd_reweight))
            .route(
                "/v1/atlas/rbd-images",
                get(atlas_list_rbd_images).post(atlas_create_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}",
                delete(atlas_delete_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/clone",
                post(atlas_clone_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/resize",
                post(atlas_resize_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/migrate",
                post(atlas_migrate_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/flatten",
                post(atlas_flatten_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/qos",
                post(atlas_qos_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/snapshots",
                get(atlas_list_rbd_snapshots).post(atlas_create_rbd_snapshot),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/rollback",
                post(atlas_rollback_rbd_image),
            )
            .route(
                "/v1/atlas/rbd-images/{pool}/{image}/snapshots/{snap}",
                delete(atlas_delete_rbd_snapshot),
            )
            .route("/v1/atlas/rbd-usage/refresh", post(atlas_refresh_rbd_usage))
            .route(
                "/v1/atlas/buckets",
                get(atlas_list_buckets).post(atlas_create_bucket),
            )
            .route(
                "/v1/atlas/buckets/{id}",
                get(atlas_get_bucket).delete(atlas_delete_bucket),
            )
            .route("/v1/atlas/buckets/{id}/stats", get(atlas_bucket_stats))
            .route(
                "/v1/atlas/buckets/{id}/objects",
                get(atlas_list_bucket_objects).delete(atlas_delete_bucket_object),
            )
            .route(
                "/v1/atlas/buckets/{id}/objects/upload-url",
                post(atlas_bucket_object_upload_url),
            )
            .route(
                "/v1/atlas/buckets/{id}/objects/download-url",
                get(atlas_bucket_object_download_url),
            )
            .route(
                "/v1/atlas/buckets/{id}/objects/prune",
                post(atlas_prune_bucket_objects),
            )
            .route("/v1/atlas/backup-jobs", post(atlas_create_backup))
            .route("/v1/atlas/restore-jobs", post(atlas_create_restore))
            .route("/v1/atlas/backups", get(atlas_list_backups))
            .route(
                "/v1/atlas/backups/{id}",
                get(atlas_get_backup).delete(atlas_delete_backup),
            )
            .route(
                "/v1/atlas/backups/{id}/download",
                get(atlas_download_backup),
            )
            .route(
                "/v1/atlas/dr/peers",
                get(atlas_list_dr_peers).post(atlas_register_dr_peer),
            )
            .route("/v1/atlas/dr/peers/{id}", delete(atlas_delete_dr_peer))
            .route("/v1/atlas/dr/mirrors", get(atlas_list_dr_mirrors))
            .route("/v1/atlas/dr/status", get(atlas_dr_status))
            .route("/v1/atlas/dr/preflight", get(atlas_dr_preflight))
            .route(
                "/v1/atlas/dr/mirrors/{id}/promote",
                post(atlas_promote_mirror),
            )
            .route(
                "/v1/atlas/dr/mirrors/{id}/demote",
                post(atlas_demote_mirror),
            )
            .route("/v1/atlas/dr/mirrors/{id}/rpo", post(atlas_set_mirror_rpo))
            .route("/v1/atlas/dr/failover", post(atlas_dr_failover))
            .route(
                "/v1/atlas/volumes/{id}/mirror",
                post(atlas_enable_mirror).delete(atlas_disable_mirror),
            )
            .route("/v1/atlas/ai/advisor", post(atlas_ai_advisor))
            .route("/v1/atlas/ai/anomalies", get(atlas_ai_anomalies))
            .route("/v1/atlas/ai/incidents", get(atlas_ai_incidents))
            .route("/v1/atlas/ai/what-if", post(atlas_ai_what_if))
            .route("/v1/atlas/metrics/summary", get(atlas_metrics_summary))
            .route("/v1/atlas/metrics/ceph", get(atlas_metrics_ceph))
            .route("/v1/atlas/metrics/history", get(atlas_metrics_history))
            .route("/v1/atlas/metrics/forecast", get(atlas_metrics_forecast))
            .route("/v1/atlas/alerts", get(atlas_list_alerts))
            .route("/v1/atlas/alerts/evaluate", post(atlas_evaluate_alerts))
            .route("/v1/atlas/alerts/{id}/ack", post(atlas_ack_alert))
            .route("/v1/atlas/alerts/{id}/silence", post(atlas_silence_alert))
            .route("/v1/atlas/alerts/{id}/resolve", post(atlas_resolve_alert))
            .route("/v1/atlas/audit", get(atlas_list_audit))
            .route("/v1/atlas/audit.csv", get(atlas_export_audit_csv))
            .route("/v1/atlas/chargeback", get(atlas_chargeback))
            .route("/v1/atlas/policy-drift", get(atlas_policy_drift))
            .route("/v1/atlas/events", get(atlas_list_events))
            .route("/v1/atlas/tenants", get(atlas_list_tenants))
            .route("/v1/atlas/policies", get(atlas_list_policies))
            .route(
                "/v1/atlas/tenants/{id}/policies",
                get(atlas_list_tenant_policies),
            )
            .route(
                "/v1/atlas/tenants/{id}/policies/{intent}",
                put(atlas_put_tenant_policy).delete(atlas_delete_tenant_policy),
            )
            .route(
                "/v1/atlas/tenants/{id}/quota",
                get(atlas_get_tenant_quota).put(atlas_put_tenant_quota),
            )
            .route(
                "/v1/atlas/volumes/{id}/schedule",
                post(atlas_create_schedule),
            )
            .route("/v1/atlas/schedules", get(atlas_list_schedules))
            .route("/v1/atlas/schedules/{id}", delete(atlas_delete_schedule))
            .route(
                "/v1/atlas/volumes/{id}/labels",
                get(atlas_get_volume_labels).put(atlas_put_volume_labels),
            )
            .route(
                "/v1/atlas/volumes/{id}/bindings",
                get(atlas_list_volume_bindings),
            )
            .route(
                "/v1/atlas/databridge/sources",
                get(atlas_db_list_sources).post(atlas_db_create_source),
            )
            .route(
                "/v1/atlas/databridge/sources/{id}",
                get(atlas_db_get_source).delete(atlas_db_delete_source),
            )
            .route(
                "/v1/atlas/databridge/sources/{id}/discover",
                post(atlas_db_discover_source),
            )
            .route(
                "/v1/atlas/databridge/plans",
                get(atlas_db_list_plans).post(atlas_db_create_plan),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}",
                get(atlas_db_get_plan).delete(atlas_db_delete_plan),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/assess",
                post(atlas_db_assess_plan),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/provision",
                post(atlas_db_provision_edge),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/full-load",
                post(atlas_db_full_load),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/cdc/start",
                post(atlas_db_cdc_start),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/cdc/stop",
                post(atlas_db_cdc_stop),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/cdc/restart",
                post(atlas_db_cdc_restart),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/validate",
                post(atlas_db_validate),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/cutover",
                post(atlas_db_cutover),
            )
            .route(
                "/v1/atlas/databridge/plans/{id}/rollback",
                post(atlas_db_rollback),
            )
            .route(
                "/v1/atlas/databridge/edge-clusters",
                get(atlas_db_list_edge_clusters),
            )
            .route(
                "/v1/atlas/databridge/edge-clusters/{id}",
                get(atlas_db_get_edge_cluster).delete(atlas_db_delete_edge_cluster),
            )
            .route(
                "/v1/atlas/databridge/cdc-streams",
                get(atlas_db_list_cdc_streams),
            )
            .route(
                "/v1/atlas/databridge/cdc-streams/{id}",
                get(atlas_db_get_cdc_stream),
            )
            .route(
                "/v1/atlas/databridge/object",
                get(atlas_db_list_object_migrations).post(atlas_db_create_object_migration),
            )
            .route(
                "/v1/atlas/databridge/object/{id}",
                get(atlas_db_get_object_migration).delete(atlas_db_delete_object_migration),
            )
            .route(
                "/v1/atlas/databridge/object/{id}/start",
                post(atlas_db_start_object_migration),
            )
            .route(
                "/v1/atlas/databridge/validations",
                get(atlas_db_list_validations),
            )
            .route("/v1/atlas/databridge/cutovers", get(atlas_db_list_cutovers))
            .route("/v1/health", get(health_handler))
            .route("/v1/features", get(features_registry_handler))
            .route("/v1/adopt/report", get(adopt_report_handler))
            .route(
                "/v1/enterprise/s3-backup/plan",
                post(enterprise_s3_backup_plan),
            )
            .route(
                "/v1/enterprise/transiva/plan",
                post(enterprise_transiva_plan),
            )
            .route(
                "/v1/enterprise/golden-pipeline/plan",
                post(enterprise_golden_pipeline_plan),
            )
            .route(
                "/v1/enterprise/golden-pipeline/run",
                post(enterprise_golden_pipeline_run),
            )
            .route(
                "/v1/enterprise/cross-cluster-dr/plan",
                post(enterprise_cross_cluster_dr_plan),
            )
            .route(
                "/v1/enterprise/placement/gpu-numa",
                post(enterprise_gpu_numa_plan),
            )
            .route("/v1/enterprise/fleet", get(enterprise_fleet_inventory))
            // Commercial: catalog is public; the rest is org-scoped in the
            // store, and /admin/ routes require users.admin (permissions.rs).
            .route("/v1/commercial/catalog", get(commercial_catalog_handler))
            .route(
                "/v1/commercial/quote-requests",
                get(commercial_quote_requests_list).post(commercial_quote_request_create),
            )
            .route("/v1/commercial/contracts", get(commercial_contracts_list))
            .route(
                "/v1/commercial/contracts/{id}",
                get(commercial_contract_get),
            )
            .route(
                "/v1/commercial/contracts/{id}/history",
                get(commercial_contract_history),
            )
            .route(
                "/v1/commercial/contracts/{id}/accept",
                post(commercial_contract_accept),
            )
            .route("/v1/commercial/coverage", get(commercial_coverage))
            .route(
                "/v1/commercial/admin/orgs",
                get(commercial_admin_orgs_list).post(commercial_admin_org_create),
            )
            .route(
                "/v1/commercial/admin/orgs/{id}/members",
                post(commercial_admin_member_add),
            )
            .route(
                "/v1/commercial/admin/orgs/{id}/members/{username}",
                delete(commercial_admin_member_remove),
            )
            .route(
                "/v1/commercial/admin/quote-requests/{id}/quote",
                post(commercial_admin_quote_generate),
            )
            .route(
                "/v1/commercial/admin/contracts/import",
                post(commercial_admin_import),
            )
            .route(
                "/v1/commercial/admin/contracts/{id}/entitlement",
                put(commercial_admin_entitlement_put),
            )
            .route(
                "/v1/commercial/admin/contracts/{id}/transition",
                post(commercial_admin_transition),
            )
            .route(
                "/v1/commercial/admin/contracts/{id}/payment-status",
                post(commercial_admin_payment_status),
            )
            // Support cases and diagnostics (org-scoped in the store; see
            // permissions.rs for the cluster.admin / users.admin gates).
            .route(
                "/v1/support/cases",
                get(support_cases_list).post(support_case_create),
            )
            .route("/v1/support/cases/{id}", get(support_case_get))
            .route(
                "/v1/support/cases/{id}/messages",
                post(support_case_message),
            )
            .route("/v1/support/cases/{id}/status", post(support_case_status))
            .route(
                "/v1/support/cases/{id}/escalate",
                post(support_case_escalate),
            )
            .route(
                "/v1/support/cases/{id}/attachments/{aid}",
                get(support_attachment_download),
            )
            .route(
                "/v1/support/cases/{id}/diagnostics/upload",
                post(support_diagnostics_upload),
            )
            .route("/v1/support/diagnostics", post(support_diagnostics))
            .route(
                "/v1/support/admin/cases/{id}/assign",
                post(support_case_assign),
            )
            // Service engagements and managed-operations records. Records
            // only; /admin/ routes are the service desk (users.admin).
            .route("/v1/services/engagements", get(engagements_list))
            .route("/v1/services/engagements/{id}", get(engagement_get))
            .route(
                "/v1/services/engagements/{id}/milestones/{key}/evidence",
                post(engagement_evidence_add),
            )
            .route(
                "/v1/services/engagements/{id}/accept",
                post(engagement_accept),
            )
            .route("/v1/services/admin/engagements", post(engagement_create))
            .route(
                "/v1/services/admin/engagements/{id}/milestones/{key}",
                post(engagement_milestone_update),
            )
            .route(
                "/v1/services/admin/engagements/{id}/request-acceptance",
                post(engagement_request_acceptance),
            )
            .route(
                "/v1/services/admin/engagements/{id}/cancel",
                post(engagement_cancel),
            )
            .route(
                "/v1/managed/enrollments",
                get(managed_enrollments_list).post(managed_enroll),
            )
            .route("/v1/managed/enrollments/{id}", get(managed_enrollment_get))
            .route(
                "/v1/managed/enrollments/{id}/remote-operation",
                post(managed_remote_operation),
            )
            .route(
                "/v1/managed/enrollments/{id}/withdraw",
                post(managed_withdraw),
            )
            .route(
                "/v1/managed/enrollments/{id}/evidence",
                post(managed_evidence_add),
            )
            .route(
                "/v1/managed/enrollments/{id}/tasks/{tid}/approve",
                post(managed_task_approve),
            )
            .route(
                "/v1/managed/enrollments/{id}/policies/{pid}/authorize",
                post(managed_policy_authorize),
            )
            .route(
                "/v1/managed/enrollments/{id}/policies/{pid}/revoke",
                post(managed_policy_revoke),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/owner",
                post(managed_owner_set),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/tasks",
                post(managed_task_propose),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/tasks/{tid}/status",
                post(managed_task_status),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/incidents",
                post(managed_incident_open),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/incidents/{iid}",
                post(managed_incident_update),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/reports",
                post(managed_report_add),
            )
            .route(
                "/v1/managed/admin/enrollments/{id}/policies",
                post(managed_policy_create),
            )
            // Capacity reconciliation and invoice records (no payments).
            .route("/v1/billing/invoices", get(billing_invoices_list))
            .route("/v1/billing/invoices/{id}", get(billing_invoice_get))
            .route(
                "/v1/billing/invoices/{id}/document",
                get(billing_invoice_document),
            )
            .route(
                "/v1/billing/capacity/reconcile",
                get(billing_capacity_reconcile),
            )
            .route(
                "/v1/billing/capacity/observe",
                post(billing_capacity_observe),
            )
            .route(
                "/v1/billing/admin/capacity/observations",
                get(billing_admin_observations_list).post(billing_admin_observation_record),
            )
            .route(
                "/v1/billing/admin/invoices",
                post(billing_admin_invoice_create),
            )
            .route(
                "/v1/billing/admin/invoices/{id}/issue",
                post(billing_admin_invoice_issue),
            )
            .route(
                "/v1/billing/admin/invoices/{id}/mark-paid",
                post(billing_admin_invoice_paid),
            )
            .route(
                "/v1/billing/admin/invoices/{id}/void",
                post(billing_admin_invoice_void),
            )
            .fallback(fabric_not_implemented)
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                std::time::Duration::from_secs(30),
            ))
            .with_state(state.clone());

        Router::new()
            .route(
                "/dashboard",
                get(|| async { axum::response::Redirect::temporary("/app") }),
            )
            .route("/ws/console/{name}", get(ws_console))
            .route("/ws/vnc/{name}", get(ws_vnc))
            .route("/ws/ssh/{name}", get(ws_ssh))
            .route("/ws/pods/{ns}/{name}/logs", get(ws_pod_logs))
            .route("/ws/pods/{ns}/{name}/exec", get(ws_pod_exec))
            .nest("/api", api)
            .fallback_service(spa)
            .layer(middleware::from_fn(security_headers_middleware))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                rate_limit_middleware,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .layer(build_cors_layer())
            .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
            .with_state(state)
    }

    pub async fn start_server(
        host: &str,
        port: u16,
        namespace: String,
        tls_config: Option<TlsConfig>,
        rate_limit_per_minute: u64,
    ) -> anyhow::Result<()> {
        // Validate TLS config early if provided
        if let Some(ref tls) = tls_config {
            tls.validate()?;
        }

        // Initialize kube client at startup instead of lazily per-request
        let state = Arc::new(RwLock::new(
            WebState::new(namespace, rate_limit_per_minute).await?,
        ));
        spawn_leader_election();
        spawn_operations_reconciler(state.clone());
        spawn_backup_scheduler_loop(state.clone());
        spawn_power_schedule_loop(state.clone());
        spawn_alert_evaluation_loop(state.clone());
        spawn_warm_pool_loop(state.clone());
        let app = build_router(state);
        let addr = format!("{}:{}", host, port);

        if let Some(tls) = tls_config {
            log::info!("Starting HTTPS server on {}", addr);

            // Install the ring crypto provider (already used by kube-client)
            let _ = rustls::crypto::ring::default_provider().install_default();

            let rustls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path)
                    .await?;
            if let Some(every) = crate::api::tls_reload::reload_interval() {
                tokio::spawn(crate::api::tls_reload::watch(
                    rustls_config.clone(),
                    tls.cert_path.clone(),
                    tls.key_path.clone(),
                    every,
                ));
            }
            let addr: std::net::SocketAddr = addr.parse()?;
            axum_server::bind_rustls(addr, rustls_config)
                .serve(app.into_make_service())
                .await?;
        } else {
            log::info!("Starting HTTP server on {}", addr);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }

        Ok(())
    }

    async fn auth_shared(state: &SharedState) -> crate::api::auth::SharedAuth {
        state.read().await.auth.clone()
    }

    async fn auth_login(
        State(state): State<SharedState>,
        Json(body): Json<crate::api::auth::handlers::LoginRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        let username = body.username.clone();
        let response = crate::api::auth::login_handler(axum::extract::State(auth), Json(body))
            .await
            .into_response();
        let success = response.status().is_success();

        // Real login attempts, feeding the SecurityDashboard's real
        // failed-login widget -- built directly rather than via
        // record_audit(), which derives its `user` from an already-issued
        // Bearer token that doesn't exist yet at login time.
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let audit = s.audit.clone();
        drop(s);
        audit.write().await.record(crate::audit_trail::AuditEntry {
            id: crate::utils::generate_id("audit", &username),
            timestamp: chrono::Utc::now(),
            user: username,
            action: crate::audit_trail::AuditAction::Login,
            resource_type: "auth".to_string(),
            resource_name: "login".to_string(),
            namespace,
            details: serde_json::Value::Null,
            ip_address: String::new(),
            success,
            severity: crate::audit_trail::AuditSeverity::Medium,
        });

        response
    }

    async fn auth_me(State(state): State<SharedState>, headers: HeaderMap) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::me_handler(axum::extract::State(auth), headers).await
    }

    async fn auth_providers(State(state): State<SharedState>) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::providers_handler(axum::extract::State(auth)).await
    }

    async fn auth_totp_setup(
        State(state): State<SharedState>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::totp_setup_handler(axum::extract::State(auth), headers, body).await
    }

    async fn auth_change_password(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Json(body): Json<crate::api::auth::ChangePasswordRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::change_password_handler(axum::extract::State(auth), headers, Json(body))
            .await
    }

    async fn auth_logout(
        State(state): State<SharedState>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::logout_handler(axum::extract::State(auth), headers).await
    }

    async fn auth_totp_verify(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Json(body): Json<crate::api::auth::handlers::TotpVerifyRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::totp_verify_handler(axum::extract::State(auth), headers, Json(body)).await
    }

    async fn auth_totp_disable(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Json(body): Json<crate::api::auth::TotpDisableRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::totp_disable_handler(axum::extract::State(auth), headers, Json(body))
            .await
    }

    async fn api_tokens_list(
        State(state): State<SharedState>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::list_api_tokens_handler(axum::extract::State(auth), headers).await
    }

    async fn api_tokens_create(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Json(body): Json<crate::api::auth::handlers::CreateApiTokenRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::create_api_token_handler(axum::extract::State(auth), headers, Json(body))
            .await
    }

    async fn api_tokens_revoke(
        State(state): State<SharedState>,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::revoke_api_token_handler(axum::extract::State(auth), Path(id)).await
    }

    async fn api_tokens_delete(
        State(state): State<SharedState>,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::delete_api_token_handler(axum::extract::State(auth), Path(id)).await
    }

    // ── Admin-only user management ──────────────────────────────────────
    // Protected by require_admin_middleware (users.admin) and auth_middleware RBAC.

    async fn auth_oidc_login(
        State(state): State<SharedState>,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::oidc_login_handler(axum::extract::State(auth), Path(id)).await
    }

    async fn auth_oidc_callback(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Query(q): Query<crate::api::auth::handlers::OidcCallbackQuery>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::oidc_callback_handler(axum::extract::State(auth), headers, Query(q)).await
    }

    // ── Admin-only user management ──────────────────────────────────────
    // Protected by require_admin_middleware (users.admin) and auth_middleware RBAC.

    async fn users_list(State(state): State<SharedState>, headers: HeaderMap) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::list_users_handler(axum::extract::State(auth), headers).await
    }

    async fn users_create(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Json(body): Json<crate::api::auth::handlers::CreateUserRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::create_user_handler(
            axum::extract::State(auth),
            headers,
            Json(body),
        )
        .await
    }

    async fn users_delete(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::delete_user_handler(
            axum::extract::State(auth),
            headers,
            Path(id),
        )
        .await
    }

    async fn users_update_role(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(id): Path<String>,
        Json(body): Json<crate::api::auth::handlers::UpdateRoleRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::update_role_handler(
            axum::extract::State(auth),
            headers,
            Path(id),
            Json(body),
        )
        .await
    }

    async fn users_set_namespaces(
        State(state): State<SharedState>,
        Path(id): Path<String>,
        Json(body): Json<crate::api::auth::handlers::SetNamespacesRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::set_namespaces_handler(
            axum::extract::State(auth),
            Path(id),
            Json(body),
        )
        .await
    }

    async fn users_reset_totp(
        State(state): State<SharedState>,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::reset_totp_handler(axum::extract::State(auth), Path(id)).await
    }

    async fn users_get_namespaces(
        State(state): State<SharedState>,
        Path(id): Path<String>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::get_namespaces_handler(axum::extract::State(auth), Path(id))
            .await
    }

    async fn users_set_enabled(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(id): Path<String>,
        Json(body): Json<crate::api::auth::handlers::SetEnabledRequest>,
    ) -> impl IntoResponse {
        let auth = auth_shared(&state).await;
        crate::api::auth::handlers::set_enabled_handler(
            axum::extract::State(auth),
            headers,
            Path(id),
            Json(body),
        )
        .await
    }

    async fn instance_handler() -> impl IntoResponse {
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| "zorvia".into());
        Json(serde_json::json!({
            "product": "Zorvia",
            "product_id": "zorvia",
            "version": env!("CARGO_PKG_VERSION"),
            "hostname": hostname,
            "deploy_mode": "kubernetes",
            "deploy_label": "Kubernetes · NodePort 30152",
            "kubernetes": true,
            "kubernetes_namespace": "zorvia-system",
            "listen": ":30152",
        }))
    }

    async fn fabric_health() -> impl IntoResponse {
        Json(serde_json::json!({
            "status": "healthy",
            "service": "zorvia-api",
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }

    async fn fabric_readyz(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        match s.kube_client.list_vms(&s.namespace).await {
            Ok(_) => (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ready", "kube": true})),
            )
                .into_response(),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "not-ready",
                    "kube": false,
                    "error": sanitize_error(&e),
                })),
            )
                .into_response(),
        }
    }

    async fn fabric_prom_metrics() -> impl IntoResponse {
        let body = format!(
            "# HELP zorvia_up 1 if the API process is running\n# TYPE zorvia_up gauge\nzorvia_up 1\n# HELP zorvia_build_info Build version\n# TYPE zorvia_build_info gauge\nzorvia_build_info{{version=\"{}\"}} 1\n",
            env!("CARGO_PKG_VERSION")
        );
        (
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            body,
        )
    }

    async fn fabric_not_implemented() -> impl IntoResponse {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "success": false,
                "status": 501,
                "error": {
                    "code": "NOT_IMPLEMENTED",
                    "message": "This API is not available on Zorvia yet"
                },
                "data": null
            })),
        )
    }

    fn fabric_map_state(status: &str) -> &'static str {
        match status.to_lowercase().as_str() {
            s if s.contains("run") => "running",
            s if s.contains("stop") => "stopped",
            s if s.contains("paus") => "paused",
            s if s.contains("start") || s.contains("provision") || s.contains("wait") => "starting",
            s if s.contains("fail") || s.contains("error") => "failed",
            _ => "unknown",
        }
    }

    fn fabric_vm_json(info: &VmInfo) -> serde_json::Value {
        let cpus = info
            .cpu
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let mem_s = info.memory.trim().to_lowercase();
        let memory = if let Some(n) = mem_s.strip_suffix("gi") {
            (n.trim().parse::<f64>().unwrap_or(1.0) * 1024.0) as u64
        } else if let Some(n) = mem_s.strip_suffix('g') {
            (n.trim().parse::<f64>().unwrap_or(1.0) * 1024.0) as u64
        } else if let Some(n) = mem_s.strip_suffix("mi") {
            n.trim().parse().unwrap_or(1024)
        } else {
            1024
        };
        let ip = if info.ip.is_empty() || info.ip == "N/A" {
            serde_json::Value::Null
        } else {
            serde_json::json!(info.ip)
        };
        serde_json::json!({
            "name": info.name,
            "state": fabric_map_state(&info.status),
            "cpus": cpus,
            "memory": memory,
            "image": info.disk,
            "ip": ip,
            "tags": info.tags,
        })
    }

    async fn fabric_list_vms(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        match client.list_vms(&namespace).await {
            Ok(vms) => {
                let mut out = Vec::new();
                for vm in &vms {
                    let name = vm.metadata.name.clone().unwrap_or_default();
                    let ip = client.get_vm_ip(&namespace, &name).await.unwrap_or(None);
                    out.push(fabric_vm_json(&VmInfo::from_vm_with_ip(vm, ip)));
                }
                Json(out).into_response()
            }
            Err(e) => {
                let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_get_vm(
        State(state): State<SharedState>,
        Path(name): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        match client.get_vm(&namespace, &name).await {
            Ok(vm) => {
                let ip = client.get_vm_ip(&namespace, &name).await.unwrap_or(None);
                let mut body = fabric_vm_json(&VmInfo::from_vm_with_ip(&vm, ip));
                if let Ok(pfs) = client.list_port_forwards(&namespace, &name).await {
                    if let Some(obj) = body.as_object_mut() {
                        obj.insert(
                            "port_forwards".into(),
                            serde_json::json!(pfs
                                .iter()
                                .map(|p| serde_json::json!({
                                    "host_port": p.host_port,
                                    "guest_port": p.guest_port,
                                    "protocol": p.protocol,
                                    "expose_host": p.expose_host,
                                }))
                                .collect::<Vec<_>>()),
                        );
                    }
                }
                Json(body).into_response()
            }
            Err(e) => {
                let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_start_vm(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(name): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        drop(s);
        let result = client.start_vm(&namespace, &name).await;
        record_audit(
            &state,
            &headers,
            crate::audit_trail::AuditAction::Start,
            "vm",
            &name,
            result.is_ok(),
            result.as_ref().err().map(|e| sanitize_error(e)),
        )
        .await;
        webhook_handlers::dispatch_webhook_event(WebhookEvent::VMStarted, &name, result.is_ok())
            .await;
        match result {
            Ok(_) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                let (st, j) = err_json(500, "START_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_stop_vm(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(name): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        drop(s);
        let result = client.stop_vm(&namespace, &name).await;
        record_audit(
            &state,
            &headers,
            crate::audit_trail::AuditAction::Stop,
            "vm",
            &name,
            result.is_ok(),
            result.as_ref().err().map(|e| sanitize_error(e)),
        )
        .await;
        webhook_handlers::dispatch_webhook_event(WebhookEvent::VMStopped, &name, result.is_ok())
            .await;
        match result {
            Ok(_) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                let (st, j) = err_json(500, "STOP_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_restart_vm(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(name): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        drop(s);
        let result = client.restart_vm(&namespace, &name).await;
        record_audit(
            &state,
            &headers,
            crate::audit_trail::AuditAction::Restart,
            "vm",
            &name,
            result.is_ok(),
            result.as_ref().err().map(|e| sanitize_error(e)),
        )
        .await;
        webhook_handlers::dispatch_webhook_event(WebhookEvent::VMRestarted, &name, result.is_ok())
            .await;
        match result {
            Ok(_) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                let (st, j) = err_json(500, "RESTART_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_delete_vm(
        State(state): State<SharedState>,
        headers: HeaderMap,
        Path(name): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        drop(s);
        let _ = client.delete_vm_port_forwards(&namespace, &name).await;
        let result = client.delete_vm(&namespace, &name).await;
        record_audit(
            &state,
            &headers,
            crate::audit_trail::AuditAction::Delete,
            "vm",
            &name,
            result.is_ok(),
            result.as_ref().err().map(|e| sanitize_error(e)),
        )
        .await;
        webhook_handlers::dispatch_webhook_event(WebhookEvent::VMDeleted, &name, result.is_ok())
            .await;
        match result {
            Ok(_) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                let (st, j) = err_json(500, "DELETE_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_list_snapshots(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        drop(s);
        match crate::snapshots::SnapshotManager::new(&namespace).await {
            Ok(manager) => match manager.list_all_snapshots().await {
                Ok(snapshots) => {
                    let items: Vec<_> = snapshots
                        .into_iter()
                        .map(|s| {
                            let warning = s.captured_no_volumes().then(|| format!(
                                "VM '{}' has no PVC/DataVolume-backed disks -- this snapshot captured only the VM's configuration, not any disk data.",
                                s.vm_name
                            ));
                            serde_json::json!({
                                "id": s.name,
                                "vm_name": s.vm_name,
                                "name": s.name,
                                "description": null,
                                "snapshot_type": "Disk",
                                "parent_id": null,
                                "size_bytes": 0,
                                "created": s.created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                                "warning": warning,
                            })
                        })
                        .collect();
                    Json(items).into_response()
                }
                Err(e) => {
                    let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
                    (st, j).into_response()
                }
            },
            Err(e) => {
                let (st, j) = err_json(503, "SERVICE_UNAVAILABLE", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_list_vm_snapshots(
        State(state): State<SharedState>,
        Path(vm): Path<String>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        drop(s);
        match crate::snapshots::SnapshotManager::new(&namespace).await {
            Ok(manager) => match manager.list_snapshots_for_vm(&vm).await {
                Ok(snapshots) => {
                    let items: Vec<_> = snapshots
                        .into_iter()
                        .map(|s| {
                            let warning = s.captured_no_volumes().then(|| format!(
                                "VM '{}' has no PVC/DataVolume-backed disks -- this snapshot captured only the VM's configuration, not any disk data.",
                                vm
                            ));
                            serde_json::json!({
                                "id": s.name,
                                "vm_name": vm,
                                "name": s.name,
                                "description": null,
                                "snapshot_type": "Disk",
                                "parent_id": null,
                                "size_bytes": 0,
                                "created": s.created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                                "warning": warning,
                            })
                        })
                        .collect();
                    Json(items).into_response()
                }
                Err(e) => {
                    let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
                    (st, j).into_response()
                }
            },
            Err(e) => {
                let (st, j) = err_json(503, "SERVICE_UNAVAILABLE", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    async fn fabric_list_events(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        use k8s_openapi::api::core::v1::Event;
        use kube::Api;
        let events_api: Api<Event> = Api::namespaced(client.client(), &namespace);
        let lp = kube::api::ListParams::default().limit(100);
        match events_api.list(&lp).await {
            Ok(event_list) => {
                let items: Vec<_> = event_list
                    .items
                    .into_iter()
                    .map(|e| {
                        serde_json::json!({
                            "type": e.type_.unwrap_or_else(|| "Normal".into()),
                            "reason": e.reason.unwrap_or_default(),
                            "message": e.message.unwrap_or_default(),
                            "object": e.involved_object.name.unwrap_or_default(),
                            "timestamp": e.last_timestamp
                                .map(|t| t.0.to_string())
                                .or_else(|| e.metadata.creation_timestamp.map(|t| t.0.to_string()))
                                .unwrap_or_default(),
                        })
                    })
                    .collect();
                Json(items).into_response()
            }
            Err(e) => {
                let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    /// Live SSE stream of cluster `Event` objects, shaped for `useEventStream.ts`.
    async fn fabric_events_stream(State(state): State<SharedState>) -> impl IntoResponse {
        use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
        use futures_util::StreamExt;
        use k8s_openapi::api::core::v1::Event as K8sEvent;
        use kube::runtime::{watcher, WatchStreamExt};
        use kube::Api;
        use std::convert::Infallible;

        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        drop(s);

        let api: Api<K8sEvent> = Api::namespaced(client.client(), &namespace);
        let stream = watcher(api, watcher::Config::default())
            .applied_objects()
            .filter_map(|res| async move { res.ok() })
            .map(|e| {
                let payload = serde_json::json!({
                    "id": e.metadata.uid.clone().unwrap_or_default(),
                    "event_type": e.reason.clone().unwrap_or_else(|| "Normal".into()),
                    "vm_name": e.involved_object.name.clone().unwrap_or_default(),
                    "detail": e.message.clone(),
                    "timestamp": e.last_timestamp
                        .as_ref()
                        .map(|t| t.0.to_string())
                        .or_else(|| {
                            e.metadata.creation_timestamp.as_ref().map(|t| t.0.to_string())
                        })
                        .unwrap_or_default(),
                });
                Ok::<_, Infallible>(
                    SseEvent::default()
                        .event("vm-event")
                        .data(payload.to_string()),
                )
            });

        Sse::new(stream).keep_alive(KeepAlive::default())
    }

    /// Lightweight live-reachability probe for each subsystem the "offline" pill and
    /// capabilities page care about.
    async fn fabric_capabilities(State(state): State<SharedState>) -> impl IntoResponse {
        use k8s_openapi::api::core::v1::PersistentVolumeClaim;
        use k8s_openapi::api::networking::v1::NetworkPolicy;
        use kube::Api;

        fn phase(ok: bool, err: Option<String>) -> serde_json::Value {
            serde_json::json!({ "phase": if ok { "live" } else { "unreachable" }, "detail": err })
        }

        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        let auth_configured =
            s.lab_api_key.is_some() || std::env::var("ZORVIA_ADMIN_PASSWORD").is_ok();
        drop(s);

        let vm_driver = match client.list_vms(&namespace).await {
            Ok(_) => phase(true, None),
            Err(e) => phase(false, Some(sanitize_error(&e))),
        };

        let events = {
            use k8s_openapi::api::core::v1::Event;
            let api: Api<Event> = Api::namespaced(client.client(), &namespace);
            match api.list(&kube::api::ListParams::default().limit(1)).await {
                Ok(_) => phase(true, None),
                Err(e) => phase(false, Some(sanitize_error(&e))),
            }
        };

        let storage = {
            let api: Api<PersistentVolumeClaim> = Api::namespaced(client.client(), &namespace);
            match api.list(&kube::api::ListParams::default().limit(1)).await {
                Ok(_) => phase(true, None),
                Err(e) => phase(false, Some(sanitize_error(&e))),
            }
        };

        let network_security = {
            let api: Api<NetworkPolicy> = Api::namespaced(client.client(), &namespace);
            match api.list(&kube::api::ListParams::default().limit(1)).await {
                Ok(_) => phase(true, None),
                Err(e) => phase(false, Some(sanitize_error(&e))),
            }
        };

        let auth_phase = if auth_configured { "live" } else { "off" };
        let auth = serde_json::json!({ "phase": auth_phase, "detail": null });

        Json(serde_json::json!({
            "vm_driver": vm_driver,
            "storage": storage,
            "network_security": network_security,
            "vm_dataplane": network_security,
            "auth": auth,
            "events": events,
            "hubble_ui_url": std::env::var("ZORVIA_HUBBLE_UI_URL").ok(),
        }))
    }

    async fn fabric_overview(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();
        match client.list_vms(&namespace).await {
            Ok(vms) => {
                let mut running = 0u32;
                let mut stopped = 0u32;
                let mut failed = 0u32;
                let total = vms.len() as u32;
                for vm in &vms {
                    let status = vm
                        .status
                        .as_ref()
                        .and_then(|st| st.printable_status.as_deref())
                        .unwrap_or("Unknown");
                    match fabric_map_state(status) {
                        "running" => running += 1,
                        "failed" => failed += 1,
                        _ => stopped += 1,
                    }
                }
                Json(serde_json::json!({
                    "vms": { "total": total, "running": running, "stopped": stopped, "failed": failed },
                    "namespace": namespace,
                }))
                .into_response()
            }
            Err(e) => {
                let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
                (st, j).into_response()
            }
        }
    }

    fn req_ctx(method: HttpMethod, path: &str) -> RequestContext {
        RequestContext::new(method, path)
    }

    /// Sanitize internal error details before sending to clients.
    ///
    /// For known patterns (e.g. kube errors), returns just the first sentence.
    /// For anything else, returns a generic message to avoid leaking internals.
    fn sanitize_error(e: &dyn std::fmt::Display) -> String {
        let msg = e.to_string();
        // Known patterns where the first sentence is safe to expose
        let known_prefixes = [
            "ApiError",
            "NotFound",
            "Conflict",
            "Unauthorized",
            "Forbidden",
            "Timeout",
            "connection",
        ];
        let is_known = known_prefixes.iter().any(|p| msg.starts_with(p));

        if is_known {
            // Keep the first sentence, bounded to a safe length. Cutting at
            // the first ": " (as this used to do) is wrong: kube-rs's
            // ApiError Display starts "ApiError: <reason>", so the very
            // first ": " is the one separating the prefix from the actual
            // reason — truncating there dropped 100% of the useful detail,
            // leaving clients with just the literal word "ApiError" and no
            // way to diagnose a failed request. Sentence-end (". ") is a
            // safe cut point since it only appears after real content.
            let mut end = msg.find(". ").unwrap_or(msg.len()).min(500);
            while end > 0 && !msg.is_char_boundary(end) {
                end -= 1;
            }
            msg[..end].to_string()
        } else {
            "Internal server error".to_string()
        }
    }

    // ── VM Endpoints ──────────────────────────────────────────────

    #[derive(Deserialize)]
    pub struct VmQuery {
        pub namespace: Option<String>,
    }

    async fn list_vms_handler(
        State(state): State<SharedState>,
        Query(query): Query<VmQuery>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = query
            .namespace
            .clone()
            .unwrap_or_else(|| s.namespace.clone());
        let client = s.client();

        match client.list_vms(&namespace).await {
            Ok(vms) => {
                let mut vm_infos = Vec::with_capacity(vms.len());
                for vm in &vms {
                    let name = vm.metadata.name.clone().unwrap_or_default();
                    let ip = client.get_vm_ip(&namespace, &name).await.unwrap_or(None);
                    vm_infos.push(VmInfo::from_vm_with_ip(vm, ip));
                }
                let ctx = req_ctx(HttpMethod::GET, "/api/v1/vms");
                ok_json(&ApiResponse::success(&vm_infos, &ctx.request_id))
            }
            Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
        }
    }

    async fn get_vm_handler(
        State(state): State<SharedState>,
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        let s = state.read().await;
        let client = s.client();

        match client.get_vm(&ns, &name).await {
            Ok(vm) => {
                let ip = client.get_vm_ip(&ns, &name).await.unwrap_or(None);
                let info = VmInfo::from_vm_with_ip(&vm, ip);

                // Also fetch VMI details if running
                let vmi_detail = client.get_vmi(&ns, &name).await.ok();
                let detail = VmDetail {
                    info,
                    vmi_status: vmi_detail.and_then(|v| v.status),
                };

                let ctx = req_ctx(HttpMethod::GET, "/api/v1/vms/{ns}/{name}");
                ok_json(&ApiResponse::success(&detail, &ctx.request_id))
            }
            Err(e) => {
                let msg = sanitize_error(&e);
                if msg.contains("NotFound") || msg.contains("not found") {
                    err_json(404, "NOT_FOUND", &format!("VM '{}' not found", name))
                } else {
                    err_json(500, "INTERNAL_ERROR", &msg)
                }
            }
        }
    }

    async fn start_vm_handler(
        State(state): State<SharedState>,
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        let s = state.read().await;
        let client = s.client();

        match client.start_vm(&ns, &name).await {
            Ok(_) => {
                let ctx = req_ctx(HttpMethod::POST, "/api/v1/vms/{ns}/{name}/start");
                ok_json(&ApiResponse::success(
                    &serde_json::json!({"message": format!("VM '{}' started", name)}),
                    &ctx.request_id,
                ))
            }
            Err(e) => err_json(500, "START_FAILED", &sanitize_error(&e)),
        }
    }

    async fn stop_vm_handler(
        State(state): State<SharedState>,
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        let s = state.read().await;
        let client = s.client();

        match client.stop_vm(&ns, &name).await {
            Ok(_) => {
                let ctx = req_ctx(HttpMethod::POST, "/api/v1/vms/{ns}/{name}/stop");
                ok_json(&ApiResponse::success(
                    &serde_json::json!({"message": format!("VM '{}' stopped", name)}),
                    &ctx.request_id,
                ))
            }
            Err(e) => err_json(500, "STOP_FAILED", &sanitize_error(&e)),
        }
    }

    async fn restart_vm_handler(
        State(state): State<SharedState>,
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        let s = state.read().await;
        let client = s.client();

        match client.restart_vm(&ns, &name).await {
            Ok(_) => {
                let ctx = req_ctx(HttpMethod::POST, "/api/v1/vms/{ns}/{name}/restart");
                ok_json(&ApiResponse::success(
                    &serde_json::json!({"message": format!("VM '{}' restarted", name)}),
                    &ctx.request_id,
                ))
            }
            Err(e) => err_json(500, "RESTART_FAILED", &sanitize_error(&e)),
        }
    }

    async fn delete_vm_handler(
        State(state): State<SharedState>,
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        let s = state.read().await;
        let client = s.client();

        match client.delete_vm(&ns, &name).await {
            Ok(_) => {
                let ctx = req_ctx(HttpMethod::DELETE, "/api/v1/vms/{ns}/{name}");
                ok_json(&ApiResponse::success(
                    &serde_json::json!({"message": format!("VM '{}' deleted", name)}),
                    &ctx.request_id,
                ))
            }
            Err(e) => err_json(500, "DELETE_FAILED", &sanitize_error(&e)),
        }
    }

    // ── Snapshot Endpoints ────────────────────────────────────────

    async fn list_snapshots_handler(
        State(state): State<SharedState>,
        Query(query): Query<VmQuery>,
    ) -> impl IntoResponse {
        let namespace = {
            let s = state.read().await;
            query
                .namespace
                .clone()
                .unwrap_or_else(|| s.namespace.clone())
        };

        match crate::snapshots::SnapshotManager::new(&namespace).await {
            Ok(manager) => match manager.list_all_snapshots().await {
                Ok(snapshots) => {
                    let items: Vec<SnapshotItem> = snapshots
                        .into_iter()
                        .map(|s| {
                            let age = s.age();
                            let status = s.status.to_string();
                            SnapshotItem {
                                name: s.name,
                                vm_name: s.vm_name,
                                status,
                                ready: s.ready_to_use,
                                age,
                            }
                        })
                        .collect();
                    let ctx = req_ctx(HttpMethod::GET, "/api/v1/snapshots");
                    ok_json(&ApiResponse::success(&items, &ctx.request_id))
                }
                Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
            },
            Err(e) => err_json(503, "SERVICE_UNAVAILABLE", &sanitize_error(&e)),
        }
    }

    async fn list_vm_snapshots_handler(
        Path((ns, vm)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("vm", &vm)]) {
            return resp;
        }

        match crate::snapshots::SnapshotManager::new(&ns).await {
            Ok(manager) => match manager.list_snapshots_for_vm(&vm).await {
                Ok(snapshots) => {
                    let items: Vec<SnapshotItem> = snapshots
                        .into_iter()
                        .map(|s| {
                            let age = s.age();
                            let status = s.status.to_string();
                            SnapshotItem {
                                name: s.name,
                                vm_name: s.vm_name,
                                status,
                                ready: s.ready_to_use,
                                age,
                            }
                        })
                        .collect();
                    let ctx = req_ctx(HttpMethod::GET, "/api/v1/snapshots/{ns}/{vm}");
                    ok_json(&ApiResponse::success(&items, &ctx.request_id))
                }
                Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
            },
            Err(e) => err_json(503, "SERVICE_UNAVAILABLE", &sanitize_error(&e)),
        }
    }

    async fn delete_snapshot_handler(
        Path((ns, name)): Path<(String, String)>,
    ) -> impl IntoResponse {
        if let Some(resp) = validate_k8s_params(&[("namespace", &ns), ("name", &name)]) {
            return resp;
        }

        match crate::snapshots::SnapshotManager::new(&ns).await {
            Ok(manager) => match manager.delete_snapshot(&name).await {
                Ok(_) => {
                    let ctx = req_ctx(HttpMethod::POST, "/api/v1/snapshots/{ns}/{name}/delete");
                    ok_json(&ApiResponse::success(
                        &serde_json::json!({"message": format!("Snapshot '{}' deleted", name)}),
                        &ctx.request_id,
                    ))
                }
                Err(e) => err_json(500, "DELETE_FAILED", &sanitize_error(&e)),
            },
            Err(e) => err_json(503, "SERVICE_UNAVAILABLE", &sanitize_error(&e)),
        }
    }

    // ── Events ────────────────────────────────────────────────────

    #[derive(Deserialize)]
    pub struct EventsQuery {
        pub limit: Option<u32>,
    }

    async fn list_events_handler(
        State(state): State<SharedState>,
        Query(query): Query<EventsQuery>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();

        let limit = query.limit.unwrap_or(50).min(1000);

        use k8s_openapi::api::core::v1::Event;
        use kube::Api;
        let events_api: Api<Event> = Api::namespaced(client.client(), &namespace);
        let lp = kube::api::ListParams::default().limit(limit);
        match events_api.list(&lp).await {
            Ok(event_list) => {
                let items: Vec<EventItem> = event_list
                    .items
                    .into_iter()
                    .map(|e| EventItem {
                        type_: e.type_.unwrap_or_default(),
                        reason: e.reason.unwrap_or_default(),
                        message: e.message.unwrap_or_default(),
                        namespace: e.metadata.namespace.unwrap_or_default(),
                        involved_object: e.involved_object.name.unwrap_or_default(),
                        timestamp: e
                            .last_timestamp
                            .map(|t| t.0.to_string())
                            .or_else(|| e.metadata.creation_timestamp.map(|t| t.0.to_string()))
                            .unwrap_or_default(),
                    })
                    .collect();
                let ctx = req_ctx(HttpMethod::GET, "/api/v1/events");
                ok_json(&ApiResponse::success(&items, &ctx.request_id))
            }
            Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
        }
    }

    async fn recent_events_handler(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();

        use k8s_openapi::api::core::v1::Event;
        use kube::Api;
        let events_api: Api<Event> = Api::namespaced(client.client(), &namespace);
        let lp = kube::api::ListParams::default();
        match events_api.list(&lp).await {
            Ok(event_list) => {
                let one_hour_ago_secs = k8s_openapi::jiff::Timestamp::now().as_second() - 3600;
                let items: Vec<EventItem> = event_list
                    .items
                    .into_iter()
                    .filter(|e| {
                        // Keep events from the last hour based on timestamp
                        let ts = e
                            .last_timestamp
                            .as_ref()
                            .map(|t| t.0)
                            .or_else(|| e.metadata.creation_timestamp.as_ref().map(|t| t.0));
                        match ts {
                            Some(t) => t.as_second() >= one_hour_ago_secs,
                            None => false, // exclude events with no timestamp
                        }
                    })
                    .map(|e| EventItem {
                        type_: e.type_.unwrap_or_default(),
                        reason: e.reason.unwrap_or_default(),
                        message: e.message.unwrap_or_default(),
                        namespace: e.metadata.namespace.unwrap_or_default(),
                        involved_object: e.involved_object.name.unwrap_or_default(),
                        timestamp: e
                            .last_timestamp
                            .map(|t| t.0.to_string())
                            .or_else(|| e.metadata.creation_timestamp.map(|t| t.0.to_string()))
                            .unwrap_or_default(),
                    })
                    .collect();
                let ctx = req_ctx(HttpMethod::GET, "/api/v1/events/recent");
                ok_json(&ApiResponse::success(&items, &ctx.request_id))
            }
            Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
        }
    }

    // ── Dashboard Overview ────────────────────────────────────────

    async fn dashboard_overview_handler(State(state): State<SharedState>) -> impl IntoResponse {
        let s = state.read().await;
        let namespace = s.namespace.clone();
        let client = s.client();

        let vms = match client.list_vms(&namespace).await {
            Ok(vms) => vms,
            Err(e) => {
                return err_json(500, "INTERNAL_ERROR", &sanitize_error(&e));
            }
        };
        let total = vms.len();
        let mut running = 0usize;
        let mut stopped = 0usize;
        for vm in &vms {
            match vm
                .status
                .as_ref()
                .and_then(|s| s.printable_status.as_deref())
            {
                Some("Running") => running += 1,
                Some("Stopped") => stopped += 1,
                _ => {}
            }
        }
        let error = total.saturating_sub(running + stopped);

        // Count total allocated CPU/memory
        let mut total_cpus = 0u32;
        let mut total_memory_bytes = 0u64;
        for vm in &vms {
            total_cpus += vm
                .spec
                .template
                .spec
                .domain
                .cpu
                .as_ref()
                .and_then(|c| c.cores)
                .unwrap_or(1);
            if let Some(mem_str) = vm
                .spec
                .template
                .spec
                .domain
                .resources
                .requests
                .as_ref()
                .and_then(|r| r.get("memory"))
            {
                total_memory_bytes += parse_memory(mem_str);
            }
        }

        let snapshot_count = match crate::snapshots::SnapshotManager::new(&namespace).await {
            Ok(m) => m.list_all_snapshots().await.map(|s| s.len()).unwrap_or(0),
            Err(_) => 0,
        };

        let overview = DashboardOverview {
            cluster: ClusterStats {
                total_vms: total,
                running_vms: running,
                stopped_vms: stopped,
                error_vms: error,
                total_vcpus_allocated: total_cpus,
                total_memory_allocated_gb: (total_memory_bytes as f64) / (1024.0 * 1024.0 * 1024.0),
                total_snapshots: snapshot_count,
            },
        };

        let ctx = req_ctx(HttpMethod::GET, "/api/v1/dashboard/overview");
        ok_json(&ApiResponse::success(&overview, &ctx.request_id))
    }

    // ── Health ─────────────────────────────────────────────────────

    async fn health_handler() -> impl IntoResponse {
        let ctx = req_ctx(HttpMethod::GET, "/api/v1/health");
        let health = serde_json::json!({
            "status": "healthy",
            "version": "v1",
            "service": "zorvia-api"
        });
        ok_json(&ApiResponse::success(&health, &ctx.request_id))
    }

    async fn features_registry_handler() -> impl IntoResponse {
        Json(crate::features::registry_json())
    }

    fn enterprise_err(msg: String) -> (StatusCode, Json<serde_json::Value>) {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "success": false,
                "status": 403,
                "error": { "code": "feature_disabled", "message": msg },
                "data": null
            })),
        )
    }

    #[derive(Deserialize)]
    struct S3BackupPlanReq {
        vm_name: String,
        snapshot_name: String,
        bucket: String,
        #[serde(default = "default_region")]
        region: String,
        #[serde(default)]
        prefix: String,
        #[serde(default = "default_retention")]
        retention_days: u32,
    }
    fn default_region() -> String {
        "us-east-1".into()
    }
    fn default_retention() -> u32 {
        30
    }

    async fn enterprise_s3_backup_plan(Json(body): Json<S3BackupPlanReq>) -> impl IntoResponse {
        match crate::enterprise::S3ImmutableBackupPlan::try_plan(
            body.vm_name,
            body.snapshot_name,
            body.bucket,
            body.region,
            body.prefix,
            body.retention_days,
        ) {
            Ok(plan) => Json(serde_json::json!({ "success": true, "data": plan })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    #[derive(Deserialize)]
    struct TransivaPlanReq {
        source_vm: String,
        vcenter_endpoint: String,
        #[serde(default = "default_ns")]
        target_namespace: String,
    }
    fn default_ns() -> String {
        "default".into()
    }

    async fn enterprise_transiva_plan(Json(body): Json<TransivaPlanReq>) -> impl IntoResponse {
        match crate::enterprise::TransivaPlan::try_plan(
            body.source_vm,
            body.vcenter_endpoint,
            body.target_namespace,
        ) {
            Ok(plan) => Json(serde_json::json!({ "success": true, "data": plan })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    #[derive(Deserialize)]
    struct GoldenPipelineReq {
        image_name: String,
        version: String,
        #[serde(default = "default_ns")]
        namespace: String,
    }

    async fn enterprise_golden_pipeline_plan(
        Json(body): Json<GoldenPipelineReq>,
    ) -> impl IntoResponse {
        match crate::enterprise::GoldenPipelinePlan::try_plan(
            body.image_name,
            body.version,
            body.namespace,
        ) {
            Ok(plan) => Json(serde_json::json!({ "success": true, "data": plan })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    #[derive(Deserialize)]
    struct GoldenPipelineRunReq {
        image_name: String,
        version: String,
        /// Registry path (`quay.io/...`) or `docker://` / `https://` source.
        source: String,
        #[serde(default = "default_ns")]
        namespace: String,
        #[serde(default = "default_golden_size")]
        size: String,
    }
    fn default_golden_size() -> String {
        "20Gi".into()
    }

    async fn enterprise_golden_pipeline_run(
        State(state): State<SharedState>,
        Json(body): Json<GoldenPipelineRunReq>,
    ) -> impl IntoResponse {
        let s = state.read().await;
        let client = s.kube_client.clone();
        drop(s);
        match crate::enterprise::GoldenPipelineRun::try_run(
            body.image_name,
            body.version,
            body.namespace,
            body.source,
            body.size,
            &client,
        )
        .await
        {
            Ok(run) => Json(serde_json::json!({ "success": true, "data": run })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    #[derive(Deserialize)]
    struct CrossClusterDrReq {
        source_cluster: String,
        target_cluster: String,
        vm_names: Vec<String>,
        #[serde(default = "default_rpo")]
        rpo_seconds: u64,
    }
    fn default_rpo() -> u64 {
        300
    }

    async fn enterprise_cross_cluster_dr_plan(
        Json(body): Json<CrossClusterDrReq>,
    ) -> impl IntoResponse {
        match crate::enterprise::CrossClusterDrPlan::try_plan(
            body.source_cluster,
            body.target_cluster,
            body.vm_names,
            body.rpo_seconds,
        ) {
            Ok(plan) => Json(serde_json::json!({ "success": true, "data": plan })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    #[derive(Deserialize)]
    struct GpuNumaReq {
        vm_name: String,
        #[serde(default = "default_gpu")]
        gpu_count: u32,
        #[serde(default)]
        sriov_networks: Vec<String>,
        #[serde(default)]
        numa_passthrough: bool,
    }
    fn default_gpu() -> u32 {
        1
    }

    async fn enterprise_gpu_numa_plan(Json(body): Json<GpuNumaReq>) -> impl IntoResponse {
        match crate::enterprise::GpuSriovNumaPlan::try_plan(
            body.vm_name,
            body.gpu_count,
            body.sriov_networks,
            body.numa_passthrough,
        ) {
            Ok(plan) => Json(serde_json::json!({ "success": true, "data": plan })).into_response(),
            Err(e) => enterprise_err(e).into_response(),
        }
    }

    async fn enterprise_fleet_inventory(State(state): State<SharedState>) -> impl IntoResponse {
        if !crate::features::enterprise_flag("ZORVIA_FEATURE_FLEET") {
            return enterprise_err(
                "Enterprise feature disabled. Set ZORVIA_EXPERIMENTAL=1 and ZORVIA_FEATURE_FLEET=1"
                    .into(),
            )
            .into_response();
        }
        let fleet = state.read().await.fleet.clone();
        Json(serde_json::json!({ "success": true, "data": fleet.snapshot().await })).into_response()
    }

    // ── Types ──────────────────────────────────────────────────────

    #[derive(Serialize)]
    struct VmDetail {
        #[serde(flatten)]
        info: VmInfo,
        vmi_status: Option<crate::kube::types::VirtualMachineInstanceStatus>,
    }

    #[derive(Serialize)]
    struct SnapshotItem {
        name: String,
        vm_name: String,
        status: String,
        ready: bool,
        age: String,
    }

    #[derive(Serialize)]
    struct EventItem {
        #[serde(rename = "type")]
        type_: String,
        reason: String,
        message: String,
        namespace: String,
        involved_object: String,
        timestamp: String,
    }

    #[derive(Serialize)]
    struct DashboardOverview {
        cluster: ClusterStats,
    }

    #[derive(Serialize)]
    struct ClusterStats {
        total_vms: usize,
        running_vms: usize,
        stopped_vms: usize,
        error_vms: usize,
        total_vcpus_allocated: u32,
        total_memory_allocated_gb: f64,
        total_snapshots: usize,
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn ok_json<T: Serialize>(data: &T) -> (StatusCode, Json<serde_json::Value>) {
        let value = serde_json::to_value(data).unwrap_or_else(
            |e| serde_json::json!({"error": format!("serialization failed: {}", e)}),
        );
        // Extract the status code from the serialized response if present,
        // so that 201/204/etc. responses get the correct HTTP status.
        let status_code = value
            .get("status")
            .and_then(|v| v.as_u64())
            .and_then(|s| StatusCode::from_u16(s as u16).ok())
            .unwrap_or(StatusCode::OK);
        (status_code, Json(value))
    }

    pub(crate) fn err_json(
        status: u16,
        code: &str,
        message: &str,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let ctx = req_ctx(HttpMethod::GET, "");
        let resp = ApiResponse::error(status, code, message, &ctx.request_id);
        let value = serde_json::to_value(&resp).unwrap_or_else(
            |_| serde_json::json!({"status": status, "error": code, "message": "internal error"}),
        );
        (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(value),
        )
    }

    /// Best-effort caller identity for audit entries -- decodes the same
    /// Bearer token `auth_middleware` already validated, so this never
    /// fails a request on its own account.
    fn audit_username(headers: &HeaderMap, auth: &crate::api::auth::SharedAuth) -> String {
        if let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        {
            if let Some(identity) = auth.resolve_credential(token) {
                return identity.username.unwrap_or_else(|| "api-token".to_string());
            }
        }
        if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
            if let Some(identity) = auth.resolve_credential(key) {
                return identity.username.unwrap_or_else(|| "api-token".to_string());
            }
        }
        "unknown".to_string()
    }

    /// Records one audit entry. `crate::audit_trail::AuditTrail` had real
    /// query/stats logic with zero callers anywhere in the codebase until
    /// this -- every mutating VM handler wired here closes that gap for
    /// the actions VMDetails.tsx's activity panel actually shows
    /// (create/start/stop/restart/delete/migrate).
    async fn record_audit(
        state: &SharedState,
        headers: &HeaderMap,
        action: crate::audit_trail::AuditAction,
        resource_type: &str,
        resource_name: &str,
        success: bool,
        message: Option<String>,
    ) {
        let s = state.read().await;
        let user = audit_username(headers, &s.auth);
        let namespace = s.namespace.clone();
        let audit = s.audit.clone();
        drop(s);
        let entry = crate::audit_trail::AuditEntry {
            id: crate::utils::generate_id("audit", resource_name),
            timestamp: chrono::Utc::now(),
            user,
            action,
            resource_type: resource_type.to_string(),
            resource_name: resource_name.to_string(),
            namespace,
            details: message
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
            ip_address: String::new(),
            success,
            severity: crate::audit_trail::AuditSeverity::Info,
        };
        audit.write().await.record(entry);
    }

    fn parse_memory(s: &str) -> u64 {
        let s = s.trim();
        if let Some(val) = s.strip_suffix("Gi") {
            val.parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
        } else if let Some(val) = s.strip_suffix("Mi") {
            val.parse::<u64>().unwrap_or(0) * 1024 * 1024
        } else if let Some(val) = s.strip_suffix("Ki") {
            val.parse::<u64>().unwrap_or(0) * 1024
        } else {
            s.parse::<u64>().unwrap_or(0)
        }
    }
}
