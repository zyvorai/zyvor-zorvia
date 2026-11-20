// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! AWS Signature Version 4 for S3-compatible object stores (AWS S3, MinIO,
//! Ceph RGW). Only depends on `sha2` and `chrono`; HMAC-SHA256 is
//! implemented here (RFC 2104) rather than adding a crate.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn to_array(d: impl AsRef<[u8]>) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&to_array(Sha256::digest(key)));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner = to_array(inner.finalize());
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    to_array(outer.finalize())
}

#[derive(Clone)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .finish()
    }
}

/// Percent-encode per SigV4 (unreserved characters stay; `/` stays only
/// when `encode_slash` is false, i.e. for the canonical URI path).
pub fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn amz_date(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Sign a request. `extra_headers` are additional headers to sign and send
/// (names are lowercased). Returns every header the caller must set:
/// `authorization`, `x-amz-date`, `x-amz-content-sha256`, the optional
/// `x-amz-security-token`, plus the extras. `host` is signed but left to the
/// HTTP client to send.
#[allow(clippy::too_many_arguments)]
pub fn sign(
    creds: &Credentials,
    region: &str,
    method: &str,
    host: &str,
    path: &str,
    query: &[(String, String)],
    extra_headers: &[(String, String)],
    payload_sha256_hex: &str,
    now: DateTime<Utc>,
) -> Vec<(String, String)> {
    let date_time = amz_date(now);
    let date = &date_time[..8];

    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), host.to_string()),
        (
            "x-amz-content-sha256".into(),
            payload_sha256_hex.to_string(),
        ),
        ("x-amz-date".into(), date_time.clone()),
    ];
    if let Some(tok) = &creds.session_token {
        headers.push(("x-amz-security-token".into(), tok.clone()));
    }
    for (k, v) in extra_headers {
        headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let mut q: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
        .collect();
    q.sort();
    let canonical_query = q
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_request = format!(
        "{method}\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_sha256_hex}",
        uri_encode(path, false)
    );

    let scope = format!("{date}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{date_time}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(
        format!("AWS4{}", creds.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        creds.access_key
    );

    let mut out: Vec<(String, String)> = headers.into_iter().filter(|(k, _)| k != "host").collect();
    out.push(("authorization".into(), authorization));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn hmac_matches_rfc4231_vectors() {
        // Test case 1
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: key longer than the block size
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn matches_aws_documented_get_object_example() {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html
        let creds = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let headers = sign(
            &creds,
            "us-east-1",
            "GET",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            &[],
            &[("range".into(), "bytes=0-9".into())],
            &sha256_hex(b""),
            now,
        );
        let auth = headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn uri_encoding_follows_sigv4_rules() {
        assert_eq!(uri_encode("a b/c~d", false), "a%20b/c~d");
        assert_eq!(uri_encode("a b/c", true), "a%20b%2Fc");
        assert_eq!(uri_encode("é", true), "%C3%A9");
    }

    #[test]
    fn session_token_is_signed_and_returned() {
        let creds = Credentials {
            access_key: "AK".into(),
            secret_key: "SK".into(),
            session_token: Some("TOKEN".into()),
        };
        let h = sign(
            &creds,
            "r",
            "PUT",
            "h",
            "/b/k",
            &[("partNumber".into(), "1".into())],
            &[],
            &sha256_hex(b"x"),
            Utc::now(),
        );
        assert!(h
            .iter()
            .any(|(k, v)| k == "x-amz-security-token" && v == "TOKEN"));
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(auth.contains("x-amz-security-token"));
    }

    #[test]
    fn credentials_debug_hides_secret() {
        let c = Credentials {
            access_key: "AK".into(),
            secret_key: "supersecret".into(),
            session_token: None,
        };
        assert!(!format!("{c:?}").contains("supersecret"));
    }
}
