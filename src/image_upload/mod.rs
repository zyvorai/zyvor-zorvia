// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Disk image upload through the CDI upload proxy.
//!
//! Flow: sniff the first bytes of the image (format must be recognised and agree
//! with any declared format), create a CDI `DataVolume` with an `upload` source,
//! wait for it to reach `UploadReady`, mint a short-lived token with an
//! `UploadTokenRequest`, then stream the body to the proxy while counting bytes
//! (hard size cap) and hashing (optional expected SHA-256). Nothing is buffered
//! beyond the first ~32 KiB used for format detection. If the cap or the checksum
//! fails, the request body is aborted with an error (so CDI does not finalise the
//! upload) and the DataVolume is deleted.
//!
//! The upload token is never logged, returned or stored. Conversion to raw is done
//! by CDI itself inside the upload server; Zorvia only detects and validates the
//! format and never shells out to `qemu-img`.

use crate::disk_conversion::{validate_image_header, DiskFormat, DETECT_PREFIX_LEN};
use crate::kube::KubeClient;
use anyhow::{anyhow, bail, Context, Result};
use axum::body::Bytes;
use futures_util::stream::{BoxStream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ENV_PROXY_URL: &str = "ZORVIA_CDI_UPLOAD_PROXY_URL";
pub const ENV_PROXY_CA_FILE: &str = "ZORVIA_CDI_UPLOAD_PROXY_CA_FILE";
pub const ENV_MAX_BYTES: &str = "ZORVIA_IMAGE_UPLOAD_MAX_BYTES";
/// Default hard cap on bytes sent to the proxy (20 GiB).
pub const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const UPLOAD_READY_TIMEOUT_SECS: u64 = 120;

pub type ByteStream = BoxStream<'static, std::io::Result<Bytes>>;

/// Short-lived CDI upload token. `Debug` is redacted; it has no `Serialize`.
pub struct UploadToken(String);

impl std::fmt::Debug for UploadToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UploadToken(<redacted>)")
    }
}

/// Where and how to reach the CDI upload proxy.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    upload_url: reqwest::Url,
    ca_pem: Option<Vec<u8>>,
}

impl ProxyConfig {
    /// `base` must be `https://...`; plain `http://` is accepted only for a
    /// loopback host (local test servers). TLS verification is never disabled:
    /// supply the CDI CA as PEM if the proxy certificate is not publicly trusted.
    pub fn new(base: &str, ca_pem: Option<Vec<u8>>) -> Result<Self> {
        let mut url = reqwest::Url::parse(base.trim())
            .map_err(|e| anyhow!("invalid upload proxy URL: {e}"))?;
        let loopback = url.host_str().is_some_and(|h| {
            let h = h.trim_start_matches('[').trim_end_matches(']');
            h.eq_ignore_ascii_case("localhost")
                || h.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => bail!("upload proxy URL must be https:// (http:// is allowed only for loopback)"),
        }
        if !url.username().is_empty() || url.password().is_some() {
            bail!("upload proxy URL must not contain credentials");
        }
        url.set_query(None);
        url.set_fragment(None);
        if !url.path().ends_with('/') {
            let p = format!("{}/", url.path());
            url.set_path(&p);
        }
        let upload_url = url
            .join("v1beta1/upload")
            .map_err(|e| anyhow!("invalid upload proxy URL: {e}"))?;
        Ok(Self { upload_url, ca_pem })
    }

    /// Read `ZORVIA_CDI_UPLOAD_PROXY_URL` (+ optional `..._CA_FILE`).
    pub fn from_env() -> Result<Self> {
        let base = std::env::var(ENV_PROXY_URL).map_err(|_| {
            anyhow!(
                "{ENV_PROXY_URL} is not set (expose the CDI upload proxy and set its https URL)"
            )
        })?;
        let ca = match std::env::var(ENV_PROXY_CA_FILE) {
            Ok(p) if !p.trim().is_empty() => Some(
                std::fs::read(&p).with_context(|| format!("cannot read {ENV_PROXY_CA_FILE}"))?,
            ),
            _ => None,
        };
        Self::new(&base, ca)
    }

    pub fn upload_url(&self) -> &str {
        self.upload_url.as_str()
    }
}

/// Configured size cap (`ZORVIA_IMAGE_UPLOAD_MAX_BYTES`, default 20 GiB).
pub fn max_bytes_from_env() -> u64 {
    std::env::var(ENV_MAX_BYTES)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_BYTES)
}

#[derive(Debug, Clone)]
pub struct UploadRequest {
    pub name: String,
    pub namespace: String,
    /// PVC size, Kubernetes quantity (e.g. `20Gi`).
    pub size: String,
    pub storage_class: Option<String>,
    pub declared_format: Option<DiskFormat>,
    pub expected_sha256: Option<String>,
    pub max_bytes: u64,
}

fn validate_dns_label(kind: &str, v: &str) -> Result<()> {
    crate::kube::lifecycle::validate_k8s_name(kind, v)?;
    if v.starts_with('-')
        || v.ends_with('-')
        || !v
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("{kind} must be a lowercase DNS label (a-z, 0-9, '-')");
    }
    Ok(())
}

impl UploadRequest {
    /// Validate and normalise (lowercases the expected checksum).
    pub fn validate(&mut self) -> Result<()> {
        validate_dns_label("name", &self.name)?;
        validate_dns_label("namespace", &self.namespace)?;
        if let Some(sc) = &self.storage_class {
            validate_dns_label("storage class", sc)?;
        }
        if !matches!(crate::storage::parse_size_to_bytes(&self.size), Some(b) if b > 0) {
            bail!("invalid Kubernetes storage size '{}'", self.size);
        }
        if let Some(h) = &mut self.expected_sha256 {
            h.make_ascii_lowercase();
            if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("sha256 must be 64 hex characters");
            }
        }
        if self.max_bytes == 0 {
            bail!("max_bytes must be greater than zero");
        }
        Ok(())
    }
}

/// CDI `DataVolume` with an `upload` source.
pub fn upload_data_volume_manifest(req: &UploadRequest, format: &DiskFormat) -> Value {
    let mut annotations = serde_json::Map::new();
    // Without this a WaitForFirstConsumer StorageClass never schedules the upload pod.
    annotations.insert(
        "cdi.kubevirt.io/storage.bind.immediate.requested".into(),
        json!("true"),
    );
    annotations.insert("zorvia.io/image-format".into(), json!(format.name()));
    if let Some(h) = &req.expected_sha256 {
        annotations.insert("zorvia.io/image-sha256".into(), json!(h));
    }
    let mut storage = serde_json::Map::new();
    storage.insert(
        "resources".into(),
        json!({ "requests": { "storage": req.size } }),
    );
    if let Some(sc) = &req.storage_class {
        storage.insert("storageClassName".into(), json!(sc));
    }
    json!({
        "apiVersion": "cdi.kubevirt.io/v1beta1",
        "kind": "DataVolume",
        "metadata": {
            "name": req.name,
            "namespace": req.namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "zorvia",
                "zorvia.io/uploaded-image": "true"
            },
            "annotations": Value::Object(annotations)
        },
        "spec": { "source": { "upload": {} }, "storage": Value::Object(storage) }
    })
}

/// Body of the `UploadTokenRequest` for the PVC backing the DataVolume.
pub fn upload_token_request_manifest(namespace: &str, pvc: &str) -> Value {
    json!({
        "apiVersion": "upload.cdi.kubevirt.io/v1beta1",
        "kind": "UploadTokenRequest",
        "metadata": { "name": pvc, "namespace": namespace },
        "spec": { "pvcName": pvc }
    })
}

fn token_from_response(v: &Value) -> Result<UploadToken> {
    v.pointer("/status/token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(|t| UploadToken(t.to_string()))
        .ok_or_else(|| anyhow!("UploadTokenRequest response carried no token"))
}

async fn kube_json(
    kube: &KubeClient,
    method: http::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let mut b = http::Request::builder().method(method).uri(path);
    let payload = match body {
        Some(v) => {
            b = b.header(http::header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(v)?
        }
        None => Vec::new(),
    };
    let req = b
        .body(payload)
        .map_err(|e| anyhow!("failed to build request: {e}"))?;
    Ok(kube.client().request::<Value>(req).await?)
}

async fn request_upload_token(kube: &KubeClient, ns: &str, pvc: &str) -> Result<UploadToken> {
    let path = format!("/apis/upload.cdi.kubevirt.io/v1beta1/namespaces/{ns}/uploadtokenrequests");
    let resp = kube_json(
        kube,
        http::Method::POST,
        &path,
        Some(&upload_token_request_manifest(ns, pvc)),
    )
    .await?;
    token_from_response(&resp)
}

async fn delete_data_volume(kube: &KubeClient, ns: &str, name: &str) {
    let path = format!("/apis/cdi.kubevirt.io/v1beta1/namespaces/{ns}/datavolumes/{name}");
    if let Err(e) = kube_json(kube, http::Method::DELETE, &path, None).await {
        log::warn!("could not delete failed upload DataVolume {ns}/{name}: {e}");
    }
}

async fn wait_upload_ready(kube: &KubeClient, ns: &str, name: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(UPLOAD_READY_TIMEOUT_SECS);
    loop {
        let phase = kube
            .get_data_volume(ns, name)
            .await
            .ok()
            .and_then(|o| crate::kube::cdi::data_volume_phase_from_object(&o));
        match phase.as_deref() {
            Some("UploadReady") => return Ok(()),
            Some("Failed") => bail!("DataVolume '{name}' failed before the upload server was ready"),
            _ if Instant::now() >= deadline => bail!(
                "DataVolume '{name}' did not reach UploadReady within {UPLOAD_READY_TIMEOUT_SECS}s (last phase: {})",
                phase.as_deref().unwrap_or("unknown")
            ),
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

/// Read just enough of `stream` to detect and validate the format, returning a
/// stream that replays the held chunks before the rest. Memory held is bounded by
/// the chunks needed to cover [`DETECT_PREFIX_LEN`].
pub async fn sniff_stream(
    mut stream: ByteStream,
    declared: Option<&DiskFormat>,
) -> Result<(DiskFormat, ByteStream)> {
    let mut held: Vec<Bytes> = Vec::new();
    let mut len = 0usize;
    while len < DETECT_PREFIX_LEN {
        match stream.next().await {
            Some(Ok(c)) => {
                len += c.len();
                held.push(c);
            }
            Some(Err(e)) => bail!("failed reading image: {e}"),
            None => break,
        }
    }
    if len == 0 {
        bail!("image is empty");
    }
    let mut prefix = Vec::with_capacity(DETECT_PREFIX_LEN.min(len));
    for c in &held {
        let room = DETECT_PREFIX_LEN - prefix.len();
        prefix.extend_from_slice(&c[..c.len().min(room)]);
        if prefix.len() >= DETECT_PREFIX_LEN {
            break;
        }
    }
    let format = validate_image_header(&prefix, declared).map_err(|e| anyhow!(e))?;
    let replay = futures_util::stream::iter(held.into_iter().map(Ok)).chain(stream);
    Ok((format, replay.boxed()))
}

/// Stream a file in 256 KiB chunks.
pub fn file_stream(file: tokio::fs::File) -> ByteStream {
    use tokio::io::AsyncReadExt;
    futures_util::stream::unfold(file, |mut f| async move {
        let mut buf = vec![0u8; 256 * 1024];
        match f.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), f))
            }
            Err(e) => Some((Err(e), f)),
        }
    })
    .boxed()
}

struct Tally {
    hasher: Sha256,
    bytes: u64,
    failure: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadOutcome {
    pub bytes: u64,
    pub sha256: String,
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// POST `body` to the proxy with the bearer token, enforcing `max_bytes` and the
/// optional `expected_sha256` while streaming. A cap or checksum failure aborts the
/// request body with an error rather than ending it cleanly.
pub async fn send_to_proxy(
    cfg: &ProxyConfig,
    token: &UploadToken,
    body: ByteStream,
    content_length: Option<u64>,
    max_bytes: u64,
    expected_sha256: Option<String>,
) -> Result<UploadOutcome> {
    let tally = Arc::new(Mutex::new(Tally {
        hasher: Sha256::new(),
        bytes: 0,
        failure: None,
    }));
    let t_map = tally.clone();
    let counted = body.map(move |item| {
        let mut t = t_map.lock().unwrap_or_else(|e| e.into_inner());
        match item {
            Ok(chunk) => {
                t.bytes += chunk.len() as u64;
                if t.bytes > max_bytes {
                    let m = format!("image exceeds the {max_bytes}-byte upload limit");
                    t.failure = Some(m.clone());
                    return Err(std::io::Error::other(m));
                }
                t.hasher.update(&chunk);
                Ok(chunk)
            }
            Err(e) => {
                t.failure = Some(format!("image stream failed: {e}"));
                Err(e)
            }
        }
    });
    let t_end = tally.clone();
    let tail = futures_util::stream::once(async move {
        let mut t = t_end.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(exp) = expected_sha256 {
            let got = hex(&t.hasher.clone().finalize());
            if got != exp {
                let m = format!("sha256 mismatch: expected {exp}, got {got}");
                t.failure = Some(m.clone());
                return Err(std::io::Error::other(m));
            }
        }
        Ok(Bytes::new())
    });

    let mut cb = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10));
    if let Some(pem) = &cfg.ca_pem {
        let cert = reqwest::Certificate::from_pem(pem)
            .map_err(|e| anyhow!("invalid upload proxy CA PEM: {e}"))?;
        cb = cb.add_root_certificate(cert);
    }
    let client = cb.build().context("failed to build HTTP client")?;
    let mut rb = client
        .post(cfg.upload_url.clone())
        .bearer_auth(&token.0)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(reqwest::Body::wrap_stream(counted.chain(tail)));
    if let Some(n) = content_length {
        rb = rb.header(reqwest::header::CONTENT_LENGTH, n);
    }
    let sent = rb.send().await;
    let failure = tally
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .failure
        .clone();
    if let Some(f) = failure {
        bail!("{f}");
    }
    let resp = sent.map_err(|e| anyhow!("upload request failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(200).collect();
        bail!("upload proxy returned {status}: {snippet}");
    }
    let t = tally.lock().unwrap_or_else(|e| e.into_inner());
    Ok(UploadOutcome {
        bytes: t.bytes,
        sha256: hex(&t.hasher.clone().finalize()),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadReport {
    pub name: String,
    pub namespace: String,
    pub format: String,
    pub bytes: u64,
    pub sha256: String,
    /// CDI converts everything except raw and ISO to raw inside the upload server.
    pub converted_to_raw_by_cdi: bool,
    pub phase: Option<String>,
}

/// Full upload: validate, sniff, create DataVolume, wait, token, stream.
/// `content_length` is the declared body size when known (used for early limits).
pub async fn upload_image(
    kube: &KubeClient,
    mut req: UploadRequest,
    cfg: &ProxyConfig,
    body: ByteStream,
    content_length: Option<u64>,
) -> Result<UploadReport> {
    req.validate()?;
    if let Some(n) = content_length {
        if n > req.max_bytes {
            bail!(
                "image is {n} bytes, over the {}-byte upload limit",
                req.max_bytes
            );
        }
    }
    let (format, body) = sniff_stream(body, req.declared_format.as_ref()).await?;
    if let (Some(n), DiskFormat::Raw | DiskFormat::Iso) = (content_length, &format) {
        if let Some(cap) = crate::storage::parse_size_to_bytes(&req.size) {
            if n > cap {
                bail!(
                    "{} image is {n} bytes, larger than the requested size {}",
                    format.name(),
                    req.size
                );
            }
        }
    }

    kube.apply_data_volume(&req.namespace, &upload_data_volume_manifest(&req, &format))
        .await
        .context("failed to create upload DataVolume")?;

    let result = async {
        wait_upload_ready(kube, &req.namespace, &req.name).await?;
        let token = request_upload_token(kube, &req.namespace, &req.name).await?;
        send_to_proxy(
            cfg,
            &token,
            body,
            content_length,
            req.max_bytes,
            req.expected_sha256.clone(),
        )
        .await
    }
    .await;

    match result {
        Ok(outcome) => {
            let phase = kube
                .get_data_volume(&req.namespace, &req.name)
                .await
                .ok()
                .and_then(|o| crate::kube::cdi::data_volume_phase_from_object(&o));
            Ok(UploadReport {
                name: req.name,
                namespace: req.namespace,
                converted_to_raw_by_cdi: !matches!(format, DiskFormat::Raw | DiskFormat::Iso),
                format: format.name().to_string(),
                bytes: outcome.bytes,
                sha256: outcome.sha256,
                phase,
            })
        }
        Err(e) => {
            delete_data_volume(kube, &req.namespace, &req.name).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Router};

    type Seen = Arc<Mutex<Vec<(String, u64, String)>>>;

    /// Local stand-in for the CDI upload proxy: records auth header, byte count
    /// and sha256 of each completed body.
    async fn spawn_proxy(status: u16) -> (String, Seen) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        async fn h(
            State((seen, status)): State<(Seen, u16)>,
            headers: HeaderMap,
            body: axum::body::Body,
        ) -> (axum::http::StatusCode, &'static str) {
            let mut s = body.into_data_stream();
            let mut hasher = Sha256::new();
            let mut n = 0u64;
            while let Some(c) = s.next().await {
                match c {
                    Ok(c) => {
                        n += c.len() as u64;
                        hasher.update(&c);
                    }
                    Err(_) => return (axum::http::StatusCode::BAD_REQUEST, "aborted"),
                }
            }
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            seen.lock()
                .unwrap()
                .push((auth, n, hex(&hasher.finalize())));
            (axum::http::StatusCode::from_u16(status).unwrap(), "done")
        }
        let app = Router::new()
            .route("/v1beta1/upload", post(h))
            .with_state((seen.clone(), status));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    fn chunks(data: Vec<u8>, size: usize) -> ByteStream {
        let parts: Vec<std::io::Result<Bytes>> = data
            .chunks(size)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        futures_util::stream::iter(parts).boxed()
    }

    fn qcow2_fixture(len: usize) -> Vec<u8> {
        let mut v = vec![7u8; len];
        v[..4].copy_from_slice(b"QFI\xfb");
        v
    }

    fn sha(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    fn token() -> UploadToken {
        UploadToken("secret-token".into())
    }

    #[test]
    fn proxy_url_rules() {
        assert!(ProxyConfig::new("https://cdi.example.com", None).is_ok());
        assert_eq!(
            ProxyConfig::new("https://cdi.example.com/base?x=1", None)
                .unwrap()
                .upload_url(),
            "https://cdi.example.com/base/v1beta1/upload"
        );
        assert!(ProxyConfig::new("http://127.0.0.1:9", None).is_ok());
        assert!(ProxyConfig::new("http://cdi.example.com", None).is_err());
        assert!(ProxyConfig::new("https://u:p@cdi.example.com", None).is_err());
        assert!(ProxyConfig::new("ftp://x", None).is_err());
        assert!(ProxyConfig::new("not a url", None).is_err());
    }

    #[test]
    fn request_validation() {
        let base = UploadRequest {
            name: "ubuntu-24".into(),
            namespace: "default".into(),
            size: "20Gi".into(),
            storage_class: None,
            declared_format: None,
            expected_sha256: Some("A".repeat(64)),
            max_bytes: 10,
        };
        let mut ok = base.clone();
        ok.validate().unwrap();
        assert_eq!(ok.expected_sha256.unwrap(), "a".repeat(64));
        for mutate in [
            |r: &mut UploadRequest| r.name = "Bad_Name".into(),
            |r: &mut UploadRequest| r.name = "../x".into(),
            |r: &mut UploadRequest| r.namespace = "a b".into(),
            |r: &mut UploadRequest| r.size = "lots".into(),
            |r: &mut UploadRequest| r.expected_sha256 = Some("abc".into()),
            |r: &mut UploadRequest| r.storage_class = Some("-x".into()),
            |r: &mut UploadRequest| r.max_bytes = 0,
        ] {
            let mut r = base.clone();
            mutate(&mut r);
            assert!(r.validate().is_err());
        }
    }

    #[test]
    fn manifests_have_expected_shape() {
        let mut r = UploadRequest {
            name: "img".into(),
            namespace: "vms".into(),
            size: "10Gi".into(),
            storage_class: Some("ceph-rbd".into()),
            declared_format: None,
            expected_sha256: None,
            max_bytes: 1,
        };
        r.validate().unwrap();
        let dv = upload_data_volume_manifest(&r, &DiskFormat::Qcow2);
        assert_eq!(dv["spec"]["source"]["upload"], json!({}));
        assert_eq!(dv["spec"]["storage"]["storageClassName"], "ceph-rbd");
        assert_eq!(
            dv["spec"]["storage"]["resources"]["requests"]["storage"],
            "10Gi"
        );
        assert_eq!(
            dv["metadata"]["annotations"]["zorvia.io/image-format"],
            "qcow2"
        );
        let tr = upload_token_request_manifest("vms", "img");
        assert_eq!(tr["spec"]["pvcName"], "img");
    }

    #[test]
    fn token_parsing_and_redaction() {
        let t = token_from_response(&json!({"status": {"token": "abc"}})).unwrap();
        assert_eq!(t.0, "abc");
        assert!(!format!("{t:?}").contains("abc"));
        assert!(token_from_response(&json!({"status": {}})).is_err());
        assert!(token_from_response(&json!({"status": {"token": ""}})).is_err());
    }

    #[tokio::test]
    async fn sniff_rejects_garbage_and_replays_stream() {
        let r = sniff_stream(chunks(vec![0u8; 100_000], 4096), None).await;
        assert!(r.is_err());
        assert!(sniff_stream(chunks(Vec::new(), 10), Some(&DiskFormat::Raw))
            .await
            .is_err());
        let data = qcow2_fixture(100_000);
        let (fmt, s) = sniff_stream(chunks(data.clone(), 1000), None)
            .await
            .unwrap();
        assert_eq!(fmt, DiskFormat::Qcow2);
        let collected: Vec<u8> = s
            .map(|c| c.unwrap().to_vec())
            .collect::<Vec<_>>()
            .await
            .concat();
        assert_eq!(collected, data);
        // declared mismatch
        assert!(sniff_stream(chunks(data, 1000), Some(&DiskFormat::Vmdk))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn streams_to_proxy_with_token_and_checksum() {
        let (url, seen) = spawn_proxy(200).await;
        let cfg = ProxyConfig::new(&url, None).unwrap();
        let data = qcow2_fixture(1_000_000);
        let (_, s) = sniff_stream(chunks(data.clone(), 65536), None)
            .await
            .unwrap();
        let out = send_to_proxy(
            &cfg,
            &token(),
            s,
            Some(data.len() as u64),
            2_000_000,
            Some(sha(&data)),
        )
        .await
        .unwrap();
        assert_eq!(out.bytes, 1_000_000);
        assert_eq!(out.sha256, sha(&data));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "Bearer secret-token");
        assert_eq!(seen[0].1, 1_000_000);
        assert_eq!(seen[0].2, sha(&data));
    }

    #[tokio::test]
    async fn size_cap_aborts_the_request() {
        let (url, seen) = spawn_proxy(200).await;
        let cfg = ProxyConfig::new(&url, None).unwrap();
        let data = qcow2_fixture(500_000);
        let (_, s) = sniff_stream(chunks(data, 65536), None).await.unwrap();
        let err = send_to_proxy(&cfg, &token(), s, None, 100_000, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("upload limit"), "{err}");
        // The proxy never saw a completed body.
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn checksum_mismatch_aborts_the_request() {
        let (url, seen) = spawn_proxy(200).await;
        let cfg = ProxyConfig::new(&url, None).unwrap();
        let data = qcow2_fixture(200_000);
        let (_, s) = sniff_stream(chunks(data, 65536), None).await.unwrap();
        let err = send_to_proxy(&cfg, &token(), s, None, 1_000_000, Some("0".repeat(64)))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("sha256 mismatch"), "{err}");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn proxy_error_status_is_reported_without_token() {
        let (url, _seen) = spawn_proxy(500).await;
        let cfg = ProxyConfig::new(&url, None).unwrap();
        let data = qcow2_fixture(10_000);
        let (_, s) = sniff_stream(chunks(data, 4096), None).await.unwrap();
        let err = send_to_proxy(&cfg, &token(), s, None, 1_000_000, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500"), "{err}");
        assert!(!err.contains("secret-token"));
    }

    #[tokio::test]
    async fn file_stream_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("disk.qcow2");
        let data = qcow2_fixture(700_000);
        std::fs::write(&p, &data).unwrap();
        let f = tokio::fs::File::open(&p).await.unwrap();
        let got: Vec<u8> = file_stream(f)
            .map(|c| c.unwrap().to_vec())
            .collect::<Vec<_>>()
            .await
            .concat();
        assert_eq!(got, data);
    }
}
