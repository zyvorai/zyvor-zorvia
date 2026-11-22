// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Cloud KMS key wrapping (Experimental): AWS KMS and Google Cloud KMS over their
//! REST APIs with `reqwest`, no vendor SDK (see docs/VOLUME_ENCRYPTION.md for why).
//!
//! * The wrap is bound to its context: AWS `EncryptionContext`, GCP
//!   `additionalAuthenticatedData`. A wrapped key presented with another context is
//!   refused by the KMS.
//! * Credentials come only from the environment, a mounted file or the GCE/GKE
//!   metadata server. They are never accepted from request bodies and are never
//!   logged: none of these types implements `Debug`, and error text carries only the
//!   HTTP status and the service's error *code*, never a response body.
//! * https only (plain http is accepted for loopback, for tests and local mocks),
//!   10 s request timeout, 64 KiB response cap, no redirects, bounded retries with
//!   exponential backoff for throttling and 5xx.
//!
//! Nothing here has been run against a real cloud KMS.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_RESPONSE: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const GCE_METADATA_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

// ---------------------------------------------------------------------------
// Crypto helpers (SHA-256 comes from the `sha2` crate already in the tree)
// ---------------------------------------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(data));
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// HMAC-SHA256 (RFC 2104), checked against RFC 4231 vectors in the tests.
pub(crate) fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    inner.extend(k.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(msg);
    let ih = sha256(&inner);
    let mut outer = Vec::with_capacity(96);
    outer.extend(k.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&ih);
    sha256(&outer)
}

/// AWS Signature Version 4 parameters for one service and region.
pub(crate) struct SigV4<'a> {
    pub region: &'a str,
    pub service: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
}

impl SigV4<'_> {
    pub fn signing_key(&self, date: &str) -> [u8; 32] {
        let k = hmac_sha256(
            format!("AWS4{}", self.secret_key).as_bytes(),
            date.as_bytes(),
        );
        let k = hmac_sha256(&k, self.region.as_bytes());
        let k = hmac_sha256(&k, self.service.as_bytes());
        hmac_sha256(&k, b"aws4_request")
    }

    /// The `Authorization` header value. `headers` is every header to sign (names
    /// are lowercased here; include `host` and `x-amz-date`); `query` must already be
    /// canonical (KMS uses none); `path` is the canonical URI (`/`).
    pub fn authorization(
        &self,
        method: &str,
        path: &str,
        query: &str,
        headers: &[(String, String)],
        payload: &[u8],
        amz_date: &str,
    ) -> String {
        let date = &amz_date[..8];
        let mut hs: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_ascii_lowercase(),
                    v.split_whitespace().collect::<Vec<_>>().join(" "),
                )
            })
            .collect();
        hs.sort();
        let canon_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
        let signed = hs
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");
        let canonical = format!(
            "{method}\n{path}\n{query}\n{canon_headers}\n{signed}\n{}",
            hex(&sha256(payload))
        );
        let scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex(&sha256(canonical.as_bytes()))
        );
        let sig = hex(&hmac_sha256(&self.signing_key(date), to_sign.as_bytes()));
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={sig}",
            self.access_key
        )
    }
}

// ---------------------------------------------------------------------------
// HTTP plumbing shared by both providers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) struct Retry {
    pub max_attempts: u32,
    pub base: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base: Duration::from_millis(250),
        }
    }
}

/// An endpoint must be https; plain http only for loopback. No userinfo, query or path.
fn parse_endpoint(raw: &str, what: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| anyhow!("{what} is not a valid URL"))?;
    let host = url.host_str().unwrap_or("");
    let loopback = url.scheme() == "http" && matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    if url.scheme() != "https" && !loopback {
        bail!("{what} must be https:// (plain http is only accepted for loopback)");
    }
    if host.is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("{what} must be a bare scheme://host[:port] URL");
    }
    Ok(url)
}

fn clean_secret(v: &str, what: &str) -> Result<String> {
    let v = v.trim();
    if v.is_empty() || v.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        bail!("{what} is empty or contains whitespace/control characters");
    }
    Ok(v.to_string())
}

/// A service error code that is safe to show: short and made of identifier characters.
fn safe_code(s: &str) -> String {
    let c: String = s
        .rsplit('#')
        .next()
        .unwrap_or("")
        .chars()
        .take(64)
        .collect();
    if !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        c
    } else {
        "unknown".into()
    }
}

fn is_throttle_code(code: &str) -> bool {
    matches!(
        code,
        "ThrottlingException"
            | "Throttling"
            | "RequestLimitExceeded"
            | "LimitExceededException"
            | "KMSInternalException"
            | "RESOURCE_EXHAUSTED"
    )
}

/// Send a request (rebuilt each attempt, so signatures stay fresh) with retries and a
/// response cap. Returns the parsed JSON of a 2xx. Errors name the service, operation,
/// HTTP status and error code only, never a response body.
async fn call_json<F>(
    svc: &str,
    op: &str,
    retry: Retry,
    mut build: F,
    code_of: fn(&Value) -> String,
) -> Result<Value>
where
    F: FnMut(&reqwest::Client) -> Result<reqwest::RequestBuilder>,
{
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .build()?;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let last = attempt >= retry.max_attempts;
        let rb = build(&client)?;
        let outcome: Result<(u16, Vec<u8>), String> = async {
            let mut resp = rb
                .send()
                .await
                .map_err(|e| format!("request failed: {}", e.without_url()))?;
            let status = resp.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| format!("reading response failed: {}", e.without_url()))?
            {
                if body.len() + chunk.len() > MAX_RESPONSE {
                    return Err(format!("response larger than {MAX_RESPONSE} bytes"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok((status, body))
        }
        .await;
        match outcome {
            Ok((status, body)) if (200..300).contains(&status) => {
                return serde_json::from_slice(&body)
                    .map_err(|_| anyhow!("{svc} {op}: response is not JSON"));
            }
            Ok((status, body)) => {
                let code = code_of(&serde_json::from_slice(&body).unwrap_or(Value::Null));
                let retryable = status == 429
                    || (500..600).contains(&status)
                    || (status == 400 && is_throttle_code(&code));
                if retryable && !last {
                    backoff(retry, attempt).await;
                    continue;
                }
                bail!("{svc} {op} failed: HTTP {status} ({code})");
            }
            Err(e) => {
                // Transport failures retry; an oversized response does not.
                if !last && e.starts_with("request failed") {
                    backoff(retry, attempt).await;
                    continue;
                }
                bail!("{svc} {op}: {e}");
            }
        }
    }
}

async fn backoff(r: Retry, attempt: u32) {
    let base = r.base.as_millis() as u64;
    let exp = base.saturating_mul(1 << (attempt - 1).min(6));
    let jitter = if base == 0 {
        0
    } else {
        rand::rng().random_range(0..=base)
    };
    tokio::time::sleep(Duration::from_millis((exp + jitter).min(8_000))).await;
}

// ---------------------------------------------------------------------------
// AWS KMS
// ---------------------------------------------------------------------------

enum AwsCredSource {
    Env {
        access_key: String,
        secret_key: String,
        token: Option<String>,
    },
    /// A mounted AWS shared-credentials style file, re-read on every request so a
    /// rotated file is picked up.
    File { path: PathBuf, profile: String },
}

struct AwsCreds {
    access_key: String,
    secret_key: String,
    token: Option<String>,
}

fn parse_aws_credentials_file(text: &str, profile: &str) -> Result<AwsCreds> {
    let (mut ak, mut sk, mut tok) = (None, None, None);
    let mut in_profile = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_profile = sec.trim() == profile;
            continue;
        }
        if !in_profile {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().to_string();
            match k.trim().to_ascii_lowercase().as_str() {
                "aws_access_key_id" => ak = Some(v),
                "aws_secret_access_key" => sk = Some(v),
                "aws_session_token" => tok = Some(v),
                _ => {}
            }
        }
    }
    match (ak, sk) {
        (Some(a), Some(s)) => Ok(AwsCreds {
            access_key: clean_secret(&a, "the AWS access key id")?,
            secret_key: clean_secret(&s, "the AWS secret access key")?,
            token: tok
                .map(|t| clean_secret(&t, "the AWS session token"))
                .transpose()?,
        }),
        _ => bail!("the AWS credentials file has no complete '{profile}' profile"),
    }
}

pub struct AwsKms {
    region: String,
    key_id: String,
    endpoint: reqwest::Url,
    creds: AwsCredSource,
    retry: Retry,
}

fn valid_aws_key_id(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 2048
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'/'))
}

impl AwsKms {
    fn creds(&self) -> Result<AwsCreds> {
        match &self.creds {
            AwsCredSource::Env {
                access_key,
                secret_key,
                token,
            } => Ok(AwsCreds {
                access_key: access_key.clone(),
                secret_key: secret_key.clone(),
                token: token.clone(),
            }),
            AwsCredSource::File { path, profile } => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("read AWS credentials file {}", path.display()))?;
                parse_aws_credentials_file(&text, profile)
            }
        }
    }

    fn host(&self) -> String {
        let h = self.endpoint.host_str().unwrap_or("");
        match self.endpoint.port() {
            Some(p) => format!("{h}:{p}"),
            None => h.to_string(),
        }
    }

    async fn call(&self, target: &str, body: Value) -> Result<Value> {
        let payload = serde_json::to_vec(&body)?;
        let op = target.rsplit('.').next().unwrap_or(target).to_string();
        call_json(
            "AWS KMS",
            &op,
            self.retry,
            |client| {
                let creds = self.creds()?;
                let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
                let mut headers = vec![
                    (
                        "content-type".to_string(),
                        "application/x-amz-json-1.1".to_string(),
                    ),
                    ("host".to_string(), self.host()),
                    ("x-amz-date".to_string(), amz_date.clone()),
                    ("x-amz-target".to_string(), target.to_string()),
                ];
                if let Some(t) = &creds.token {
                    headers.push(("x-amz-security-token".to_string(), t.clone()));
                }
                let auth = SigV4 {
                    region: &self.region,
                    service: "kms",
                    access_key: &creds.access_key,
                    secret_key: &creds.secret_key,
                }
                .authorization("POST", "/", "", &headers, &payload, &amz_date);
                let mut rb = client
                    .post(self.endpoint.clone())
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-date", &amz_date)
                    .header("x-amz-target", target)
                    .header("authorization", auth)
                    .body(payload.clone());
                if let Some(t) = &creds.token {
                    rb = rb.header("x-amz-security-token", t);
                }
                Ok(rb)
            },
            |v| safe_code(v["__type"].as_str().unwrap_or("")),
        )
        .await
    }

    fn context(purpose: &str, owner: &str) -> Value {
        json!({"zorvia:purpose": purpose, "zorvia:owner": owner})
    }

    fn seal(&self, purpose: &str, owner: &str, secret: &str) -> Result<String> {
        let v = crate::keys::block_on(self.call(
            "TrentService.Encrypt",
            json!({
                "KeyId": self.key_id,
                "Plaintext": B64.encode(secret),
                "EncryptionContext": Self::context(purpose, owner),
            }),
        ))?;
        v["CiphertextBlob"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("AWS KMS returned no ciphertext"))
    }

    fn open(&self, purpose: &str, owner: &str, blob: &str) -> Result<String> {
        let v = crate::keys::block_on(self.call(
            "TrentService.Decrypt",
            json!({
                "KeyId": self.key_id,
                "CiphertextBlob": blob,
                "EncryptionContext": Self::context(purpose, owner),
            }),
        ))?;
        let raw = B64
            .decode(
                v["Plaintext"]
                    .as_str()
                    .ok_or_else(|| anyhow!("AWS KMS returned no plaintext"))?,
            )
            .map_err(|_| anyhow!("AWS KMS plaintext is not base64"))?;
        String::from_utf8(raw).map_err(|_| anyhow!("AWS KMS plaintext is not UTF-8"))
    }
}

// ---------------------------------------------------------------------------
// Google Cloud KMS
// ---------------------------------------------------------------------------

enum GcpTokenSource {
    /// `ZORVIA_GCP_ACCESS_TOKEN`: fixed (short-lived tokens need a restart; prefer the others).
    Static(String),
    /// A mounted file holding an OAuth2 access token, re-read on every request.
    File(PathBuf),
    /// The GCE/GKE metadata server (workload identity). Plain http by design: the
    /// metadata server is link-local and has no TLS.
    Metadata(reqwest::Url),
}

pub struct GcpKms {
    endpoint: reqwest::Url,
    key: String,
    token: GcpTokenSource,
    cache: Mutex<Option<(String, Instant)>>,
    retry: Retry,
}

fn valid_gcp_key(k: &str) -> bool {
    let p: Vec<&str> = k.split('/').collect();
    let ok = |s: &str, extra: &[u8]| {
        !s.is_empty()
            && s.len() <= 63
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || extra.contains(&b))
    };
    p.len() == 8
        && p[0] == "projects"
        && p[2] == "locations"
        && p[4] == "keyRings"
        && p[6] == "cryptoKeys"
        && ok(p[1], b":.")
        && ok(p[3], b"")
        && ok(p[5], b"")
        && ok(p[7], b"")
}

impl GcpKms {
    async fn token(&self) -> Result<String> {
        match &self.token {
            GcpTokenSource::Static(t) => Ok(t.clone()),
            GcpTokenSource::File(p) => {
                let t = std::fs::read_to_string(p)
                    .with_context(|| format!("read GCP token file {}", p.display()))?;
                clean_secret(&t, "the GCP access token")
            }
            GcpTokenSource::Metadata(url) => {
                if let Some((t, exp)) = &*self.cache.lock().unwrap() {
                    if Instant::now() < *exp {
                        return Ok(t.clone());
                    }
                }
                let v = call_json(
                    "GCP metadata",
                    "token",
                    self.retry,
                    |c| Ok(c.get(url.clone()).header("Metadata-Flavor", "Google")),
                    |_| "error".into(),
                )
                .await?;
                let t = clean_secret(
                    v["access_token"].as_str().unwrap_or(""),
                    "the GCP access token",
                )?;
                let ttl = v["expires_in"].as_u64().unwrap_or(0).saturating_sub(60);
                *self.cache.lock().unwrap() =
                    Some((t.clone(), Instant::now() + Duration::from_secs(ttl)));
                Ok(t)
            }
        }
    }

    async fn call(&self, op: &str, body: Value) -> Result<Value> {
        let token = self.token().await?;
        let url = format!(
            "{}/v1/{}:{op}",
            self.endpoint.as_str().trim_end_matches('/'),
            self.key
        );
        call_json(
            "GCP KMS",
            op,
            self.retry,
            |client| Ok(client.post(&url).bearer_auth(&token).json(&body)),
            |v| safe_code(v["error"]["status"].as_str().unwrap_or("")),
        )
        .await
    }

    fn aad(purpose: &str, owner: &str) -> String {
        B64.encode(format!("zorvia:{purpose}:{owner}"))
    }

    fn seal(&self, purpose: &str, owner: &str, secret: &str) -> Result<String> {
        let v = crate::keys::block_on(self.call(
            "encrypt",
            json!({
                "plaintext": B64.encode(secret),
                "additionalAuthenticatedData": Self::aad(purpose, owner),
            }),
        ))?;
        v["ciphertext"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("GCP KMS returned no ciphertext"))
    }

    fn open(&self, purpose: &str, owner: &str, ct: &str) -> Result<String> {
        let v = crate::keys::block_on(self.call(
            "decrypt",
            json!({
                "ciphertext": ct,
                "additionalAuthenticatedData": Self::aad(purpose, owner),
            }),
        ))?;
        let raw = B64
            .decode(
                v["plaintext"]
                    .as_str()
                    .ok_or_else(|| anyhow!("GCP KMS returned no plaintext"))?,
            )
            .map_err(|_| anyhow!("GCP KMS plaintext is not base64"))?;
        String::from_utf8(raw).map_err(|_| anyhow!("GCP KMS plaintext is not UTF-8"))
    }
}

// ---------------------------------------------------------------------------
// Public enum + configuration
// ---------------------------------------------------------------------------

pub enum CloudKms {
    Aws(AwsKms),
    Gcp(GcpKms),
}

pub const AWS_ID: &str = "aws";
pub const GCP_ID: &str = "gcp";

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

impl CloudKms {
    /// The id used in sealed values (`enc:v1:<id>:<ciphertext>`).
    pub fn id(&self) -> &'static str {
        match self {
            Self::Aws(_) => AWS_ID,
            Self::Gcp(_) => GCP_ID,
        }
    }

    /// Cloud providers the environment selects (more than one is a config error).
    pub fn selected(env: Env) -> Vec<&'static str> {
        let mut v = Vec::new();
        if env("ZORVIA_AWS_KMS_KEY_ID").is_some() {
            v.push(AWS_ID);
        }
        if env("ZORVIA_GCP_KMS_KEY").is_some() {
            v.push(GCP_ID);
        }
        v
    }

    pub fn from_env(env: Env) -> Result<Option<Self>> {
        match Self::selected(env).as_slice() {
            [] => Ok(None),
            [AWS_ID] => Ok(Some(Self::Aws(Self::aws_from_env(env)?))),
            [GCP_ID] => Ok(Some(Self::Gcp(Self::gcp_from_env(env)?))),
            _ => bail!("configure only one of ZORVIA_AWS_KMS_KEY_ID and ZORVIA_GCP_KMS_KEY"),
        }
    }

    fn aws_from_env(env: Env) -> Result<AwsKms> {
        let key_id = env("ZORVIA_AWS_KMS_KEY_ID").unwrap_or_default();
        let region = env("ZORVIA_AWS_REGION")
            .or_else(|| env("AWS_REGION"))
            .or_else(|| env("AWS_DEFAULT_REGION"))
            .ok_or_else(|| anyhow!("set ZORVIA_AWS_REGION (or AWS_REGION)"))?;
        let creds = match env("ZORVIA_AWS_CREDENTIALS_FILE") {
            Some(f) => AwsCredSource::File {
                path: PathBuf::from(f),
                profile: env("ZORVIA_AWS_PROFILE").unwrap_or_else(|| "default".into()),
            },
            None => {
                let ak = env("AWS_ACCESS_KEY_ID").ok_or_else(|| {
                    anyhow!(
                        "set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or ZORVIA_AWS_CREDENTIALS_FILE"
                    )
                })?;
                let sk = env("AWS_SECRET_ACCESS_KEY")
                    .ok_or_else(|| anyhow!("set AWS_SECRET_ACCESS_KEY"))?;
                AwsCredSource::Env {
                    access_key: clean_secret(&ak, "AWS_ACCESS_KEY_ID")?,
                    secret_key: clean_secret(&sk, "AWS_SECRET_ACCESS_KEY")?,
                    token: env("AWS_SESSION_TOKEN")
                        .map(|t| clean_secret(&t, "AWS_SESSION_TOKEN"))
                        .transpose()?,
                }
            }
        };
        let endpoint = env("ZORVIA_AWS_KMS_ENDPOINT");
        Self::aws(&key_id, &region, endpoint.as_deref(), creds)
    }

    fn aws(
        key_id: &str,
        region: &str,
        endpoint: Option<&str>,
        creds: AwsCredSource,
    ) -> Result<AwsKms> {
        if !valid_aws_key_id(key_id) {
            bail!("ZORVIA_AWS_KMS_KEY_ID must be a key id, key ARN or alias (letters, digits, - _ : /)");
        }
        if region.is_empty()
            || region.len() > 32
            || !region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bail!("the AWS region is not valid");
        }
        let default = format!("https://kms.{region}.amazonaws.com");
        Ok(AwsKms {
            region: region.to_string(),
            key_id: key_id.to_string(),
            endpoint: parse_endpoint(endpoint.unwrap_or(&default), "ZORVIA_AWS_KMS_ENDPOINT")?,
            creds,
            retry: Retry::default(),
        })
    }

    fn gcp_from_env(env: Env) -> Result<GcpKms> {
        let key = env("ZORVIA_GCP_KMS_KEY").unwrap_or_default();
        let meta = env("ZORVIA_GCP_USE_METADATA").is_some_and(|v| v == "1");
        let token = match (
            env("ZORVIA_GCP_ACCESS_TOKEN_FILE"),
            env("ZORVIA_GCP_ACCESS_TOKEN"),
            meta,
        ) {
            (Some(f), None, false) => GcpTokenSource::File(PathBuf::from(f)),
            (None, Some(t), false) => {
                GcpTokenSource::Static(clean_secret(&t, "ZORVIA_GCP_ACCESS_TOKEN")?)
            }
            (None, None, true) => {
                GcpTokenSource::Metadata(reqwest::Url::parse(GCE_METADATA_URL).expect("static url"))
            }
            _ => bail!(
                "set exactly one of ZORVIA_GCP_ACCESS_TOKEN_FILE, ZORVIA_GCP_ACCESS_TOKEN or ZORVIA_GCP_USE_METADATA=1"
            ),
        };
        Self::gcp(&key, env("ZORVIA_GCP_KMS_ENDPOINT").as_deref(), token)
    }

    fn gcp(key: &str, endpoint: Option<&str>, token: GcpTokenSource) -> Result<GcpKms> {
        if !valid_gcp_key(key) {
            bail!("ZORVIA_GCP_KMS_KEY must be projects/P/locations/L/keyRings/R/cryptoKeys/K");
        }
        Ok(GcpKms {
            endpoint: parse_endpoint(
                endpoint.unwrap_or("https://cloudkms.googleapis.com"),
                "ZORVIA_GCP_KMS_ENDPOINT",
            )?,
            key: key.to_string(),
            token,
            cache: Mutex::new(None),
            retry: Retry::default(),
        })
    }

    /// The configured key, for the startup log line (an id, not a secret).
    pub fn key_label(&self) -> &str {
        match self {
            Self::Aws(a) => &a.key_id,
            Self::Gcp(g) => &g.key,
        }
    }

    pub fn seal(&self, purpose: &str, owner: &str, secret: &str) -> Result<String> {
        match self {
            Self::Aws(a) => a.seal(purpose, owner, secret),
            Self::Gcp(g) => g.seal(purpose, owner, secret),
        }
    }

    pub fn open(&self, purpose: &str, owner: &str, payload: &str) -> Result<String> {
        match self {
            Self::Aws(a) => a.open(purpose, owner, payload),
            Self::Gcp(g) => g.open(purpose, owner, payload),
        }
    }
}

#[cfg(test)]
mod tests;
