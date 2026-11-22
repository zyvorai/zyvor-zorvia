// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Certificate lifecycle (Experimental, `cert-lifecycle`): an inventory of the X.509
//! certificates Zorvia can see, expiry alerts, and a cert-manager renew action.
//!
//! * [`inspect`] parses certificates with OpenSSL (already a dependency; no new crate).
//! * [`expiry`] decides which alerts are due, once per threshold per certificate.
//! * [`certmanager`] reads cert-manager `Certificate` objects and triggers a renewal.
//! * [`kube_source`] is the Kubernetes-backed [`CertSource`]; tests use an in-memory one.
//!
//! Hard rules: only `tls.crt` / `ca.crt` are ever read from a Secret (never `tls.key` or
//! any other key), and nothing returned by this module contains PEM text or key material.
//! See docs/CERTIFICATE_LIFECYCLE.md.

use chrono::{DateTime, Utc};
use serde::Serialize;

pub mod certmanager;
pub mod expiry;
pub mod inspect;
pub mod kube_source;

use inspect::CertMeta;

pub const FEATURE_ID: &str = "cert-lifecycle";

/// Public certificate PEMs read from one `kubernetes.io/tls` Secret.
#[derive(Debug, Default, Clone)]
pub struct RawSecret {
    pub namespace: String,
    pub name: String,
    pub tls_crt: Option<Vec<u8>>,
    pub ca_crt: Option<Vec<u8>>,
}

/// Result of listing TLS Secrets: what was readable and what was not.
#[derive(Debug, Default, Clone)]
pub struct SecretScan {
    pub secrets: Vec<RawSecret>,
    /// Human-readable notes about namespaces or objects that could not be read.
    pub skipped: Vec<String>,
}

/// The cluster access the feature needs. Implemented by [`kube_source::KubeCertSource`]
/// and by an in-memory fake in the tests.
#[allow(async_fn_in_trait)]
pub trait CertSource {
    async fn list_tls_secrets(&self) -> anyhow::Result<SecretScan>;
    /// All cert-manager `Certificate` objects, or `None` when the CRD is not installed.
    async fn list_certmanager(&self) -> anyhow::Result<Option<Vec<serde_json::Value>>>;
    async fn get_certmanager(
        &self,
        namespace: &str,
        name: &str,
    ) -> anyhow::Result<Option<serde_json::Value>>;
    /// Replace the object through the `status` subresource (optimistic concurrency via the
    /// object's `resourceVersion`).
    async fn replace_certmanager_status(
        &self,
        namespace: &str,
        name: &str,
        object: &serde_json::Value,
    ) -> anyhow::Result<()>;
}

/// Where a certificate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    TlsSecret,
    ServingCert,
    CaFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// First certificate of a `tls.crt` or the serving certificate.
    Leaf,
    /// Later certificates of a `tls.crt` (intermediates).
    Chain,
    /// `ca.crt` of a Secret or a configured CA bundle file.
    Ca,
}

/// One certificate in the inventory: metadata only.
#[derive(Debug, Clone, Serialize)]
pub struct CertInfo {
    pub source: Source,
    pub namespace: Option<String>,
    /// Secret name, or the configured purpose of a file (for example `database-ca`).
    pub name: String,
    /// Secret data key or environment variable it was read from.
    pub key: String,
    pub role: Role,
    pub index: usize,
    #[serde(flatten)]
    pub meta: CertMeta,
    /// Summary flags: `expired`, `not-yet-valid`, `self-signed`, `weak-signature:..`, `weak-key:..`.
    pub flags: Vec<String>,
}

impl CertInfo {
    /// Stable identity used by expiry alerting (independent of the certificate content).
    pub fn id(&self) -> String {
        format!(
            "{:?}/{}/{}/{}#{}",
            self.source,
            self.namespace.as_deref().unwrap_or("-"),
            self.name,
            self.key,
            self.index
        )
    }
    /// Intermediates are listed but not alerted on.
    pub fn monitored(&self) -> bool {
        self.role != Role::Chain
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub total: usize,
    pub expired: usize,
    /// Not expired, fewer than 30 days left.
    pub expiring_30d: usize,
    pub self_signed: usize,
    pub weak: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub generated_at: DateTime<Utc>,
    pub certificates: Vec<CertInfo>,
    pub summary: Summary,
    pub skipped: Vec<String>,
}

/// A file-backed certificate Zorvia knows about.
#[derive(Debug, Clone)]
pub struct FileSpec {
    pub name: &'static str,
    pub env: &'static str,
    pub source: Source,
    pub role: Role,
    pub path: String,
}

/// Certificates named by Zorvia's own configuration: the serving certificate and the CA
/// files of the outgoing connections that have one. OIDC, Vault and S3 have no CA-file
/// setting today, so there is nothing to read for them.
pub fn file_specs_from_env() -> Vec<FileSpec> {
    let table: [(&str, &str, Source, Role); 3] = [
        (
            "serving-cert",
            "ZORVIA_TLS_CERT",
            Source::ServingCert,
            Role::Leaf,
        ),
        (
            "cdi-upload-proxy-ca",
            "ZORVIA_CDI_UPLOAD_PROXY_CA_FILE",
            Source::CaFile,
            Role::Ca,
        ),
        (
            "database-ca",
            "ZORVIA_DATABASE_CA_FILE",
            Source::CaFile,
            Role::Ca,
        ),
    ];
    table
        .into_iter()
        .filter_map(|(name, env, source, role)| {
            let p = std::env::var(env).ok()?;
            let p = p.trim();
            (!p.is_empty()).then(|| FileSpec {
                name,
                env,
                source,
                role,
                path: p.to_string(),
            })
        })
        .collect()
}

/// Read the files (size-capped). Unreadable files become `skipped` notes, never errors.
pub async fn read_files(specs: &[FileSpec]) -> (Vec<(FileSpec, Vec<u8>)>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for s in specs {
        let too_big = match tokio::fs::metadata(&s.path).await {
            Ok(m) => m.len() as usize > inspect::MAX_PEM_BYTES,
            Err(_) => {
                skipped.push(format!("{}: file from {} is not readable", s.name, s.env));
                continue;
            }
        };
        if too_big {
            skipped.push(format!("{}: file from {} is too large", s.name, s.env));
            continue;
        }
        match tokio::fs::read(&s.path).await {
            Ok(b) => out.push((s.clone(), b)),
            Err(_) => skipped.push(format!("{}: file from {} is not readable", s.name, s.env)),
        }
    }
    (out, skipped)
}

fn flags_for(m: &CertMeta) -> Vec<String> {
    let mut f = Vec::new();
    if m.expired {
        f.push("expired".to_string());
    }
    if m.not_yet_valid {
        f.push("not-yet-valid".into());
    }
    if m.self_signed {
        f.push("self-signed".into());
    }
    f.extend(m.weak.iter().cloned());
    f
}

#[allow(clippy::too_many_arguments)]
fn push_pem(
    out: &mut Vec<CertInfo>,
    skipped: &mut Vec<String>,
    pem: &[u8],
    source: Source,
    namespace: Option<&str>,
    name: &str,
    key: &str,
    first_role: Role,
    now: DateTime<Utc>,
) {
    match inspect::parse_pem(pem, now) {
        Ok(metas) => {
            for (index, meta) in metas.into_iter().enumerate() {
                let role = if index == 0 {
                    first_role
                } else if first_role == Role::Ca {
                    Role::Ca
                } else {
                    Role::Chain
                };
                let flags = flags_for(&meta);
                out.push(CertInfo {
                    source,
                    namespace: namespace.map(str::to_string),
                    name: name.to_string(),
                    key: key.to_string(),
                    role,
                    index,
                    meta,
                    flags,
                });
            }
        }
        Err(e) => skipped.push(format!(
            "{}{name}: {key} could not be parsed ({e})",
            namespace.map(|n| format!("{n}/")).unwrap_or_default()
        )),
    }
}

/// Pure inventory builder (no cluster or filesystem access).
pub fn build_inventory(
    scan: SecretScan,
    files: &[(FileSpec, Vec<u8>)],
    mut skipped: Vec<String>,
    namespace_filter: Option<&str>,
    now: DateTime<Utc>,
) -> Inventory {
    let mut certs = Vec::new();
    skipped.extend(scan.skipped);
    for s in &scan.secrets {
        if namespace_filter.is_some_and(|n| n != s.namespace) {
            continue;
        }
        match &s.tls_crt {
            Some(p) => push_pem(
                &mut certs,
                &mut skipped,
                p,
                Source::TlsSecret,
                Some(&s.namespace),
                &s.name,
                "tls.crt",
                Role::Leaf,
                now,
            ),
            None => skipped.push(format!("{}/{}: no tls.crt", s.namespace, s.name)),
        }
        if let Some(p) = &s.ca_crt {
            push_pem(
                &mut certs,
                &mut skipped,
                p,
                Source::TlsSecret,
                Some(&s.namespace),
                &s.name,
                "ca.crt",
                Role::Ca,
                now,
            );
        }
    }
    if namespace_filter.is_none() {
        for (spec, bytes) in files {
            push_pem(
                &mut certs,
                &mut skipped,
                bytes,
                spec.source,
                None,
                spec.name,
                spec.env,
                spec.role,
                now,
            );
        }
    }
    certs.sort_by(|a, b| {
        a.meta
            .not_after
            .cmp(&b.meta.not_after)
            .then_with(|| a.id().cmp(&b.id()))
    });
    let summary = Summary {
        total: certs.len(),
        expired: certs.iter().filter(|c| c.meta.expired).count(),
        expiring_30d: certs
            .iter()
            .filter(|c| !c.meta.expired && c.meta.days_remaining < 30)
            .count(),
        self_signed: certs.iter().filter(|c| c.meta.self_signed).count(),
        weak: certs.iter().filter(|c| !c.meta.weak.is_empty()).count(),
    };
    Inventory {
        generated_at: now,
        certificates: certs,
        summary,
        skipped,
    }
}

/// Collect everything the source and the configured files offer.
pub async fn collect(
    source: &impl CertSource,
    files: &[FileSpec],
    namespace_filter: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Inventory> {
    let scan = source.list_tls_secrets().await?;
    let (loaded, skipped) = read_files(files).await;
    Ok(build_inventory(
        scan,
        &loaded,
        skipped,
        namespace_filter,
        now,
    ))
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// In-memory stand-in for the cluster.
    #[derive(Default)]
    pub struct FakeSource {
        pub scan: SecretScan,
        pub scan_error: Option<String>,
        /// `None` = cert-manager CRD not installed.
        pub certmanager: Mutex<Option<Vec<serde_json::Value>>>,
        pub replaced: Mutex<Vec<(String, String, serde_json::Value)>>,
        pub replace_error: Option<String>,
    }

    impl CertSource for FakeSource {
        async fn list_tls_secrets(&self) -> anyhow::Result<SecretScan> {
            match &self.scan_error {
                Some(e) => anyhow::bail!("{e}"),
                None => Ok(self.scan.clone()),
            }
        }
        async fn list_certmanager(&self) -> anyhow::Result<Option<Vec<serde_json::Value>>> {
            Ok(self.certmanager.lock().unwrap().clone())
        }
        async fn get_certmanager(
            &self,
            namespace: &str,
            name: &str,
        ) -> anyhow::Result<Option<serde_json::Value>> {
            Ok(self
                .certmanager
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|l| {
                    l.iter().find(|o| {
                        o["metadata"]["namespace"] == namespace && o["metadata"]["name"] == name
                    })
                })
                .cloned())
        }
        async fn replace_certmanager_status(
            &self,
            namespace: &str,
            name: &str,
            object: &serde_json::Value,
        ) -> anyhow::Result<()> {
            if let Some(e) = &self.replace_error {
                anyhow::bail!("{e}");
            }
            self.replaced
                .lock()
                .unwrap()
                .push((namespace.into(), name.into(), object.clone()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeSource;
    use super::inspect::testutil::*;
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }
    fn at(days: i64) -> i64 {
        now().timestamp() + days * 86_400
    }
    fn leaf_pem(cn: &str, days: i64) -> Vec<u8> {
        let k = key(&Key::EcP256);
        pem(&cert(
            &Spec {
                cn,
                dns: &[cn],
                ip: None,
                not_before: at(-30),
                not_after: at(days) + 600,
                digest: None,
            },
            &k,
            None,
        ))
    }

    #[test]
    fn inventory_lists_metadata_and_flags() {
        let weak_key = key(&Key::Rsa(1024));
        let weak = pem(&cert(
            &Spec {
                cn: "weak.test",
                dns: &[],
                ip: None,
                not_before: at(-5),
                not_after: at(400),
                digest: Some(openssl::hash::MessageDigest::sha1()),
            },
            &weak_key,
            None,
        ));
        let scan = SecretScan {
            secrets: vec![
                RawSecret {
                    namespace: "a".into(),
                    name: "web".into(),
                    tls_crt: Some(leaf_pem("web.test", 10)),
                    ca_crt: None,
                },
                RawSecret {
                    namespace: "b".into(),
                    name: "old".into(),
                    tls_crt: Some(leaf_pem("old.test", -2)),
                    ca_crt: None,
                },
                RawSecret {
                    namespace: "b".into(),
                    name: "weak".into(),
                    tls_crt: Some(weak),
                    ca_crt: Some(leaf_pem("ca.test", 800)),
                },
                RawSecret {
                    namespace: "b".into(),
                    name: "broken".into(),
                    tls_crt: Some(b"garbage".to_vec()),
                    ca_crt: None,
                },
                RawSecret {
                    namespace: "b".into(),
                    name: "empty".into(),
                    tls_crt: None,
                    ca_crt: None,
                },
            ],
            skipped: vec!["namespace c: not permitted".into()],
        };
        let inv = build_inventory(scan, &[], vec![], None, now());
        assert_eq!(inv.summary.total, 4);
        assert_eq!(inv.summary.expired, 1);
        assert_eq!(inv.summary.expiring_30d, 1);
        assert_eq!(inv.summary.weak, 1);
        assert_eq!(inv.summary.self_signed, 4);
        // Soonest expiry first.
        assert_eq!(inv.certificates[0].name, "old");
        assert_eq!(inv.skipped.len(), 3);
        let weak = inv
            .certificates
            .iter()
            .find(|c| c.name == "weak" && c.key == "tls.crt")
            .unwrap();
        assert!(weak.flags.iter().any(|f| f.starts_with("weak-signature")));
        let ca = inv.certificates.iter().find(|c| c.key == "ca.crt").unwrap();
        assert_eq!(ca.role, Role::Ca);
    }

    #[test]
    fn namespace_filter_and_json_has_no_pem() {
        let scan = SecretScan {
            secrets: vec![
                RawSecret {
                    namespace: "a".into(),
                    name: "x".into(),
                    tls_crt: Some(leaf_pem("x.test", 50)),
                    ca_crt: None,
                },
                RawSecret {
                    namespace: "b".into(),
                    name: "y".into(),
                    tls_crt: Some(leaf_pem("y.test", 50)),
                    ca_crt: None,
                },
            ],
            skipped: vec![],
        };
        let inv = build_inventory(scan, &[], vec![], Some("a"), now());
        assert_eq!(inv.certificates.len(), 1);
        let json = serde_json::to_string(&inv).unwrap();
        assert!(!json.contains("BEGIN") && !json.contains("PRIVATE"));
        assert!(json.contains("\"days_remaining\":50"));
        assert!(json.contains("\"source\":\"tls-secret\""));
    }

    #[test]
    fn chain_certificates_are_not_monitored() {
        let ca_k = key(&Key::EcP256);
        let ca = cert(
            &Spec {
                cn: "int",
                dns: &[],
                ip: None,
                not_before: at(-5),
                not_after: at(300),
                digest: None,
            },
            &ca_k,
            None,
        );
        let lk = key(&Key::EcP256);
        let leaf = cert(
            &Spec {
                cn: "leaf",
                dns: &[],
                ip: None,
                not_before: at(-5),
                not_after: at(30),
                digest: None,
            },
            &lk,
            Some((&ca, &ca_k)),
        );
        let mut p = pem(&leaf);
        p.extend(pem(&ca));
        let scan = SecretScan {
            secrets: vec![RawSecret {
                namespace: "a".into(),
                name: "x".into(),
                tls_crt: Some(p),
                ca_crt: None,
            }],
            skipped: vec![],
        };
        let inv = build_inventory(scan, &[], vec![], None, now());
        assert_eq!(inv.certificates.len(), 2);
        assert_eq!(inv.certificates.iter().filter(|c| c.monitored()).count(), 1);
    }

    #[tokio::test]
    async fn files_are_read_and_unreadable_ones_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("tls.crt");
        std::fs::write(&good, leaf_pem("serving.test", 90)).unwrap();
        let specs = vec![
            FileSpec {
                name: "serving-cert",
                env: "ZORVIA_TLS_CERT",
                source: Source::ServingCert,
                role: Role::Leaf,
                path: good.to_string_lossy().into(),
            },
            FileSpec {
                name: "database-ca",
                env: "ZORVIA_DATABASE_CA_FILE",
                source: Source::CaFile,
                role: Role::Ca,
                path: dir.path().join("nope").to_string_lossy().into(),
            },
        ];
        let fake = FakeSource::default();
        let inv = collect(&fake, &specs, None, now()).await.unwrap();
        assert_eq!(inv.certificates.len(), 1);
        assert_eq!(inv.certificates[0].source, Source::ServingCert);
        assert_eq!(inv.certificates[0].namespace, None);
        assert_eq!(inv.skipped.len(), 1);
        // The configured path itself is not echoed.
        assert!(!inv.skipped[0].contains(dir.path().to_str().unwrap()));
    }

    #[tokio::test]
    async fn source_error_propagates() {
        let fake = FakeSource {
            scan_error: Some("boom".into()),
            ..Default::default()
        };
        assert!(collect(&fake, &[], None, now()).await.is_err());
    }
}
