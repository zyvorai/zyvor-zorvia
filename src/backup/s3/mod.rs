// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Minimal S3-compatible object client for off-cluster backups: single and
//! multipart uploads, ranged/streamed reads, HEAD, and S3 Object Lock
//! retention headers. Path-style addressing (works with AWS S3, MinIO and
//! Ceph RGW). Requests are signed with SigV4 (`sigv4`).

pub mod sigv4;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use sigv4::{sha256_hex, Credentials};

/// Object Lock retention mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Governance,
    Compliance,
}

impl LockMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Governance => "GOVERNANCE",
            Self::Compliance => "COMPLIANCE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObjectLock {
    pub mode: LockMode,
    pub retain_until: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct S3Config {
    /// e.g. `https://s3.eu-west-1.amazonaws.com` or `http://minio:9000`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub credentials: Credentials,
    /// Send `x-amz-checksum-sha256` so the store verifies every upload.
    pub send_checksums: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub size: u64,
    pub etag: String,
    /// Base64 `x-amz-checksum-sha256`, when the store recorded one.
    pub checksum_sha256: Option<String>,
    pub retain_until: Option<String>,
    pub lock_mode: Option<String>,
}

/// A request ready to be sent: everything except the transport.
#[derive(Debug, Clone)]
pub struct PreparedRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

pub struct S3Client {
    cfg: S3Config,
    http: reqwest::Client,
}

fn b64_sha256(hex_digest: &str) -> String {
    let raw: Vec<u8> = (0..hex_digest.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex_digest[i..i + 2], 16).ok())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// Text between `<tag>` and `</tag>` in an S3 XML body.
fn xml_tag<'a>(body: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    Some(&body[start..end])
}

fn s3_error(status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    let code = xml_tag(body, "Code").unwrap_or("Unknown");
    let msg = xml_tag(body, "Message").unwrap_or("");
    anyhow!("S3 error {status}: {code} {msg}")
}

fn lock_headers(lock: &ObjectLock) -> Vec<(String, String)> {
    vec![
        (
            "x-amz-object-lock-mode".into(),
            lock.mode.as_str().to_string(),
        ),
        (
            "x-amz-object-lock-retain-until-date".into(),
            lock.retain_until.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        ),
    ]
}

impl S3Client {
    pub fn new(cfg: S3Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .context("build http client")?;
        Ok(Self { cfg, http })
    }

    fn host_and_base(&self) -> Result<(String, String)> {
        let ep = self.cfg.endpoint.trim_end_matches('/');
        let rest = ep
            .strip_prefix("https://")
            .or_else(|| ep.strip_prefix("http://"))
            .ok_or_else(|| anyhow!("endpoint must start with http:// or https://"))?;
        if rest.is_empty() || rest.contains('/') {
            bail!("endpoint must be scheme://host[:port] without a path");
        }
        // The signed `host` must be exactly what the HTTP client sends. Clients omit the
        // port from the Host header when it is the scheme's default (80 / 443), so sign
        // without it: signing `host:80` while sending `host` fails with 403
        // SignatureDoesNotMatch (seen live against Ceph RGW at `...svc:80`).
        let default_port = if ep.starts_with("https://") {
            "443"
        } else {
            "80"
        };
        let host = match rest.rsplit_once(':') {
            Some((h, port)) if port == default_port => h,
            _ => rest,
        };
        Ok((host.to_string(), ep.to_string()))
    }

    /// Build and sign a request for `key` (relative to the bucket).
    pub fn prepare(
        &self,
        method: &'static str,
        key: &str,
        query: &[(String, String)],
        extra_headers: &[(String, String)],
        payload_sha256_hex: &str,
        now: DateTime<Utc>,
    ) -> Result<PreparedRequest> {
        if key.is_empty() || key.starts_with('/') || key.contains("..") {
            bail!("invalid object key '{key}'");
        }
        let (host, base) = self.host_and_base()?;
        let path = format!("/{}/{}", self.cfg.bucket, key);
        let headers = sigv4::sign(
            &self.cfg.credentials,
            &self.cfg.region,
            method,
            &host,
            &path,
            query,
            extra_headers,
            payload_sha256_hex,
            now,
        );
        let mut url = format!("{base}{}", sigv4::uri_encode(&path, false));
        if !query.is_empty() {
            let mut q: Vec<String> = query
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}={}",
                        sigv4::uri_encode(k, true),
                        sigv4::uri_encode(v, true)
                    )
                })
                .collect();
            q.sort();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        Ok(PreparedRequest {
            method,
            url,
            headers,
        })
    }

    async fn send(
        &self,
        method: &'static str,
        key: &str,
        query: &[(String, String)],
        extra: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<reqwest::Response> {
        let payload = sha256_hex(&body);
        let mut extra = extra.to_vec();
        if self.cfg.send_checksums && !body.is_empty() && method == "PUT" {
            extra.push(("x-amz-checksum-sha256".into(), b64_sha256(&payload)));
        }
        let prepared = self.prepare(method, key, query, &extra, &payload, Utc::now())?;
        let m = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut req = self.http.request(m, &prepared.url);
        for (k, v) in &prepared.headers {
            req = req.header(k, v);
        }
        req = req.body(body);
        req.send().await.with_context(|| format!("{method} {key}"))
    }

    async fn ok_or_err(resp: reqwest::Response) -> Result<reqwest::Response> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(s3_error(status, &body))
    }

    fn etag(resp: &reqwest::Response) -> String {
        resp.headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_matches('"')
            .to_string()
    }

    /// Upload a small object (manifests, markers) in one request.
    pub async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
        lock: Option<&ObjectLock>,
    ) -> Result<String> {
        let mut extra = vec![("content-type".to_string(), content_type.to_string())];
        if let Some(l) = lock {
            extra.extend(lock_headers(l));
        }
        let resp = self.send("PUT", key, &[], &extra, body).await?;
        Ok(Self::etag(&Self::ok_or_err(resp).await?))
    }

    /// Start a multipart upload; returns the upload id.
    pub async fn create_multipart(&self, key: &str, lock: Option<&ObjectLock>) -> Result<String> {
        let mut extra = Vec::new();
        if let Some(l) = lock {
            extra.extend(lock_headers(l));
        }
        let q = [("uploads".to_string(), String::new())];
        let resp = self.send("POST", key, &q, &extra, Vec::new()).await?;
        let body = Self::ok_or_err(resp).await?.text().await?;
        xml_tag(&body, "UploadId")
            .map(String::from)
            .ok_or_else(|| anyhow!("CreateMultipartUpload response had no UploadId"))
    }

    /// Upload part `number` (1-based). Returns its ETag.
    pub async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        number: u32,
        body: Vec<u8>,
    ) -> Result<String> {
        let q = [
            ("partNumber".to_string(), number.to_string()),
            ("uploadId".to_string(), upload_id.to_string()),
        ];
        let resp = self.send("PUT", key, &q, &[], body).await?;
        Ok(Self::etag(&Self::ok_or_err(resp).await?))
    }

    pub async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<()> {
        let q = [("uploadId".to_string(), upload_id.to_string())];
        let resp = self
            .send(
                "POST",
                key,
                &q,
                &[("content-type".into(), "application/xml".into())],
                complete_body(parts).into_bytes(),
            )
            .await?;
        // S3 can return 200 with an <Error> body for a failed completion.
        let body = Self::ok_or_err(resp).await?.text().await?;
        if body.contains("<Error>") {
            bail!(
                "CompleteMultipartUpload failed: {} {}",
                xml_tag(&body, "Code").unwrap_or("Unknown"),
                xml_tag(&body, "Message").unwrap_or("")
            );
        }
        Ok(())
    }

    pub async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<()> {
        let q = [("uploadId".to_string(), upload_id.to_string())];
        let resp = self.send("DELETE", key, &q, &[], Vec::new()).await?;
        Self::ok_or_err(resp).await?;
        Ok(())
    }

    /// `None` when the object does not exist.
    pub async fn head_object(&self, key: &str) -> Result<Option<ObjectMeta>> {
        let extra = [("x-amz-checksum-mode".to_string(), "ENABLED".to_string())];
        let resp = self.send("HEAD", key, &[], &extra, Vec::new()).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = Self::ok_or_err(resp).await?;
        let h = resp.headers();
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(String::from);
        Ok(Some(ObjectMeta {
            size: get("content-length")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            etag: Self::etag(&resp),
            checksum_sha256: get("x-amz-checksum-sha256"),
            retain_until: get("x-amz-object-lock-retain-until-date"),
            lock_mode: get("x-amz-object-lock-mode"),
        }))
    }

    /// Whole-object read (manifests). Use `get_object_response` to stream
    /// disks.
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>> {
        let resp = self.get_object_response(key).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn get_object_response(&self, key: &str) -> Result<reqwest::Response> {
        let resp = self.send("GET", key, &[], &[], Vec::new()).await?;
        Self::ok_or_err(resp).await
    }
}

fn complete_body(parts: &[(u32, String)]) -> String {
    let mut s = String::from("<CompleteMultipartUpload>");
    for (n, etag) in parts {
        s.push_str(&format!(
            "<Part><PartNumber>{n}</PartNumber><ETag>\"{}\"</ETag></Part>",
            etag.trim_matches('"')
        ));
    }
    s.push_str("</CompleteMultipartUpload>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn client() -> S3Client {
        S3Client::new(S3Config {
            endpoint: "http://minio.local:9000".into(),
            region: "us-east-1".into(),
            bucket: "vm-backups".into(),
            credentials: Credentials {
                access_key: "AK".into(),
                secret_key: "SK".into(),
                session_token: None,
            },
            send_checksums: true,
        })
        .unwrap()
    }

    #[test]
    fn prepare_builds_path_style_url_with_sorted_query() {
        let now = Utc.with_ymd_and_hms(2026, 9, 30, 1, 2, 3).unwrap();
        let q = [
            ("uploadId".to_string(), "abc/def".to_string()),
            ("partNumber".to_string(), "2".to_string()),
        ];
        let r = client()
            .prepare(
                "PUT",
                "web-1/bk-1/disk 0.raw",
                &q,
                &[],
                &sha256_hex(b""),
                now,
            )
            .unwrap();
        assert_eq!(
            r.url,
            "http://minio.local:9000/vm-backups/web-1/bk-1/disk%200.raw?partNumber=2&uploadId=abc%2Fdef"
        );
        assert!(r.headers.iter().any(|(k, _)| k == "authorization"));
        assert!(r
            .headers
            .iter()
            .any(|(k, v)| k == "x-amz-date" && v == "20260930T010203Z"));
    }

    #[test]
    fn default_ports_are_not_part_of_the_signed_host() {
        let host = |endpoint: &str| {
            let mut c = client();
            c.cfg.endpoint = endpoint.into();
            c.host_and_base().unwrap().0
        };
        // Default ports are dropped (the HTTP client omits them from Host).
        assert_eq!(host("http://rgw.rook-ceph.svc:80"), "rgw.rook-ceph.svc");
        assert_eq!(host("https://s3.example.com:443"), "s3.example.com");
        // Non-default ports, and a 443 on plain http / 80 on https, are kept.
        assert_eq!(host("http://minio.local:9000"), "minio.local:9000");
        assert_eq!(host("http://minio.local:443"), "minio.local:443");
        assert_eq!(host("https://s3.example.com:80"), "s3.example.com:80");
        // No port at all, and an IPv6 literal.
        assert_eq!(host("http://rgw.local"), "rgw.local");
        assert_eq!(host("http://[::1]"), "[::1]");
        assert_eq!(host("http://[::1]:80"), "[::1]");
    }

    #[test]
    fn prepare_rejects_traversal_and_bad_endpoints() {
        let now = Utc::now();
        assert!(client().prepare("GET", "../x", &[], &[], "h", now).is_err());
        assert!(client().prepare("GET", "/x", &[], &[], "h", now).is_err());
        let mut bad = client();
        bad.cfg.endpoint = "minio.local".into();
        assert!(bad.prepare("GET", "k", &[], &[], "h", now).is_err());
        bad.cfg.endpoint = "http://minio.local/prefix".into();
        assert!(bad.prepare("GET", "k", &[], &[], "h", now).is_err());
    }

    #[test]
    fn lock_headers_use_iso8601_utc() {
        let lock = ObjectLock {
            mode: LockMode::Compliance,
            retain_until: Utc.with_ymd_and_hms(2027, 1, 2, 3, 4, 5).unwrap(),
        };
        let h = lock_headers(&lock);
        assert!(h.contains(&("x-amz-object-lock-mode".into(), "COMPLIANCE".into())));
        assert!(h.contains(&(
            "x-amz-object-lock-retain-until-date".into(),
            "2027-01-02T03:04:05Z".into()
        )));
    }

    #[test]
    fn checksum_header_value_is_base64_of_digest() {
        // sha256("") in base64
        assert_eq!(
            b64_sha256(&sha256_hex(b"")),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn parses_upload_id_and_errors_from_xml() {
        let ok = "<InitiateMultipartUploadResult><Bucket>b</Bucket><UploadId>VXBsb2Fk</UploadId></InitiateMultipartUploadResult>";
        assert_eq!(xml_tag(ok, "UploadId"), Some("VXBsb2Fk"));
        let err = s3_error(
            reqwest::StatusCode::FORBIDDEN,
            "<Error><Code>AccessDenied</Code><Message>nope</Message></Error>",
        );
        assert!(err.to_string().contains("AccessDenied nope"));
    }

    #[test]
    fn complete_body_lists_parts_in_order() {
        let body = complete_body(&[(1, "\"aa\"".into()), (2, "bb".into())]);
        assert_eq!(
            body,
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"aa\"</ETag></Part>\
             <Part><PartNumber>2</PartNumber><ETag>\"bb\"</ETag></Part></CompleteMultipartUpload>"
        );
    }
}
