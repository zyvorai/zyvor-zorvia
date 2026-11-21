// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Scan and sign gates for the golden-image pipeline.
//!
//! Both steps run as Kubernetes Jobs that the pipeline creates and waits on.
//! Nothing runs unless the operator configured it (environment variables,
//! never request bodies):
//!
//! | Variable | Meaning |
//! |----------|---------|
//! | `ZORVIA_GOLDEN_SCAN_IMAGE` | Scanner image (for example a trivy image). Enables the scan step. |
//! | `ZORVIA_GOLDEN_SCAN_COMMAND` | JSON array argv. `{image}` is replaced by the image reference. Default is a trivy JSON scan. |
//! | `ZORVIA_GOLDEN_SCAN_THRESHOLD` | `low`, `medium`, `high` (default) or `critical`: findings at or above it block promotion. |
//! | `ZORVIA_GOLDEN_SCAN_IGNORE_UNFIXED` | `1` ignores vulnerabilities that have no fixed version. |
//! | `ZORVIA_GOLDEN_SIGN_IMAGE` | cosign image. Enables the sign step. |
//! | `ZORVIA_GOLDEN_SIGN_KEY_SECRET` | Secret holding `cosign.pub` (and `cosign.key`, optional `cosign.password`) for key mode. |
//! | `ZORVIA_GOLDEN_SIGN_KEYLESS_IDENTITY` / `_ISSUER` | Keyless verification identity and OIDC issuer. |
//! | `ZORVIA_GOLDEN_SIGN_SIGN` | `1` also signs (key mode only) before verifying. |
//! | `ZORVIA_GOLDEN_PULL_SECRET` | Name of a `kubernetes.io/dockerconfigjson` Secret: used as `imagePullSecrets` and mounted as `DOCKER_CONFIG` in scan/sign/verify Jobs. |
//! | `ZORVIA_GOLDEN_CDI_SECRET` | Name of a CDI-format Secret (`accessKeyId`, `secretKey`) set as `secretRef` on the DataVolume registry/HTTP source. |
//! | `ZORVIA_GOLDEN_FETCH_IMAGE` | Image with `sh`, `curl` and `sha256sum` (for example a curl image). Needed for HTTP(S) checksum and blob-signature checks. |
//! | `ZORVIA_GOLDEN_BLOB_MAX_SIZE` | Ephemeral-storage limit for the downloaded blob in the verify-blob Job (default `50Gi`). |
//! | `ZORVIA_GOLDEN_HTTP_ALLOW_UNSCANNED` | `1` lets an HTTP(S) source pass when a scan gate is configured (disk blobs cannot be scanned). |
//! | `ZORVIA_GOLDEN_KEEP_ON_BLOCK` | `1` keeps the imported DataVolume when a gate blocks; default is to remove it. |
//!
//! Both gates fail closed: a failed Job, unparseable scanner output, findings
//! over the threshold or a failed verification all block promotion.

use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Capabilities, Container, EmptyDirVolumeSource, EnvVar, EnvVarSource, KeyToPath,
    LocalObjectReference, PodSecurityContext, PodSpec, PodTemplateSpec, SecretKeySelector,
    SecretVolumeSource, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_ARGS: usize = 32;
const MAX_ARG_LEN: usize = 512;
const DEFAULT_DEADLINE_SECS: i64 = 900;

/// Severity ordering used by the threshold (`Unknown` never blocks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn parse(s: &str) -> Severity {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Severity::Low,
            "medium" => Severity::Medium,
            "high" => Severity::High,
            "critical" => Severity::Critical,
            _ => Severity::Unknown,
        }
    }

    fn parse_threshold(s: &str) -> Result<Severity, String> {
        match Severity::parse(s) {
            Severity::Unknown => Err(format!(
                "invalid scan threshold '{s}' (use low, medium, high or critical)"
            )),
            sev => Ok(sev),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanConfig {
    pub image: String,
    pub command: Vec<String>,
    pub threshold: Severity,
    pub ignore_unfixed: bool,
    /// Secret (dockerconfigjson) for pulling the scanner image and the scanned image.
    pub pull_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignMode {
    /// Key from a Secret (`cosign.pub`, optionally `cosign.key` to sign).
    Key { secret: String, sign: bool },
    /// Keyless verification (signing is done elsewhere, for example CI).
    Keyless { identity: String, issuer: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignConfig {
    pub image: String,
    pub mode: SignMode,
    /// Secret (dockerconfigjson) for pulling the cosign image and the signed image.
    pub pull_secret: Option<String>,
}

/// Operator configuration of the two gates. `None` means "not configured".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateConfig {
    pub scan: Option<ScanConfig>,
    pub sign: Option<SignConfig>,
    /// Image used by HTTP(S) checksum and blob-signature Jobs.
    pub fetch_image: Option<String>,
    /// Ephemeral-storage limit for the blob download (Kubernetes quantity).
    pub blob_max_size: String,
    /// dockerconfigjson Secret name (never its contents).
    pub pull_secret: Option<String>,
    /// CDI-format Secret name for the DataVolume source `secretRef`.
    pub cdi_secret: Option<String>,
    pub keep_on_block: bool,
    pub allow_unscanned_http: bool,
}

/// Per-run checks for HTTP(S) disk sources. Not credentials: the signature is
/// verified against the operator's key Secret.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceChecks {
    /// Expected SHA-256 (64 hex characters, optional `sha256:` prefix).
    pub checksum_sha256: Option<String>,
    /// URL of a detached cosign signature for the blob.
    pub signature_url: Option<String>,
}

impl SourceChecks {
    pub fn is_empty(&self) -> bool {
        self.checksum_sha256.is_none() && self.signature_url.is_none()
    }

    /// Validates and normalises (lowercase hex digest without prefix).
    pub fn validated(&self) -> Result<Self, String> {
        let checksum_sha256 = match &self.checksum_sha256 {
            None => None,
            Some(c) => Some(normalize_sha256(c)?),
        };
        if let Some(u) = &self.signature_url {
            validate_http_url("signature URL", u)?;
        }
        Ok(Self {
            checksum_sha256,
            signature_url: self.signature_url.clone(),
        })
    }
}

/// Returns the lowercase 64-hex digest, accepting an optional `sha256:` prefix.
pub fn normalize_sha256(s: &str) -> Result<String, String> {
    let t = s.trim();
    let t = t.strip_prefix("sha256:").unwrap_or(t);
    if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(t.to_ascii_lowercase())
    } else {
        Err("checksum must be a SHA-256 hex digest (64 hex characters)".into())
    }
}

/// Validates an http(s) URL: bounded, no whitespace/control characters and no
/// embedded credentials (`user:pass@host`).
pub fn validate_http_url(kind: &str, url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("{kind} must start with http:// or https://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if url.len() > 2048
        || authority.is_empty()
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(format!("invalid {kind}"));
    }
    if authority.contains('@') {
        return Err(format!("{kind} must not embed credentials"));
    }
    Ok(())
}

/// Validates a container image reference (no whitespace, no option-looking
/// leading dash, conservative character set).
pub fn validate_image_ref(image: &str) -> Result<(), String> {
    let ok = !image.is_empty()
        && image.len() <= 255
        && !image.starts_with('-')
        && !image.starts_with('/')
        && image.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/' | b':' | b'@')
        });
    if ok {
        Ok(())
    } else {
        Err(format!("invalid image reference '{image}'"))
    }
}

/// Validates an argv: non-empty, bounded, no control characters, and the only
/// placeholder allowed is `{image}`.
pub fn validate_command(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() || argv[0].trim().is_empty() {
        return Err("scan command must be a non-empty JSON array".into());
    }
    if argv.len() > MAX_ARGS {
        return Err(format!("scan command has more than {MAX_ARGS} arguments"));
    }
    for a in argv {
        if a.len() > MAX_ARG_LEN || a.chars().any(|c| c.is_control()) {
            return Err("scan command argument too long or contains control characters".into());
        }
        let stripped = a.replace("{image}", "");
        if stripped.contains('{') || stripped.contains('}') {
            return Err("scan command only supports the {image} placeholder".into());
        }
    }
    Ok(())
}

fn validate_dns_name(kind: &str, s: &str) -> Result<(), String> {
    let ok = !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !s.starts_with(|c: char| !c.is_ascii_alphanumeric())
        && !s.ends_with(|c: char| !c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(format!("invalid {kind} '{s}'"))
    }
}

fn truthy(v: Option<String>) -> bool {
    v.map(|v| {
        let v = v.trim();
        v == "1" || v.eq_ignore_ascii_case("true")
    })
    .unwrap_or(false)
}

fn default_scan_command() -> Vec<String> {
    ["trivy", "image", "--format", "json", "--quiet", "{image}"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

impl GateConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
    }

    /// Builds and validates the configuration from a variable lookup.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let name_var = |var: &str, kind: &str| -> Result<Option<String>, String> {
            match get(var) {
                None => Ok(None),
                Some(v) => {
                    let v = v.trim().to_string();
                    validate_dns_name(kind, &v)?;
                    Ok(Some(v))
                }
            }
        };
        let pull_secret = name_var("ZORVIA_GOLDEN_PULL_SECRET", "pull Secret name")?;
        let cdi_secret = name_var("ZORVIA_GOLDEN_CDI_SECRET", "CDI Secret name")?;
        let fetch_image = match get("ZORVIA_GOLDEN_FETCH_IMAGE") {
            None => None,
            Some(i) => {
                let i = i.trim().to_string();
                validate_image_ref(&i)?;
                Some(i)
            }
        };
        let blob_max_size = get("ZORVIA_GOLDEN_BLOB_MAX_SIZE")
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| "50Gi".into());
        if !matches!(crate::storage::parse_size_to_bytes(&blob_max_size), Some(b) if b > 0) {
            return Err(format!(
                "invalid ZORVIA_GOLDEN_BLOB_MAX_SIZE '{blob_max_size}'"
            ));
        }
        let scan = match get("ZORVIA_GOLDEN_SCAN_IMAGE") {
            None => None,
            Some(image) => {
                let image = image.trim().to_string();
                validate_image_ref(&image)?;
                let command = match get("ZORVIA_GOLDEN_SCAN_COMMAND") {
                    None => default_scan_command(),
                    Some(raw) => serde_json::from_str::<Vec<String>>(&raw).map_err(|_| {
                        "ZORVIA_GOLDEN_SCAN_COMMAND must be a JSON array of strings".to_string()
                    })?,
                };
                validate_command(&command)?;
                let threshold = match get("ZORVIA_GOLDEN_SCAN_THRESHOLD") {
                    None => Severity::High,
                    Some(t) => Severity::parse_threshold(&t)?,
                };
                Some(ScanConfig {
                    image,
                    command,
                    threshold,
                    ignore_unfixed: truthy(get("ZORVIA_GOLDEN_SCAN_IGNORE_UNFIXED")),
                    pull_secret: pull_secret.clone(),
                })
            }
        };
        let sign = match get("ZORVIA_GOLDEN_SIGN_IMAGE") {
            None => None,
            Some(image) => {
                let image = image.trim().to_string();
                validate_image_ref(&image)?;
                let secret = get("ZORVIA_GOLDEN_SIGN_KEY_SECRET");
                let identity = get("ZORVIA_GOLDEN_SIGN_KEYLESS_IDENTITY");
                let issuer = get("ZORVIA_GOLDEN_SIGN_KEYLESS_ISSUER");
                let mode = match (secret, identity, issuer) {
                    (Some(s), None, None) => {
                        validate_dns_name("signing key Secret name", &s)?;
                        SignMode::Key {
                            secret: s,
                            sign: truthy(get("ZORVIA_GOLDEN_SIGN_SIGN")),
                        }
                    }
                    (None, Some(identity), Some(issuer)) => {
                        for v in [&identity, &issuer] {
                            if v.len() > 512 || v.chars().any(|c| c.is_control() || c.is_whitespace()) {
                                return Err("invalid keyless identity or issuer".into());
                            }
                        }
                        SignMode::Keyless { identity, issuer }
                    }
                    _ => {
                        return Err("sign step needs either ZORVIA_GOLDEN_SIGN_KEY_SECRET or both ZORVIA_GOLDEN_SIGN_KEYLESS_IDENTITY and ZORVIA_GOLDEN_SIGN_KEYLESS_ISSUER".into())
                    }
                };
                Some(SignConfig {
                    image,
                    mode,
                    pull_secret: pull_secret.clone(),
                })
            }
        };
        Ok(Self {
            scan,
            sign,
            fetch_image,
            blob_max_size,
            pull_secret,
            cdi_secret,
            keep_on_block: truthy(get("ZORVIA_GOLDEN_KEEP_ON_BLOCK")),
            allow_unscanned_http: truthy(get("ZORVIA_GOLDEN_HTTP_ALLOW_UNSCANNED")),
        })
    }

    pub fn any(&self) -> bool {
        self.scan.is_some() || self.sign.is_some()
    }
}

/// Inputs for [`job`]; everything beyond the main container is optional.
#[derive(Default)]
struct JobBuild<'a> {
    name: &'a str,
    namespace: &'a str,
    kind: &'a str,
    image: &'a str,
    command: Option<Vec<String>>,
    args: Option<Vec<String>>,
    env: Vec<EnvVar>,
    secret_vol: Option<Volume>,
    /// dockerconfigjson Secret name: imagePullSecrets plus `DOCKER_CONFIG`.
    pull_secret: Option<&'a str>,
    /// Init container that downloads a blob to `/blob/disk` (image, URL, size limit).
    blob_fetch: Option<(&'a str, &'a str, &'a str)>,
}

fn hardened() -> SecurityContext {
    SecurityContext {
        privileged: Some(false),
        allow_privilege_escalation: Some(false),
        read_only_root_filesystem: Some(true),
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Shell snippets run by the fetch image. The URL and digest are positional
/// parameters, never interpolated into the script text.
const CHECKSUM_SCRIPT: &str =
    "set -eu; curl -fsSL --proto '=http,https' --max-time 3600 -- \"$1\" | sha256sum | cut -d' ' -f1";
const FETCH_SCRIPT: &str =
    "set -eu; exec curl -fsSL --proto '=http,https' --max-time 3600 -o /blob/disk -- \"$1\"";

fn job(b: JobBuild) -> Job {
    let JobBuild {
        name,
        namespace,
        kind,
        image,
        command,
        args,
        env,
        secret_vol,
        pull_secret,
        blob_fetch,
    } = b;
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        format!("zorvia-golden-{kind}"),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "zorvia".to_string(),
    );

    let mut volumes = vec![Volume {
        name: "tmp".into(),
        empty_dir: Some(EmptyDirVolumeSource::default()),
        ..Default::default()
    }];
    let tmp_mount = VolumeMount {
        name: "tmp".into(),
        mount_path: "/tmp".into(),
        ..Default::default()
    };
    let mut mounts = vec![tmp_mount.clone()];
    if let Some(v) = secret_vol {
        mounts.push(VolumeMount {
            name: v.name.clone(),
            mount_path: "/cosign".into(),
            read_only: Some(true),
            ..Default::default()
        });
        volumes.push(v);
    }
    let mut all_env = vec![
        EnvVar {
            name: "HOME".into(),
            value: Some("/tmp".into()),
            ..Default::default()
        },
        EnvVar {
            name: "TRIVY_CACHE_DIR".into(),
            value: Some("/tmp/trivy".into()),
            ..Default::default()
        },
    ];
    if let Some(ps) = pull_secret {
        // Only the registry auth file is mounted; the Secret is referenced by name.
        volumes.push(Volume {
            name: "registry-auth".into(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(ps.to_string()),
                items: Some(vec![KeyToPath {
                    key: ".dockerconfigjson".into(),
                    path: "config.json".into(),
                    mode: None,
                }]),
                default_mode: Some(0o400),
                ..Default::default()
            }),
            ..Default::default()
        });
        mounts.push(VolumeMount {
            name: "registry-auth".into(),
            mount_path: "/docker".into(),
            read_only: Some(true),
            ..Default::default()
        });
        all_env.push(EnvVar {
            name: "DOCKER_CONFIG".into(),
            value: Some("/docker".into()),
            ..Default::default()
        });
    }
    all_env.extend(env);

    let mut init_containers = None;
    if let Some((fetch_image, url, max_size)) = blob_fetch {
        volumes.push(Volume {
            name: "blob".into(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(max_size.to_string())),
                ..Default::default()
            }),
            ..Default::default()
        });
        init_containers = Some(vec![Container {
            name: "fetch".into(),
            image: Some(fetch_image.to_string()),
            command: Some(
                ["sh", "-c", FETCH_SCRIPT, "sh", url]
                    .map(String::from)
                    .to_vec(),
            ),
            env: Some(vec![EnvVar {
                name: "HOME".into(),
                value: Some("/tmp".into()),
                ..Default::default()
            }]),
            volume_mounts: Some(vec![
                tmp_mount,
                VolumeMount {
                    name: "blob".into(),
                    mount_path: "/blob".into(),
                    ..Default::default()
                },
            ]),
            security_context: Some(hardened()),
            ..Default::default()
        }]);
        mounts.push(VolumeMount {
            name: "blob".into(),
            mount_path: "/blob".into(),
            read_only: Some(true),
            ..Default::default()
        });
    }

    Job {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(0),
            ttl_seconds_after_finished: Some(3600),
            active_deadline_seconds: Some(DEFAULT_DEADLINE_SECS),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("Never".into()),
                    automount_service_account_token: Some(false),
                    image_pull_secrets: pull_secret.map(|n| {
                        vec![LocalObjectReference {
                            name: n.to_string(),
                        }]
                    }),
                    security_context: Some(PodSecurityContext {
                        run_as_non_root: Some(true),
                        run_as_user: Some(65532),
                        ..Default::default()
                    }),
                    init_containers,
                    containers: vec![Container {
                        name: kind.to_string(),
                        image: Some(image.to_string()),
                        command,
                        args,
                        env: Some(all_env),
                        volume_mounts: Some(mounts),
                        security_context: Some(hardened()),
                        ..Default::default()
                    }],
                    volumes: Some(volumes),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Builds the vulnerability-scan Job for `image_ref`.
pub fn build_scan_job(
    name: &str,
    namespace: &str,
    cfg: &ScanConfig,
    image_ref: &str,
) -> Result<Job, String> {
    validate_image_ref(image_ref)?;
    validate_command(&cfg.command)?;
    let argv: Vec<String> = cfg
        .command
        .iter()
        .map(|a| a.replace("{image}", image_ref))
        .collect();
    Ok(job(JobBuild {
        name,
        namespace,
        kind: "scan",
        image: &cfg.image,
        command: Some(argv),
        pull_secret: cfg.pull_secret.as_deref(),
        ..Default::default()
    }))
}

/// What a cosign Job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosignAction {
    Sign,
    Verify,
}

/// Builds a cosign sign or verify Job for `image_ref`.
pub fn build_cosign_job(
    name: &str,
    namespace: &str,
    cfg: &SignConfig,
    action: CosignAction,
    image_ref: &str,
) -> Result<Job, String> {
    validate_image_ref(image_ref)?;
    let mut args: Vec<String> = Vec::new();
    let mut env = Vec::new();
    let mut vol = None;
    match (&cfg.mode, action) {
        (SignMode::Key { secret, sign }, CosignAction::Sign) => {
            if !*sign {
                return Err("signing is not enabled (ZORVIA_GOLDEN_SIGN_SIGN)".into());
            }
            args.extend(["sign", "--yes", "--key", "/cosign/cosign.key"].map(String::from));
            env.push(EnvVar {
                name: "COSIGN_PASSWORD".into(),
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: secret.clone(),
                        key: "cosign.password".into(),
                        optional: Some(true),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
            vol = Some(secret_volume(secret, &["cosign.key"]));
        }
        (SignMode::Key { secret, .. }, CosignAction::Verify) => {
            args.extend(["verify", "--key", "/cosign/cosign.pub"].map(String::from));
            // Only the public key is mounted into a verify Job.
            vol = Some(secret_volume(secret, &["cosign.pub"]));
        }
        (SignMode::Keyless { .. }, CosignAction::Sign) => {
            return Err("keyless mode only verifies; signing happens outside Zorvia".into());
        }
        (SignMode::Keyless { identity, issuer }, CosignAction::Verify) => {
            args.extend(
                [
                    "verify",
                    "--certificate-identity",
                    identity,
                    "--certificate-oidc-issuer",
                    issuer,
                ]
                .map(String::from),
            );
        }
    }
    args.push(image_ref.to_string());
    let kind = if action == CosignAction::Sign {
        "sign"
    } else {
        "verify"
    };
    Ok(job(JobBuild {
        name,
        namespace,
        kind,
        image: &cfg.image,
        args: Some(args),
        env,
        secret_vol: vol,
        pull_secret: cfg.pull_secret.as_deref(),
        ..Default::default()
    }))
}

/// Builds the Job that streams an HTTP(S) blob through `sha256sum` and prints
/// the digest as its last log line (compared by the gate, not in the shell).
pub fn build_checksum_job(
    name: &str,
    namespace: &str,
    fetch_image: &str,
    pull_secret: Option<&str>,
    url: &str,
) -> Result<Job, String> {
    validate_image_ref(fetch_image)?;
    validate_http_url("source URL", url)?;
    Ok(job(JobBuild {
        name,
        namespace,
        kind: "checksum",
        image: fetch_image,
        command: Some(
            ["sh", "-c", CHECKSUM_SCRIPT, "sh", url]
                .map(String::from)
                .to_vec(),
        ),
        pull_secret,
        ..Default::default()
    }))
}

/// Builds a `cosign verify-blob` Job: an init container downloads the blob,
/// the cosign container verifies it against the detached signature URL with
/// the public key from the Secret. Key mode only.
pub fn build_blob_verify_job(
    name: &str,
    namespace: &str,
    cfg: &SignConfig,
    fetch_image: &str,
    max_size: &str,
    url: &str,
    signature_url: &str,
) -> Result<Job, String> {
    validate_image_ref(fetch_image)?;
    validate_http_url("source URL", url)?;
    validate_http_url("signature URL", signature_url)?;
    let SignMode::Key { secret, .. } = &cfg.mode else {
        return Err(
            "blob signature verification needs key mode (ZORVIA_GOLDEN_SIGN_KEY_SECRET)".into(),
        );
    };
    let args = [
        "verify-blob",
        "--key",
        "/cosign/cosign.pub",
        "--signature",
        signature_url,
        "/blob/disk",
    ]
    .map(String::from)
    .to_vec();
    Ok(job(JobBuild {
        name,
        namespace,
        kind: "verify-blob",
        image: &cfg.image,
        args: Some(args),
        secret_vol: Some(secret_volume(secret, &["cosign.pub"])),
        pull_secret: cfg.pull_secret.as_deref(),
        blob_fetch: Some((fetch_image, url, max_size)),
        ..Default::default()
    }))
}

fn secret_volume(secret: &str, keys: &[&str]) -> Volume {
    Volume {
        name: "cosign".into(),
        secret: Some(SecretVolumeSource {
            secret_name: Some(secret.to_string()),
            items: Some(
                keys.iter()
                    .map(|k| KeyToPath {
                        key: k.to_string(),
                        path: k.to_string(),
                        mode: None,
                    })
                    .collect(),
            ),
            default_mode: Some(0o400),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Vulnerability counts per severity from a scanner report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSummary {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub unknown: u32,
}

impl ScanSummary {
    pub fn total(&self) -> u32 {
        self.critical + self.high + self.medium + self.low + self.unknown
    }

    /// Findings at or above `threshold`.
    pub fn at_or_above(&self, threshold: Severity) -> u32 {
        let mut n = 0;
        if threshold <= Severity::Critical {
            n += self.critical;
        }
        if threshold <= Severity::High {
            n += self.high;
        }
        if threshold <= Severity::Medium {
            n += self.medium;
        }
        if threshold <= Severity::Low {
            n += self.low;
        }
        n
    }
}

/// Parses trivy-style JSON (`Results[].Vulnerabilities[].Severity`). Tolerates
/// leading/trailing non-JSON log lines. Fails on anything that is not a
/// recognisable report so the gate fails closed.
pub fn parse_trivy_report(output: &str, ignore_unfixed: bool) -> Result<ScanSummary, String> {
    let start = output.find('{').ok_or("scanner output contains no JSON")?;
    let end = output.rfind('}').ok_or("scanner output contains no JSON")?;
    if end < start {
        return Err("scanner output contains no JSON".into());
    }
    let v: serde_json::Value = serde_json::from_str(&output[start..=end])
        .map_err(|e| format!("scanner output is not valid JSON: {e}"))?;
    let obj = v.as_object().ok_or("scanner JSON is not an object")?;
    let results = match obj.get("Results") {
        // Trivy omits or nulls Results when nothing was found.
        None | Some(serde_json::Value::Null) => {
            if obj.contains_key("SchemaVersion") || obj.contains_key("ArtifactName") {
                return Ok(ScanSummary::default());
            }
            return Err("scanner JSON has no Results (not a trivy report)".into());
        }
        Some(r) => r.as_array().ok_or("scanner Results is not an array")?,
    };
    let mut s = ScanSummary::default();
    for r in results {
        let Some(vulns) = r.get("Vulnerabilities").and_then(|v| v.as_array()) else {
            continue;
        };
        for vuln in vulns {
            if ignore_unfixed
                && vuln
                    .get("FixedVersion")
                    .and_then(|f| f.as_str())
                    .map(|f| f.trim().is_empty())
                    .unwrap_or(true)
            {
                continue;
            }
            match Severity::parse(vuln.get("Severity").and_then(|x| x.as_str()).unwrap_or("")) {
                Severity::Critical => s.critical += 1,
                Severity::High => s.high += 1,
                Severity::Medium => s.medium += 1,
                Severity::Low => s.low += 1,
                Severity::Unknown => s.unknown += 1,
            }
        }
    }
    Ok(s)
}

/// Outcome of a Job as seen by the runner.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub succeeded: bool,
    pub logs: String,
}

/// Creates a Job, waits for it and returns its outcome. Real impl is
/// `KubeJobRunner`; tests use a mock.
#[allow(async_fn_in_trait)]
pub trait JobRunner {
    async fn run_job(&self, namespace: &str, job: Job) -> Result<JobOutcome, String>;
}

/// Kubernetes-backed runner (create, poll, read logs, delete).
pub struct KubeJobRunner {
    pub client: kube::Client,
}

impl JobRunner for KubeJobRunner {
    async fn run_job(&self, namespace: &str, job: Job) -> Result<JobOutcome, String> {
        use k8s_openapi::api::core::v1::Pod;
        use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};
        let name = job.metadata.name.clone().ok_or("job has no name")?;
        let deadline = job
            .spec
            .as_ref()
            .and_then(|s| s.active_deadline_seconds)
            .unwrap_or(DEFAULT_DEADLINE_SECS) as u64
            + 60;
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        jobs.create(&PostParams::default(), &job)
            .await
            .map_err(|e| format!("creating Job {name} failed: {e}"))?;
        let started = std::time::Instant::now();
        let succeeded = loop {
            let j = jobs
                .get(&name)
                .await
                .map_err(|e| format!("reading Job {name} failed: {e}"))?;
            let st = j.status.unwrap_or_default();
            if st.succeeded.unwrap_or(0) > 0 {
                break true;
            }
            if st.failed.unwrap_or(0) > 0 {
                break false;
            }
            if started.elapsed().as_secs() > deadline {
                let _ = jobs.delete(&name, &DeleteParams::background()).await;
                return Err(format!("Job {name} did not finish within {deadline}s"));
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        };
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut logs = String::new();
        if let Ok(list) = pods
            .list(&ListParams::default().labels(&format!("job-name={name}")))
            .await
        {
            if let Some(pn) = list.items.into_iter().next().and_then(|p| p.metadata.name) {
                let lp = LogParams {
                    limit_bytes: Some(32 * 1024 * 1024),
                    ..Default::default()
                };
                logs = pods.logs(&pn, &lp).await.unwrap_or_default();
            }
        }
        let _ = jobs.delete(&name, &DeleteParams::background()).await;
        Ok(JobOutcome { succeeded, logs })
    }
}

/// Report of one gate step. Never contains scanner logs or key material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepReport {
    pub step: String,
    pub passed: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ScanSummary>,
}

/// Result of running the gates. `blocked_reason` is set when promotion must not happen.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateReport {
    pub steps: Vec<StepReport>,
    pub blocked_reason: Option<String>,
}

impl GateReport {
    pub fn promote_allowed(&self) -> bool {
        self.blocked_reason.is_none()
    }
}

/// Image reference to scan/sign for a pipeline source. Only registry sources
/// (`docker://ref`) can be scanned and signed; anything else blocks when a gate is configured.
pub fn image_ref_for_source(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return Err("scan/sign need a registry image source; HTTP(S) disk sources cannot be scanned or signed".into());
    }
    let r = source.strip_prefix("docker://").unwrap_or(source);
    validate_image_ref(r)?;
    Ok(r.to_string())
}

fn block(report: &mut GateReport, step: &str, detail: String) {
    report.blocked_reason = Some(format!("{step} blocked promotion: {detail}"));
    report.steps.push(StepReport {
        step: step.into(),
        passed: false,
        detail,
        summary: None,
    });
}

/// Runs a Job; blocks the report (and returns `None`) on a runner error or a failed Job.
async fn exec_step<R: JobRunner>(
    runner: &R,
    namespace: &str,
    job: Job,
    step: &str,
    failed: &str,
    report: &mut GateReport,
) -> Option<JobOutcome> {
    match runner.run_job(namespace, job).await {
        Err(e) => {
            block(report, step, e);
            None
        }
        Ok(o) if !o.succeeded => {
            block(report, step, failed.into());
            None
        }
        Ok(o) => Some(o),
    }
}

/// Gates for an HTTP(S) disk source: checksum (SHA-256) and detached cosign
/// blob signature, both run as Jobs. Vulnerability scanning does not apply to
/// disk blobs; it blocks unless the operator accepted that.
async fn run_http_gates<R: JobRunner>(
    cfg: &GateConfig,
    runner: &R,
    namespace: &str,
    job_prefix: &str,
    url: &str,
    checks: &SourceChecks,
) -> GateReport {
    let mut report = GateReport::default();
    if let Err(e) = validate_http_url("source URL", url) {
        block(&mut report, "source", e);
        return report;
    }
    let checks = match checks.validated() {
        Ok(c) => c,
        Err(e) => {
            block(&mut report, "source", e);
            return report;
        }
    };
    if cfg.scan.is_some() {
        if cfg.allow_unscanned_http {
            report.steps.push(StepReport {
                step: "scan:skipped".into(),
                passed: true,
                detail: "HTTP(S) disk sources cannot be scanned; accepted by ZORVIA_GOLDEN_HTTP_ALLOW_UNSCANNED".into(),
                summary: None,
            });
        } else {
            block(
                &mut report,
                "scan",
                "HTTP(S) disk sources cannot be vulnerability-scanned (set ZORVIA_GOLDEN_HTTP_ALLOW_UNSCANNED=1 to accept)".into(),
            );
            return report;
        }
    }
    if checks.signature_url.is_some() && cfg.sign.is_none() {
        block(
            &mut report,
            "verify-blob",
            "a signature URL was given but no sign gate is configured".into(),
        );
        return report;
    }
    if cfg.sign.is_some() && checks.signature_url.is_none() {
        block(
            &mut report,
            "verify-blob",
            "sign gate is configured but no signature URL was supplied for the HTTP(S) source"
                .into(),
        );
        return report;
    }
    let fetch_image = match (&cfg.fetch_image, checks.is_empty()) {
        (_, true) => return report,
        (None, false) => {
            block(
                &mut report,
                "checksum",
                "ZORVIA_GOLDEN_FETCH_IMAGE is required for HTTP(S) checksum/signature checks"
                    .into(),
            );
            return report;
        }
        (Some(i), false) => i.clone(),
    };

    if let Some(expected) = &checks.checksum_sha256 {
        let name = crate::utils::generate_id(&format!("{job_prefix}-sum"), "g");
        let job = match build_checksum_job(
            &name,
            namespace,
            &fetch_image,
            cfg.pull_secret.as_deref(),
            url,
        ) {
            Ok(j) => j,
            Err(e) => {
                block(&mut report, "checksum", e);
                return report;
            }
        };
        let Some(o) = exec_step(
            runner,
            namespace,
            job,
            "checksum",
            "checksum Job failed (download error)",
            &mut report,
        )
        .await
        else {
            return report;
        };
        let actual = o
            .logs
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| l.split_whitespace().next())
            .map(|d| d.to_ascii_lowercase())
            .unwrap_or_default();
        if actual.len() != 64 || !actual.bytes().all(|b| b.is_ascii_hexdigit()) {
            block(
                &mut report,
                "checksum",
                "checksum Job produced no SHA-256 digest".into(),
            );
            return report;
        }
        if &actual != expected {
            block(
                &mut report,
                "checksum",
                format!("checksum mismatch (expected {expected}, got {actual})"),
            );
            return report;
        }
        report.steps.push(StepReport {
            step: "checksum".into(),
            passed: true,
            detail: "SHA-256 matches the expected value".into(),
            summary: None,
        });
    }

    if let (Some(sign), Some(sig_url)) = (&cfg.sign, &checks.signature_url) {
        let name = crate::utils::generate_id(&format!("{job_prefix}-vb"), "g");
        let job = match build_blob_verify_job(
            &name,
            namespace,
            sign,
            &fetch_image,
            &cfg.blob_max_size,
            url,
            sig_url,
        ) {
            Ok(j) => j,
            Err(e) => {
                block(&mut report, "verify-blob", e);
                return report;
            }
        };
        if exec_step(
            runner,
            namespace,
            job,
            "verify-blob",
            "cosign verify-blob failed (signature missing, invalid or download error)",
            &mut report,
        )
        .await
        .is_none()
        {
            return report;
        }
        report.steps.push(StepReport {
            step: "verify-blob".into(),
            passed: true,
            detail: "cosign verify-blob succeeded".into(),
            summary: None,
        });
    }
    report
}

/// Runs the configured gates in order (scan, sign, verify) and stops at the first failure.
/// HTTP(S) sources run the checksum and blob-signature gates instead.
pub async fn run_gates<R: JobRunner>(
    cfg: &GateConfig,
    runner: &R,
    namespace: &str,
    job_prefix: &str,
    source: &str,
    checks: &SourceChecks,
) -> GateReport {
    if source.starts_with("http://") || source.starts_with("https://") {
        return run_http_gates(cfg, runner, namespace, job_prefix, source, checks).await;
    }
    let mut report = GateReport::default();
    if !checks.is_empty() {
        block(
            &mut report,
            "source",
            "checksum and signature URL only apply to HTTP(S) sources".into(),
        );
        return report;
    }
    if !cfg.any() {
        return report;
    }
    let image_ref = match image_ref_for_source(source) {
        Ok(r) => r,
        Err(e) => {
            block(
                &mut report,
                if cfg.scan.is_some() { "scan" } else { "sign" },
                e,
            );
            return report;
        }
    };

    if let Some(scan) = &cfg.scan {
        let name = crate::utils::generate_id(&format!("{job_prefix}-scan"), "g");
        let job = match build_scan_job(&name, namespace, scan, &image_ref) {
            Ok(j) => j,
            Err(e) => {
                block(&mut report, "scan", e);
                return report;
            }
        };
        match runner.run_job(namespace, job).await {
            Err(e) => {
                block(&mut report, "scan", e);
                return report;
            }
            Ok(o) if !o.succeeded => {
                block(&mut report, "scan", "scanner Job failed".into());
                return report;
            }
            Ok(o) => match parse_trivy_report(&o.logs, scan.ignore_unfixed) {
                Err(e) => {
                    block(&mut report, "scan", e);
                    return report;
                }
                Ok(sum) => {
                    let over = sum.at_or_above(scan.threshold);
                    let detail = format!(
                        "{} critical, {} high, {} medium, {} low, {} unknown",
                        sum.critical, sum.high, sum.medium, sum.low, sum.unknown
                    );
                    if over > 0 {
                        let reason = format!(
                            "{over} finding(s) at or above {:?} ({detail})",
                            scan.threshold
                        )
                        .to_lowercase();
                        report.blocked_reason = Some(format!("scan blocked promotion: {reason}"));
                        report.steps.push(StepReport {
                            step: "scan".into(),
                            passed: false,
                            detail: reason,
                            summary: Some(sum),
                        });
                        return report;
                    }
                    report.steps.push(StepReport {
                        step: "scan".into(),
                        passed: true,
                        detail: format!("below threshold ({detail})"),
                        summary: Some(sum),
                    });
                }
            },
        }
    }

    if let Some(sign) = &cfg.sign {
        let mut actions = Vec::new();
        if matches!(sign.mode, SignMode::Key { sign: true, .. }) {
            actions.push(CosignAction::Sign);
        }
        actions.push(CosignAction::Verify);
        for action in actions {
            let step = if action == CosignAction::Sign {
                "sign"
            } else {
                "verify"
            };
            let name = crate::utils::generate_id(&format!("{job_prefix}-{step}"), "g");
            let job = match build_cosign_job(&name, namespace, sign, action, &image_ref) {
                Ok(j) => j,
                Err(e) => {
                    block(&mut report, step, e);
                    return report;
                }
            };
            match runner.run_job(namespace, job).await {
                Err(e) => {
                    block(&mut report, step, e);
                    return report;
                }
                Ok(o) if !o.succeeded => {
                    block(
                        &mut report,
                        step,
                        format!("cosign {step} failed (signature missing or invalid)"),
                    );
                    return report;
                }
                Ok(_) => report.steps.push(StepReport {
                    step: step.into(),
                    passed: true,
                    detail: format!("cosign {step} succeeded"),
                    summary: None,
                }),
            }
        }
    }
    report
}

/// Label Zorvia puts on the DataVolume it creates in a pipeline run. Cleanup
/// only ever deletes an object whose label equals the run id.
pub const RUN_LABEL: &str = "zorvia.io/golden-run";

/// Minimal DataVolume access needed for cleanup (mocked in tests).
#[allow(async_fn_in_trait)]
pub trait DataVolumeStore {
    /// Labels of the DataVolume, or `None` when it does not exist.
    async fn labels(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<BTreeMap<String, String>>, String>;
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), String>;
}

impl DataVolumeStore for crate::kube::KubeClient {
    async fn labels(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<BTreeMap<String, String>>, String> {
        let dv = self
            .find_data_volume(namespace, name)
            .await
            .map_err(|e| e.to_string())?;
        Ok(dv.map(|v| {
            v["metadata"]["labels"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        }))
    }
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), String> {
        self.delete_data_volume(namespace, name)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Removes the DataVolume imported by a blocked run, but only when it carries
/// this run's label. Returns whether it was removed and a note.
pub async fn cleanup_blocked_import<S: DataVolumeStore>(
    store: &S,
    namespace: &str,
    name: &str,
    run_id: &str,
) -> (bool, String) {
    match store.labels(namespace, name).await {
        Err(e) => (
            false,
            format!("cleanup skipped: could not read DataVolume ({e})"),
        ),
        Ok(None) => (false, "imported DataVolume already gone".into()),
        Ok(Some(l)) if l.get(RUN_LABEL).map(String::as_str) != Some(run_id) => (
            false,
            "DataVolume not removed: it does not carry this run's label".into(),
        ),
        Ok(Some(_)) => match store.delete(namespace, name).await {
            Ok(()) => (true, "imported DataVolume removed after the block".into()),
            Err(e) => (
                false,
                format!("cleanup failed: could not delete DataVolume ({e})"),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const CLEAN: &str = include_str!("../../fixtures/golden-scan/trivy-clean.json");
    const MIXED: &str = include_str!("../../fixtures/golden-scan/trivy-mixed.json");

    fn cfg(pairs: &[(&str, &str)]) -> Result<GateConfig, String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        GateConfig::from_lookup(|k| m.get(k).cloned())
    }

    struct Mock {
        outcomes: Mutex<Vec<Result<JobOutcome, String>>>,
        jobs: Mutex<Vec<Job>>,
    }
    impl Mock {
        fn new(o: Vec<Result<JobOutcome, String>>) -> Self {
            Self {
                outcomes: Mutex::new(o),
                jobs: Mutex::new(vec![]),
            }
        }
    }
    impl JobRunner for Mock {
        async fn run_job(&self, _ns: &str, job: Job) -> Result<JobOutcome, String> {
            self.jobs.lock().unwrap().push(job);
            self.outcomes.lock().unwrap().remove(0)
        }
    }
    fn ok(logs: &str) -> Result<JobOutcome, String> {
        Ok(JobOutcome {
            succeeded: true,
            logs: logs.into(),
        })
    }

    fn none() -> SourceChecks {
        SourceChecks::default()
    }

    #[test]
    fn nothing_configured_means_no_gates() {
        let c = cfg(&[]).unwrap();
        assert!(!c.any());
    }

    #[test]
    fn config_validation() {
        assert!(cfg(&[(
            "ZORVIA_GOLDEN_SCAN_IMAGE",
            "ghcr.io/aquasecurity/trivy:0.50.0"
        )])
        .is_ok());
        assert!(cfg(&[("ZORVIA_GOLDEN_SCAN_IMAGE", "-evil")]).is_err());
        assert!(cfg(&[("ZORVIA_GOLDEN_SCAN_IMAGE", "a b")]).is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "t:1"),
            ("ZORVIA_GOLDEN_SCAN_COMMAND", "not json")
        ])
        .is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "t:1"),
            ("ZORVIA_GOLDEN_SCAN_COMMAND", "[]")
        ])
        .is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "t:1"),
            ("ZORVIA_GOLDEN_SCAN_COMMAND", r#"["x","{other}"]"#)
        ])
        .is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "t:1"),
            ("ZORVIA_GOLDEN_SCAN_THRESHOLD", "bogus")
        ])
        .is_err());
        let c = cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "t:1"),
            ("ZORVIA_GOLDEN_SCAN_THRESHOLD", "critical"),
        ])
        .unwrap();
        assert_eq!(c.scan.unwrap().threshold, Severity::Critical);
        // sign needs exactly one mode
        assert!(cfg(&[("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2")]).is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2"),
            ("ZORVIA_GOLDEN_SIGN_KEY_SECRET", "Bad_Name")
        ])
        .is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2"),
            ("ZORVIA_GOLDEN_SIGN_KEYLESS_IDENTITY", "a@b")
        ])
        .is_err());
        assert!(cfg(&[
            ("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2"),
            ("ZORVIA_GOLDEN_SIGN_KEY_SECRET", "k"),
            ("ZORVIA_GOLDEN_SIGN_KEYLESS_IDENTITY", "a"),
            ("ZORVIA_GOLDEN_SIGN_KEYLESS_ISSUER", "https://i")
        ])
        .is_err());
    }

    #[test]
    fn scan_job_spec() {
        let c = cfg(&[("ZORVIA_GOLDEN_SCAN_IMAGE", "trivy:1")])
            .unwrap()
            .scan
            .unwrap();
        let j = build_scan_job("s1", "ns", &c, "quay.io/containerdisks/ubuntu:24.04").unwrap();
        let spec = j.spec.unwrap();
        assert_eq!(spec.backoff_limit, Some(0));
        let pod = spec.template.spec.unwrap();
        assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
        assert_eq!(pod.automount_service_account_token, Some(false));
        let ct = &pod.containers[0];
        assert_eq!(ct.image.as_deref(), Some("trivy:1"));
        let cmd = ct.command.as_ref().unwrap();
        assert_eq!(cmd.last().unwrap(), "quay.io/containerdisks/ubuntu:24.04");
        assert_eq!(cmd[0], "trivy");
        let sc = ct.security_context.as_ref().unwrap();
        assert_eq!(sc.allow_privilege_escalation, Some(false));
        assert_eq!(sc.read_only_root_filesystem, Some(true));
        assert!(build_scan_job("s1", "ns", &c, "bad ref").is_err());
    }

    #[test]
    fn cosign_job_specs() {
        let key = SignConfig {
            image: "cosign:2".into(),
            mode: SignMode::Key {
                secret: "ck".into(),
                sign: true,
            },
            pull_secret: None,
        };
        let v = build_cosign_job("v", "ns", &key, CosignAction::Verify, "r.io/a:1").unwrap();
        let pod = v.spec.unwrap().template.spec.unwrap();
        let args = pod.containers[0].args.clone().unwrap();
        assert_eq!(args, ["verify", "--key", "/cosign/cosign.pub", "r.io/a:1"]);
        // verify mounts only the public key
        let items = pod
            .volumes
            .unwrap()
            .into_iter()
            .find(|v| v.name == "cosign")
            .unwrap()
            .secret
            .unwrap()
            .items
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "cosign.pub");
        // sign job carries password via secretKeyRef, never as a literal
        let s = build_cosign_job("s", "ns", &key, CosignAction::Sign, "r.io/a:1").unwrap();
        let ct = &s.spec.unwrap().template.spec.unwrap().containers[0];
        let pw = ct
            .env
            .as_ref()
            .unwrap()
            .iter()
            .find(|e| e.name == "COSIGN_PASSWORD")
            .unwrap();
        assert!(pw.value.is_none() && pw.value_from.is_some());
        let keyless = SignConfig {
            image: "cosign:2".into(),
            mode: SignMode::Keyless {
                identity: "me@x".into(),
                issuer: "https://i".into(),
            },
            pull_secret: None,
        };
        let k = build_cosign_job("k", "ns", &keyless, CosignAction::Verify, "r.io/a:1").unwrap();
        let args = k.spec.unwrap().template.spec.unwrap().containers[0]
            .args
            .clone()
            .unwrap();
        assert!(args.contains(&"--certificate-identity".to_string()));
        assert!(build_cosign_job("k", "ns", &keyless, CosignAction::Sign, "r.io/a:1").is_err());
        let nosign = SignConfig {
            image: "c".into(),
            mode: SignMode::Key {
                secret: "ck".into(),
                sign: false,
            },
            pull_secret: None,
        };
        assert!(build_cosign_job("s", "ns", &nosign, CosignAction::Sign, "r.io/a:1").is_err());
    }

    #[test]
    fn parse_fixtures_and_threshold() {
        let clean = parse_trivy_report(CLEAN, false).unwrap();
        assert_eq!(clean.total(), 0);
        let m = parse_trivy_report(MIXED, false).unwrap();
        assert_eq!(
            (m.critical, m.high, m.medium, m.low, m.unknown),
            (1, 2, 1, 1, 1)
        );
        assert_eq!(m.at_or_above(Severity::Critical), 1);
        assert_eq!(m.at_or_above(Severity::High), 3);
        assert_eq!(m.at_or_above(Severity::Low), 5);
        let fixed_only = parse_trivy_report(MIXED, true).unwrap();
        assert!(fixed_only.total() < m.total());
        // log noise around the JSON is tolerated
        let noisy = format!("2026 INFO scanning\n{CLEAN}\n");
        assert!(parse_trivy_report(&noisy, false).is_ok());
        // fail closed on garbage
        assert!(parse_trivy_report("", false).is_err());
        assert!(parse_trivy_report("{\"hello\":1}", false).is_err());
        assert!(parse_trivy_report("{not json}", false).is_err());
    }

    #[tokio::test]
    async fn unconfigured_runs_nothing() {
        let r = Mock::new(vec![]);
        let rep = run_gates(
            &GateConfig::default(),
            &r,
            "ns",
            "p",
            "docker://r.io/a:1",
            &none(),
        )
        .await;
        assert!(rep.promote_allowed() && rep.steps.is_empty());
        assert!(r.jobs.lock().unwrap().is_empty());
    }

    fn scan_sign_cfg() -> GateConfig {
        cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "trivy:1"),
            ("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2"),
            ("ZORVIA_GOLDEN_SIGN_KEY_SECRET", "ck"),
            ("ZORVIA_GOLDEN_SIGN_SIGN", "1"),
        ])
        .unwrap()
    }

    #[tokio::test]
    async fn gate_allows_clean_scan_and_good_signature() {
        let r = Mock::new(vec![ok(CLEAN), ok(""), ok("")]);
        let rep = run_gates(
            &scan_sign_cfg(),
            &r,
            "ns",
            "p",
            "docker://r.io/a:1",
            &none(),
        )
        .await;
        assert!(rep.promote_allowed(), "{rep:?}");
        let names: Vec<_> = rep.steps.iter().map(|s| s.step.as_str()).collect();
        assert_eq!(names, ["scan", "sign", "verify"]);
        assert_eq!(r.jobs.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn gate_blocks_on_findings_over_threshold() {
        let r = Mock::new(vec![ok(MIXED)]);
        let rep = run_gates(&scan_sign_cfg(), &r, "ns", "p", "r.io/a:1", &none()).await;
        assert!(!rep.promote_allowed());
        assert!(rep
            .blocked_reason
            .unwrap()
            .contains("3 finding(s) at or above high"));
        // sign jobs never ran
        assert_eq!(r.jobs.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn gate_blocks_on_failures_and_bad_output() {
        for outcome in [
            Err("boom".to_string()),
            Ok(JobOutcome {
                succeeded: false,
                logs: String::new(),
            }),
            ok("not json"),
        ] {
            let r = Mock::new(vec![outcome]);
            let rep = run_gates(&scan_sign_cfg(), &r, "ns", "p", "r.io/a:1", &none()).await;
            assert!(!rep.promote_allowed());
            assert!(rep
                .blocked_reason
                .unwrap()
                .starts_with("scan blocked promotion"));
        }
    }

    #[tokio::test]
    async fn gate_blocks_on_failed_verification() {
        let r = Mock::new(vec![
            ok(CLEAN),
            ok(""),
            Ok(JobOutcome {
                succeeded: false,
                logs: String::new(),
            }),
        ]);
        let rep = run_gates(&scan_sign_cfg(), &r, "ns", "p", "r.io/a:1", &none()).await;
        assert!(rep
            .blocked_reason
            .unwrap()
            .contains("verify blocked promotion"));
    }

    #[tokio::test]
    async fn http_source_blocks_when_gates_configured() {
        let r = Mock::new(vec![]);
        let rep = run_gates(
            &scan_sign_cfg(),
            &r,
            "ns",
            "p",
            "https://x/y.qcow2",
            &none(),
        )
        .await;
        assert!(!rep.promote_allowed());
        assert!(r.jobs.lock().unwrap().is_empty());
    }

    const URL: &str = "https://images.example.com/disk.qcow2";
    const SIG: &str = "https://images.example.com/disk.qcow2.sig";
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn http_cfg(extra: &[(&str, &str)]) -> GateConfig {
        let mut v = vec![
            ("ZORVIA_GOLDEN_FETCH_IMAGE", "curl:8"),
            ("ZORVIA_GOLDEN_SIGN_IMAGE", "cosign:2"),
            ("ZORVIA_GOLDEN_SIGN_KEY_SECRET", "ck"),
        ];
        v.extend_from_slice(extra);
        cfg(&v).unwrap()
    }

    fn checks(sum: Option<&str>, sig: Option<&str>) -> SourceChecks {
        SourceChecks {
            checksum_sha256: sum.map(String::from),
            signature_url: sig.map(String::from),
        }
    }

    #[test]
    fn credentials_config_and_job_wiring() {
        let c = cfg(&[
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "trivy:1"),
            ("ZORVIA_GOLDEN_PULL_SECRET", "regcred"),
            ("ZORVIA_GOLDEN_CDI_SECRET", "cdi-reg"),
            ("ZORVIA_GOLDEN_KEEP_ON_BLOCK", "1"),
        ])
        .unwrap();
        assert_eq!(c.cdi_secret.as_deref(), Some("cdi-reg"));
        assert!(c.keep_on_block);
        let j = build_scan_job("s", "ns", c.scan.as_ref().unwrap(), "r.io/a:1").unwrap();
        let pod = j.spec.unwrap().template.spec.unwrap();
        assert_eq!(pod.image_pull_secrets.unwrap()[0].name, "regcred");
        let vol = pod
            .volumes
            .unwrap()
            .into_iter()
            .find(|v| v.name == "registry-auth")
            .unwrap();
        let items = vol.secret.unwrap().items.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, ".dockerconfigjson");
        let env = pod.containers[0].env.clone().unwrap();
        assert!(env
            .iter()
            .any(|e| e.name == "DOCKER_CONFIG" && e.value.as_deref() == Some("/docker")));
        // no secret content anywhere in the spec, only names
        assert!(!serde_json::to_string(&pod.containers[0])
            .unwrap()
            .contains("auth\":"));
        // invalid names rejected; unset means no pull secret
        assert!(cfg(&[("ZORVIA_GOLDEN_PULL_SECRET", "Bad_Name")]).is_err());
        assert!(cfg(&[("ZORVIA_GOLDEN_CDI_SECRET", "a b")]).is_err());
        let plain = cfg(&[("ZORVIA_GOLDEN_SCAN_IMAGE", "trivy:1")]).unwrap();
        let j = build_scan_job("s", "ns", plain.scan.as_ref().unwrap(), "r.io/a:1").unwrap();
        assert!(j
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .image_pull_secrets
            .is_none());
    }

    #[test]
    fn url_and_checksum_validation() {
        assert!(validate_http_url("u", URL).is_ok());
        assert!(validate_http_url("u", "ftp://x/y").is_err());
        assert!(validate_http_url("u", "https://user:pw@host/y").is_err());
        assert!(validate_http_url("u", "https://host/a b").is_err());
        assert!(validate_http_url("u", "https:///y").is_err());
        assert_eq!(
            normalize_sha256(&format!("sha256:{}", DIGEST.to_uppercase())).unwrap(),
            DIGEST
        );
        assert!(normalize_sha256("abc").is_err());
        assert!(normalize_sha256(&"g".repeat(64)).is_err());
        assert!(cfg(&[("ZORVIA_GOLDEN_FETCH_IMAGE", "-x")]).is_err());
        assert!(cfg(&[("ZORVIA_GOLDEN_BLOB_MAX_SIZE", "lots")]).is_err());
    }

    #[test]
    fn blob_job_specs() {
        let c = http_cfg(&[]);
        let sign = c.sign.as_ref().unwrap();
        let j = build_blob_verify_job("v", "ns", sign, "curl:8", "50Gi", URL, SIG).unwrap();
        let pod = j.spec.unwrap().template.spec.unwrap();
        let init = &pod.init_containers.unwrap()[0];
        assert_eq!(init.image.as_deref(), Some("curl:8"));
        let cmd = init.command.clone().unwrap();
        // URL is a positional parameter, not part of the script text
        assert_eq!(cmd.last().unwrap(), URL);
        assert!(!cmd[2].contains("example.com"));
        let args = pod.containers[0].args.clone().unwrap();
        assert_eq!(
            args,
            [
                "verify-blob",
                "--key",
                "/cosign/cosign.pub",
                "--signature",
                SIG,
                "/blob/disk"
            ]
        );
        let blob = pod
            .volumes
            .unwrap()
            .into_iter()
            .find(|v| v.name == "blob")
            .unwrap();
        assert_eq!(blob.empty_dir.unwrap().size_limit.unwrap().0, "50Gi");
        let sum = build_checksum_job("c", "ns", "curl:8", None, URL).unwrap();
        let ct = &sum.spec.unwrap().template.spec.unwrap().containers[0];
        assert_eq!(ct.command.as_ref().unwrap().last().unwrap(), URL);
        assert!(build_checksum_job("c", "ns", "curl:8", None, "https://u:p@h/x").is_err());
        let keyless = SignConfig {
            image: "c".into(),
            mode: SignMode::Keyless {
                identity: "a".into(),
                issuer: "b".into(),
            },
            pull_secret: None,
        };
        assert!(build_blob_verify_job("v", "ns", &keyless, "curl:8", "1Gi", URL, SIG).is_err());
    }

    #[tokio::test]
    async fn http_checksum_and_signature_pass() {
        let r = Mock::new(vec![ok(&format!("{DIGEST}\n")), ok("")]);
        let rep = run_gates(
            &http_cfg(&[]),
            &r,
            "ns",
            "p",
            URL,
            &checks(Some(&format!("sha256:{DIGEST}")), Some(SIG)),
        )
        .await;
        assert!(rep.promote_allowed(), "{rep:?}");
        let names: Vec<_> = rep.steps.iter().map(|s| s.step.as_str()).collect();
        assert_eq!(names, ["checksum", "verify-blob"]);
    }

    #[tokio::test]
    async fn http_blocks_on_mismatch_garbage_and_failures() {
        let other = "f".repeat(64);
        for (outcome, want) in [
            (ok(&other), "checksum mismatch"),
            (ok("curl: (22) error"), "no SHA-256 digest"),
            (ok(""), "no SHA-256 digest"),
            (Err("boom".to_string()), "boom"),
            (
                Ok(JobOutcome {
                    succeeded: false,
                    logs: String::new(),
                }),
                "checksum Job failed",
            ),
        ] {
            let r = Mock::new(vec![outcome]);
            let rep = run_gates(
                &http_cfg(&[]),
                &r,
                "ns",
                "p",
                URL,
                &checks(Some(DIGEST), Some(SIG)),
            )
            .await;
            let reason = rep.blocked_reason.unwrap();
            assert!(
                reason.starts_with("checksum blocked promotion") && reason.contains(want),
                "{reason}"
            );
            assert_eq!(r.jobs.lock().unwrap().len(), 1);
        }
        // bad signature
        let r = Mock::new(vec![
            ok(DIGEST),
            Ok(JobOutcome {
                succeeded: false,
                logs: String::new(),
            }),
        ]);
        let rep = run_gates(
            &http_cfg(&[]),
            &r,
            "ns",
            "p",
            URL,
            &checks(Some(DIGEST), Some(SIG)),
        )
        .await;
        assert!(rep
            .blocked_reason
            .unwrap()
            .starts_with("verify-blob blocked promotion"));
    }

    #[tokio::test]
    async fn http_fails_closed_on_missing_inputs() {
        let r = Mock::new(vec![]);
        // sign configured but no signature URL
        let rep = run_gates(
            &http_cfg(&[]),
            &r,
            "ns",
            "p",
            URL,
            &checks(Some(DIGEST), None),
        )
        .await;
        assert!(!rep.promote_allowed());
        // signature URL with no sign gate
        let only_fetch = cfg(&[("ZORVIA_GOLDEN_FETCH_IMAGE", "curl:8")]).unwrap();
        let rep = run_gates(&only_fetch, &r, "ns", "p", URL, &checks(None, Some(SIG))).await;
        assert!(!rep.promote_allowed());
        // checksum without a fetch image
        let rep = run_gates(
            &GateConfig::default(),
            &r,
            "ns",
            "p",
            URL,
            &checks(Some(DIGEST), None),
        )
        .await;
        assert!(rep
            .blocked_reason
            .unwrap()
            .contains("ZORVIA_GOLDEN_FETCH_IMAGE"));
        // malformed checksum, URL with credentials
        let rep = run_gates(&only_fetch, &r, "ns", "p", URL, &checks(Some("zz"), None)).await;
        assert!(!rep.promote_allowed());
        let rep = run_gates(
            &only_fetch,
            &r,
            "ns",
            "p",
            "https://u:p@h/x",
            &checks(Some(DIGEST), None),
        )
        .await;
        assert!(!rep.promote_allowed());
        // checks on a registry source
        let rep = run_gates(
            &only_fetch,
            &r,
            "ns",
            "p",
            "r.io/a:1",
            &checks(Some(DIGEST), None),
        )
        .await;
        assert!(!rep.promote_allowed());
        assert!(r.jobs.lock().unwrap().is_empty());
        // nothing configured, nothing supplied: allowed as before
        let rep = run_gates(&GateConfig::default(), &r, "ns", "p", URL, &none()).await;
        assert!(rep.promote_allowed() && rep.steps.is_empty());
    }

    #[tokio::test]
    async fn http_scan_gate_needs_operator_opt_in() {
        let r = Mock::new(vec![ok(DIGEST)]);
        let base = [
            ("ZORVIA_GOLDEN_SCAN_IMAGE", "trivy:1"),
            ("ZORVIA_GOLDEN_FETCH_IMAGE", "curl:8"),
        ];
        let blocked = cfg(&base).unwrap();
        let rep = run_gates(&blocked, &r, "ns", "p", URL, &checks(Some(DIGEST), None)).await;
        assert!(rep
            .blocked_reason
            .unwrap()
            .starts_with("scan blocked promotion"));
        let mut allowed = base.to_vec();
        allowed.push(("ZORVIA_GOLDEN_HTTP_ALLOW_UNSCANNED", "1"));
        let allowed = cfg(&allowed).unwrap();
        let rep = run_gates(&allowed, &r, "ns", "p", URL, &checks(Some(DIGEST), None)).await;
        assert!(rep.promote_allowed());
        assert_eq!(rep.steps[0].step, "scan:skipped");
    }

    struct FakeStore {
        labels: Option<BTreeMap<String, String>>,
        deleted: Mutex<Vec<String>>,
        fail_delete: bool,
    }
    impl DataVolumeStore for FakeStore {
        async fn labels(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<BTreeMap<String, String>>, String> {
            Ok(self.labels.clone())
        }
        async fn delete(&self, _: &str, name: &str) -> Result<(), String> {
            if self.fail_delete {
                return Err("denied".into());
            }
            self.deleted.lock().unwrap().push(name.into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn cleanup_only_removes_this_runs_datavolume() {
        let lab = |v: &str| Some(BTreeMap::from([(RUN_LABEL.to_string(), v.to_string())]));
        let mk = |labels, fail_delete| FakeStore {
            labels,
            deleted: Mutex::new(vec![]),
            fail_delete,
        };
        let s = mk(lab("run-1"), false);
        let (gone, _) = cleanup_blocked_import(&s, "ns", "img-1", "run-1").await;
        assert!(gone && s.deleted.lock().unwrap().as_slice() == ["img-1"]);
        // another run's DataVolume, an unlabelled one, a missing one, a failing delete
        for (store, want) in [
            (mk(lab("run-2"), false), "label"),
            (mk(Some(BTreeMap::new()), false), "label"),
            (mk(None, false), "already gone"),
            (mk(lab("run-1"), true), "cleanup failed"),
        ] {
            let (gone, note) = cleanup_blocked_import(&store, "ns", "img-1", "run-1").await;
            assert!(!gone && note.contains(want), "{note}");
            assert!(store.deleted.lock().unwrap().is_empty());
        }
    }
}
