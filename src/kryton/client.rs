// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

use super::models::*;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub token: Option<String>,
    pub default_project: Option<String>,
    pub timeout: Duration,
    pub allow_invalid_tls: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let base_url = match std::env::var("KRYTON_URL") {
            Ok(value) if !value.trim().is_empty() => value.trim().trim_end_matches('/').to_string(),
            _ => return Ok(None),
        };
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|e| anyhow::anyhow!("invalid KRYTON_URL: {e}"))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            anyhow::bail!("KRYTON_URL must use http:// or https://");
        }
        let timeout_secs = std::env::var("KRYTON_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0 && *v <= 300)
            .unwrap_or(30);
        let allow_invalid_tls = std::env::var("KRYTON_TLS_INSECURE")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        Ok(Some(Self {
            base_url,
            token: std::env::var("KRYTON_TOKEN").ok().filter(|v| !v.is_empty()),
            default_project: std::env::var("KRYTON_PROJECT")
                .ok()
                .filter(|v| !v.is_empty()),
            timeout: Duration::from_secs(timeout_secs),
            allow_invalid_tls,
        }))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Kryton request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Kryton returned HTTP {status}: {message}")]
    Upstream {
        status: u16,
        code: Option<String>,
        message: String,
        hint: Option<String>,
    },
    #[error("Kryton project is required; set KRYTON_PROJECT or pass ?project=")]
    MissingProject,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamEnvelope {
    error: Option<UpstreamError>,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamError {
    code: Option<String>,
    message: Option<String>,
    hint: Option<String>,
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
                "KRYTON_TLS_INSECURE is enabled; Kryton TLS certificates will not be verified"
            );
        }
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .danger_accept_invalid_certs(config.allow_invalid_tls)
            .user_agent(format!(
                "zorvia/{}/kryton-adapter",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self { config, http })
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    pub fn configured_project(&self) -> Option<&str> {
        self.config.default_project.as_deref()
    }

    fn project<'a>(&'a self, project: Option<&'a str>) -> Result<&'a str, Error> {
        project
            .filter(|v| !v.trim().is_empty())
            .or(self.config.default_project.as_deref())
            .ok_or(Error::MissingProject)
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

    async fn empty(&self, request: reqwest::RequestBuilder) -> Result<(), Error> {
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
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
            code: parsed.as_ref().and_then(|e| e.code.clone()),
            message,
            hint: parsed.and_then(|e| e.hint),
        }
    }

    fn machine_path(id: &str) -> String {
        format!("/api/v1/machines/{}", urlencoding::encode(id))
    }

    fn snapshot_path(machine_id: &str, snapshot_id: &str) -> String {
        format!(
            "/api/v1/machines/{}/snapshots/{}",
            urlencoding::encode(machine_id),
            urlencoding::encode(snapshot_id)
        )
    }

    pub async fn ready(&self) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, "/readyz")).await
    }

    pub async fn capabilities(&self) -> Result<Capabilities, Error> {
        self.decode(self.request(Method::GET, "/api/v1/capabilities"))
            .await
    }

    pub async fn doctor(&self) -> Result<DoctorReport, Error> {
        self.decode(self.request(Method::GET, "/api/v1/doctor"))
            .await
    }

    pub async fn images(&self) -> Result<ListResponse<Image>, Error> {
        self.decode(self.request(Method::GET, "/api/v1/images"))
            .await
    }

    pub async fn summary(&self, project: Option<&str>) -> Result<Summary, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(Method::GET, "/api/v1/summary")
                .query(&[("project", project)]),
        )
        .await
    }

    pub async fn machines(
        &self,
        project: Option<&str>,
        limit: Option<u16>,
        cursor: Option<&str>,
    ) -> Result<ListResponse<Machine>, Error> {
        let project = self.project(project)?;
        let mut query: Vec<(&str, String)> = vec![("project", project.to_string())];
        if let Some(limit) = limit {
            query.push(("limit", limit.clamp(1, 500).to_string()));
        }
        if let Some(cursor) = cursor.filter(|v| !v.is_empty()) {
            query.push(("cursor", cursor.to_string()));
        }
        self.decode(self.request(Method::GET, "/api/v1/machines").query(&query))
            .await
    }

    pub async fn machine(&self, id: &str, project: Option<&str>) -> Result<Machine, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(Method::GET, &Self::machine_path(id))
                .query(&[("project", project)]),
        )
        .await
    }

    pub async fn create(&self, mut request: CreateMachineRequest) -> Result<Machine, Error> {
        if request.project.trim().is_empty() {
            request.project = self.project(None)?.to_string();
        }
        self.decode(
            self.request(Method::POST, "/api/v1/machines")
                .json(&request),
        )
        .await
    }

    pub async fn start(&self, id: &str, project: Option<&str>) -> Result<Machine, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(Method::POST, &format!("{}/start", Self::machine_path(id)))
                .query(&[("project", project)]),
        )
        .await
    }

    pub async fn stop(&self, id: &str, project: Option<&str>) -> Result<Machine, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(Method::POST, &format!("{}/stop", Self::machine_path(id)))
                .query(&[("project", project)]),
        )
        .await
    }

    pub async fn delete(&self, id: &str, project: Option<&str>) -> Result<(), Error> {
        let project = self.project(project)?;
        self.empty(
            self.request(Method::DELETE, &Self::machine_path(id))
                .query(&[("project", project)]),
        )
        .await
    }

    pub async fn snapshot(
        &self,
        id: &str,
        project: Option<&str>,
        name: Option<String>,
    ) -> Result<Snapshot, Error> {
        let project = self.project(project)?;
        let body = SnapshotRequest { name };
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/snapshot", Self::machine_path(id)),
            )
            .query(&[("project", project)])
            .json(&body),
        )
        .await
    }

    pub async fn snapshots(
        &self,
        id: &str,
        project: Option<&str>,
    ) -> Result<ListResponse<Snapshot>, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(
                Method::GET,
                &format!("{}/snapshots", Self::machine_path(id)),
            )
            .query(&[("project", project)]),
        )
        .await
    }

    pub async fn restore_snapshot(
        &self,
        machine_id: &str,
        snapshot_id: &str,
        project: Option<&str>,
    ) -> Result<Snapshot, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/restore", Self::snapshot_path(machine_id, snapshot_id)),
            )
            .query(&[("project", project)]),
        )
        .await
    }

    pub async fn delete_snapshot(
        &self,
        machine_id: &str,
        snapshot_id: &str,
        project: Option<&str>,
    ) -> Result<(), Error> {
        let project = self.project(project)?;
        self.empty(
            self.request(
                Method::DELETE,
                &Self::snapshot_path(machine_id, snapshot_id),
            )
            .query(&[("project", project)]),
        )
        .await
    }

    fn golden_path(id: &str) -> String {
        format!("/api/v1/golden/{}", urlencoding::encode(id))
    }

    pub async fn golden_builds(&self) -> Result<ListResponse<GoldenBuild>, Error> {
        self.decode(self.request(Method::GET, "/api/v1/golden"))
            .await
    }

    pub async fn golden_build(&self, id: &str) -> Result<GoldenBuild, Error> {
        self.decode(self.request(Method::GET, &Self::golden_path(id)))
            .await
    }

    pub async fn start_golden(
        &self,
        request: &GoldenStartRequest,
        project: Option<&str>,
    ) -> Result<GoldenBuild, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(Method::POST, "/api/v1/golden")
                .query(&[("project", project)])
                .json(request),
        )
        .await
    }

    pub async fn bootstrap_golden(
        &self,
        id: &str,
        project: Option<&str>,
    ) -> Result<GoldenBuild, Error> {
        let project = self.project(project)?;
        self.decode(
            self.request(
                Method::POST,
                &format!("{}/bootstrap", Self::golden_path(id)),
            )
            .query(&[("project", project)]),
        )
        .await
    }

    pub async fn golden_passport(&self, id: &str) -> Result<serde_json::Value, Error> {
        self.decode(self.request(Method::GET, &format!("{}/passport", Self::golden_path(id))))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalizes_slashes() {
        let client = Client::new(Config {
            base_url: "http://127.0.0.1:8080".into(),
            token: None,
            default_project: Some("default".into()),
            timeout: Duration::from_secs(1),
            allow_invalid_tls: false,
        })
        .unwrap();
        assert_eq!(
            client.endpoint("/api/v1/machines"),
            "http://127.0.0.1:8080/api/v1/machines"
        );
    }

    #[test]
    fn kryton_error_envelope_is_preserved() {
        let err = Client::decode_error(
            StatusCode::CONFLICT,
            r#"{"error":{"code":"CONFLICT","message":"machine exists","hint":"choose another name"}}"#.into(),
        );
        match err {
            Error::Upstream {
                status,
                code,
                message,
                hint,
            } => {
                assert_eq!(status, 409);
                assert_eq!(code.as_deref(), Some("CONFLICT"));
                assert_eq!(message, "machine exists");
                assert_eq!(hint.as_deref(), Some("choose another name"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// Serves a canned Kryton golden API on a random local port and records
    /// each request as "METHOD path?query".
    async fn golden_server() -> (Client, Seen) {
        use axum::{extract::Request, http::StatusCode, response::IntoResponse, Json, Router};
        let seen: Seen = Default::default();
        let log = seen.clone();
        let app = Router::new().fallback(move |req: Request| {
            let log = log.clone();
            async move {
                let line = format!("{} {}", req.method(), req.uri());
                log.lock().unwrap().push(line);
                let build = serde_json::json!({"id":"gb 1","imageId":"windows-11-pro","state":"ready"});
                match (req.method().as_str(), req.uri().path()) {
                    ("GET", "/api/v1/golden") => Json(serde_json::json!({"items":[build]})).into_response(),
                    ("POST", "/api/v1/golden") => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({"error":{"code":"unavailable","message":"golden image builder requires docker and /dev/kvm on the krytond host"}})),
                    )
                        .into_response(),
                    ("POST", "/api/v1/golden/gb%201/bootstrap") => (
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({"error":{"code":"conflict","message":"CDI bootstrap is already running for this build"}})),
                    )
                        .into_response(),
                    ("GET", "/api/v1/golden/gb%201/passport") => Json(serde_json::json!({"score":90})).into_response(),
                    ("GET", "/api/v1/golden/gb%201") => Json(build).into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::new(Config {
            base_url: format!("http://{addr}"),
            token: Some("t".into()),
            default_project: Some("default".into()),
            timeout: Duration::from_secs(5),
            allow_invalid_tls: false,
        })
        .unwrap();
        (client, seen)
    }

    #[tokio::test]
    async fn golden_reads_encode_ids() {
        let (client, seen) = golden_server().await;
        assert_eq!(client.golden_builds().await.unwrap().items.len(), 1);
        assert_eq!(
            client.golden_build("gb 1").await.unwrap().image_id,
            "windows-11-pro"
        );
        assert_eq!(client.golden_passport("gb 1").await.unwrap()["score"], 90);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "GET /api/v1/golden",
                "GET /api/v1/golden/gb%201",
                "GET /api/v1/golden/gb%201/passport"
            ]
        );
    }

    #[tokio::test]
    async fn golden_writes_send_project_and_keep_upstream_errors() {
        let (client, seen) = golden_server().await;
        let req = GoldenStartRequest {
            image_id: "windows-11-pro".into(),
            version: None,
            auto: true,
        };
        match client.start_golden(&req, Some("finance")).await {
            Err(Error::Upstream {
                status, message, ..
            }) => {
                assert_eq!(status, 503);
                assert!(message.contains("/dev/kvm"), "{message}");
            }
            other => panic!("unexpected: {other:?}"),
        }
        match client.bootstrap_golden("gb 1", None).await {
            Err(Error::Upstream { status, .. }) => assert_eq!(status, 409),
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "POST /api/v1/golden?project=finance",
                "POST /api/v1/golden/gb%201/bootstrap?project=default"
            ]
        );
    }

    #[tokio::test]
    async fn golden_writes_require_a_project() {
        let client = Client::new(Config {
            base_url: "http://127.0.0.1:9".into(),
            token: None,
            default_project: None,
            timeout: Duration::from_secs(1),
            allow_invalid_tls: false,
        })
        .unwrap();
        assert!(matches!(
            client.bootstrap_golden("x", None).await,
            Err(Error::MissingProject)
        ));
    }
}
