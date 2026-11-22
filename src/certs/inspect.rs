// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! X.509 metadata extraction (OpenSSL, already a dependency). Only public certificate
//! fields are read; a [`CertMeta`] holds no PEM text and no key material.

use chrono::{DateTime, TimeZone, Utc};
use openssl::asn1::{Asn1Time, Asn1TimeRef};
use openssl::hash::MessageDigest;
use openssl::pkey::Id;
use openssl::x509::X509;
use serde::Serialize;

/// Largest PEM input parsed (a Secret value or a CA file).
pub const MAX_PEM_BYTES: usize = 1024 * 1024;
/// Most certificates read from one PEM bundle.
pub const MAX_CERTS_PER_PEM: usize = 64;

/// Parsed, non-secret facts about one certificate.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CertMeta {
    pub subject: String,
    pub issuer: String,
    pub sans: Vec<String>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// Whole days until `not_after` (negative once expired), rounded down.
    pub days_remaining: i64,
    pub expired: bool,
    pub not_yet_valid: bool,
    /// "RSA", "EC", "Ed25519", ...
    pub key_type: String,
    /// RSA modulus size, or the curve's field size for EC.
    pub key_bits: u32,
    /// EC curve short name (for example `prime256v1`).
    pub curve: Option<String>,
    pub signature_algorithm: String,
    pub self_signed: bool,
    /// SHA-256 of the DER encoding, lowercase hex.
    pub fingerprint_sha256: String,
    /// Weak-algorithm findings (empty when none).
    pub weak: Vec<String>,
}

/// Parse every certificate in `pem`. Fails on empty, oversized or non-certificate input.
pub fn parse_pem(pem: &[u8], now: DateTime<Utc>) -> Result<Vec<CertMeta>, String> {
    if pem.len() > MAX_PEM_BYTES {
        return Err(format!("input larger than {MAX_PEM_BYTES} bytes"));
    }
    let certs = X509::stack_from_pem(pem).map_err(|_| "not a valid PEM certificate".to_string())?;
    if certs.is_empty() {
        return Err("no certificate found".into());
    }
    if certs.len() > MAX_CERTS_PER_PEM {
        return Err(format!("more than {MAX_CERTS_PER_PEM} certificates"));
    }
    certs.iter().map(|c| meta(c, now)).collect()
}

fn unix(t: &Asn1TimeRef) -> Result<i64, String> {
    let epoch = Asn1Time::from_unix(0).map_err(|e| e.to_string())?;
    let d = epoch.diff(t).map_err(|e| e.to_string())?;
    Ok(i64::from(d.days) * 86_400 + i64::from(d.secs))
}

fn name_string(n: &openssl::x509::X509NameRef) -> String {
    let parts: Vec<String> = n
        .entries()
        .filter_map(|e| {
            let key = e.object().nid().short_name().ok()?;
            let val = e.data().to_string().ok()?;
            Some(format!("{key}={val}"))
        })
        .collect();
    parts.join(", ")
}

fn ip_string(b: &[u8]) -> Option<String> {
    match b.len() {
        4 => Some(std::net::Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string()),
        16 => {
            let a: [u8; 16] = b.try_into().ok()?;
            Some(std::net::Ipv6Addr::from(a).to_string())
        }
        _ => None,
    }
}

fn meta(c: &X509, now: DateTime<Utc>) -> Result<CertMeta, String> {
    let nb = unix(c.not_before())?;
    let na = unix(c.not_after())?;
    let to_dt = |s: i64| {
        Utc.timestamp_opt(s, 0)
            .single()
            .ok_or_else(|| "certificate date out of range".to_string())
    };
    let remaining = na - now.timestamp();
    let days_remaining = remaining.div_euclid(86_400);

    let mut sans = Vec::new();
    if let Some(list) = c.subject_alt_names() {
        for g in list.iter() {
            if let Some(d) = g.dnsname() {
                sans.push(format!("DNS:{d}"));
            } else if let Some(ip) = g.ipaddress().and_then(ip_string) {
                sans.push(format!("IP:{ip}"));
            } else if let Some(e) = g.email() {
                sans.push(format!("email:{e}"));
            } else if let Some(u) = g.uri() {
                sans.push(format!("URI:{u}"));
            }
        }
    }

    let pkey = c.public_key().map_err(|e| e.to_string())?;
    let (key_type, curve) = match pkey.id() {
        Id::RSA => ("RSA".to_string(), None),
        Id::DSA => ("DSA".to_string(), None),
        Id::EC => {
            let curve = pkey
                .ec_key()
                .ok()
                .and_then(|k| k.group().curve_name())
                .and_then(|n| n.short_name().ok())
                .map(str::to_string);
            ("EC".to_string(), curve)
        }
        Id::ED25519 => ("Ed25519".to_string(), None),
        Id::ED448 => ("Ed448".to_string(), None),
        other => (format!("{other:?}"), None),
    };
    let key_bits = pkey.bits();
    let signature_algorithm = c
        .signature_algorithm()
        .object()
        .nid()
        .short_name()
        .unwrap_or("unknown")
        .to_string();

    let self_signed = match (c.subject_name().to_der(), c.issuer_name().to_der()) {
        (Ok(s), Ok(i)) if s == i => c.verify(&pkey).unwrap_or(false),
        _ => false,
    };

    let fingerprint_sha256 = c
        .digest(MessageDigest::sha256())
        .map(|d| d.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .map_err(|e| e.to_string())?;

    let mut weak = Vec::new();
    let sig_lower = signature_algorithm.to_ascii_lowercase();
    if ["md2", "md4", "md5", "sha1"]
        .iter()
        .any(|w| sig_lower.contains(w))
    {
        weak.push(format!("weak-signature:{signature_algorithm}"));
    }
    match pkey.id() {
        Id::RSA if key_bits < 2048 => weak.push(format!("weak-key:RSA-{key_bits}")),
        Id::DSA => weak.push("weak-key:DSA".into()),
        Id::EC if key_bits < 224 => weak.push(format!("weak-key:EC-{key_bits}")),
        _ => {}
    }

    Ok(CertMeta {
        subject: name_string(c.subject_name()),
        issuer: name_string(c.issuer_name()),
        sans,
        not_before: to_dt(nb)?,
        not_after: to_dt(na)?,
        days_remaining,
        expired: remaining < 0,
        not_yet_valid: nb > now.timestamp(),
        key_type,
        key_bits,
        curve,
        signature_algorithm,
        self_signed,
        fingerprint_sha256,
        weak,
    })
}

/// Certificate generation for tests (OpenSSL builders, fixed dates).
#[cfg(test)]
pub(crate) mod testutil {
    use openssl::asn1::Asn1Time;
    use openssl::bn::BigNum;
    use openssl::ec::{EcGroup, EcKey};
    use openssl::hash::MessageDigest;
    use openssl::nid::Nid;
    use openssl::pkey::{PKey, Private};
    use openssl::rsa::Rsa;
    use openssl::x509::extension::SubjectAlternativeName;
    use openssl::x509::{X509Builder, X509NameBuilder, X509};

    pub enum Key {
        Rsa(u32),
        EcP256,
        Ed25519,
    }

    pub fn key(k: &Key) -> PKey<Private> {
        match k {
            Key::Rsa(bits) => PKey::from_rsa(Rsa::generate(*bits).unwrap()).unwrap(),
            Key::EcP256 => {
                let g = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
                PKey::from_ec_key(EcKey::generate(&g).unwrap()).unwrap()
            }
            Key::Ed25519 => PKey::generate_ed25519().unwrap(),
        }
    }

    pub struct Spec<'a> {
        pub cn: &'a str,
        pub dns: &'a [&'a str],
        pub ip: Option<&'a str>,
        pub not_before: i64,
        pub not_after: i64,
        pub digest: Option<MessageDigest>,
    }

    /// Self-signed when `issuer` is `None`, otherwise signed by the given CA.
    pub fn cert(
        spec: &Spec,
        subject_key: &PKey<Private>,
        issuer: Option<(&X509, &PKey<Private>)>,
    ) -> X509 {
        let name = |cn: &str| {
            let mut n = X509NameBuilder::new().unwrap();
            n.append_entry_by_text("CN", cn).unwrap();
            n.build()
        };
        let mut b = X509Builder::new().unwrap();
        b.set_version(2).unwrap();
        let serial = BigNum::from_u32(rand::random::<u32>() | 1).unwrap();
        b.set_serial_number(&serial.to_asn1_integer().unwrap())
            .unwrap();
        let subj = name(spec.cn);
        b.set_subject_name(&subj).unwrap();
        match issuer {
            Some((ca, _)) => b.set_issuer_name(ca.subject_name()).unwrap(),
            None => b.set_issuer_name(&subj).unwrap(),
        }
        b.set_pubkey(subject_key).unwrap();
        b.set_not_before(&Asn1Time::from_unix(spec.not_before).unwrap())
            .unwrap();
        b.set_not_after(&Asn1Time::from_unix(spec.not_after).unwrap())
            .unwrap();
        if !spec.dns.is_empty() || spec.ip.is_some() {
            let mut san = SubjectAlternativeName::new();
            for d in spec.dns {
                san.dns(d);
            }
            if let Some(ip) = spec.ip {
                san.ip(ip);
            }
            let ext = san.build(&b.x509v3_context(None, None)).unwrap();
            b.append_extension(ext).unwrap();
        }
        let signer = issuer.map(|(_, k)| k).unwrap_or(subject_key);
        let is_ed = signer.id() == openssl::pkey::Id::ED25519;
        let md = if is_ed {
            MessageDigest::null()
        } else {
            spec.digest.unwrap_or_else(MessageDigest::sha256)
        };
        b.sign(signer, md).unwrap();
        b.build()
    }

    pub fn pem(c: &X509) -> Vec<u8> {
        c.to_pem().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use openssl::hash::MessageDigest;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }
    fn at(days: i64) -> i64 {
        now().timestamp() + days * 86_400
    }

    #[test]
    fn parses_subject_sans_dates_and_key() {
        let k = key(&Key::Rsa(2048));
        let c = cert(
            &Spec {
                cn: "zorvia.example.com",
                dns: &["zorvia.example.com", "alt.example.com"],
                ip: Some("10.1.2.3"),
                not_before: at(-10),
                not_after: at(20) + 3600,
                digest: None,
            },
            &k,
            None,
        );
        let m = &parse_pem(&pem(&c), now()).unwrap()[0];
        assert_eq!(m.subject, "CN=zorvia.example.com");
        assert_eq!(m.issuer, "CN=zorvia.example.com");
        assert!(m.sans.contains(&"DNS:alt.example.com".to_string()));
        assert!(m.sans.contains(&"IP:10.1.2.3".to_string()));
        assert_eq!(m.days_remaining, 20);
        assert!(!m.expired && !m.not_yet_valid);
        assert_eq!(m.key_type, "RSA");
        assert_eq!(m.key_bits, 2048);
        assert!(m.self_signed);
        assert!(m.weak.is_empty());
        assert_eq!(m.not_after.timestamp(), at(20) + 3600);
        assert_eq!(m.fingerprint_sha256.len(), 64);
    }

    #[test]
    fn ca_signed_leaf_is_not_self_signed_and_chain_is_parsed() {
        let ca_k = key(&Key::EcP256);
        let ca = cert(
            &Spec {
                cn: "Test CA",
                dns: &[],
                ip: None,
                not_before: at(-100),
                not_after: at(900),
                digest: None,
            },
            &ca_k,
            None,
        );
        let leaf_k = key(&Key::EcP256);
        let leaf = cert(
            &Spec {
                cn: "leaf",
                dns: &["leaf.test"],
                ip: None,
                not_before: at(-1),
                not_after: at(89),
                digest: None,
            },
            &leaf_k,
            Some((&ca, &ca_k)),
        );
        let mut bundle = pem(&leaf);
        bundle.extend(pem(&ca));
        let metas = parse_pem(&bundle, now()).unwrap();
        assert_eq!(metas.len(), 2);
        assert!(!metas[0].self_signed);
        assert_eq!(metas[0].issuer, "CN=Test CA");
        assert!(metas[1].self_signed);
        assert_eq!(metas[0].key_type, "EC");
        assert_eq!(metas[0].curve.as_deref(), Some("prime256v1"));
        assert_eq!(metas[0].key_bits, 256);
    }

    #[test]
    fn expired_and_not_yet_valid() {
        let k = key(&Key::EcP256);
        let old = cert(
            &Spec {
                cn: "old",
                dns: &[],
                ip: None,
                not_before: at(-400),
                not_after: at(-3) - 100,
                digest: None,
            },
            &k,
            None,
        );
        let m = &parse_pem(&pem(&old), now()).unwrap()[0];
        assert!(m.expired);
        assert_eq!(m.days_remaining, -4);
        let future = cert(
            &Spec {
                cn: "future",
                dns: &[],
                ip: None,
                not_before: at(5),
                not_after: at(100),
                digest: None,
            },
            &k,
            None,
        );
        assert!(parse_pem(&pem(&future), now()).unwrap()[0].not_yet_valid);
    }

    #[test]
    fn flags_weak_algorithms() {
        let k = key(&Key::Rsa(1024));
        let c = cert(
            &Spec {
                cn: "weak",
                dns: &[],
                ip: None,
                not_before: at(-1),
                not_after: at(10),
                digest: Some(MessageDigest::sha1()),
            },
            &k,
            None,
        );
        let m = &parse_pem(&pem(&c), now()).unwrap()[0];
        assert!(m.weak.iter().any(|w| w.starts_with("weak-signature:")));
        assert!(m.weak.contains(&"weak-key:RSA-1024".to_string()));
    }

    #[test]
    fn ed25519_is_reported() {
        let k = key(&Key::Ed25519);
        let c = cert(
            &Spec {
                cn: "ed",
                dns: &[],
                ip: None,
                not_before: at(-1),
                not_after: at(10),
                digest: None,
            },
            &k,
            None,
        );
        let m = &parse_pem(&pem(&c), now()).unwrap()[0];
        assert_eq!(m.key_type, "Ed25519");
        assert!(m.weak.is_empty());
    }

    #[test]
    fn rejects_garbage_private_keys_and_oversize() {
        assert!(parse_pem(b"", now()).is_err());
        assert!(parse_pem(b"not pem at all", now()).is_err());
        let k = key(&Key::EcP256);
        let key_pem = k.private_key_to_pem_pkcs8().unwrap();
        // A private key alone is never treated as a certificate and never echoed.
        let err = parse_pem(&key_pem, now()).unwrap_err();
        assert!(!err.contains("PRIVATE"));
        assert!(parse_pem(&vec![b'a'; MAX_PEM_BYTES + 1], now()).is_err());
    }

    #[test]
    fn metadata_serialisation_has_no_pem_or_key_text() {
        let k = key(&Key::EcP256);
        let c = cert(
            &Spec {
                cn: "x",
                dns: &[],
                ip: None,
                not_before: at(-1),
                not_after: at(10),
                digest: None,
            },
            &k,
            None,
        );
        let json = serde_json::to_string(&parse_pem(&pem(&c), now()).unwrap()).unwrap();
        assert!(!json.contains("BEGIN"));
    }
}
