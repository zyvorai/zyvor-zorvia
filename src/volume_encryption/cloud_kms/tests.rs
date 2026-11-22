use super::*;
use crate::keys::KeyProvider;
use crate::volume_encryption::{new_wrapped, rewrap, DataKey, DataKeyWrapper};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---- crypto vectors -------------------------------------------------------

#[test]
fn hmac_sha256_matches_rfc_4231() {
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

fn vector_signer() -> SigV4<'static> {
    SigV4 {
        region: "us-east-1",
        service: "service",
        access_key: "AKIDEXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
    }
}

fn hs(h: &[(&str, &str)]) -> Vec<(String, String)> {
    h.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn sigv4_matches_the_published_aws_test_suite() {
    // aws-sig-v4-test-suite: get-vanilla
    let a = vector_signer().authorization(
        "GET",
        "/",
        "",
        &hs(&[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ]),
        b"",
        "20150830T123600Z",
    );
    assert_eq!(
        a,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
         SignedHeaders=host;x-amz-date, \
         Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
    // aws-sig-v4-test-suite: post-vanilla
    let a = vector_signer().authorization(
        "POST",
        "/",
        "",
        &hs(&[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ]),
        b"",
        "20150830T123600Z",
    );
    assert!(
        a.ends_with("Signature=5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b")
    );
}

#[test]
fn sigv4_signing_key_matches_the_aws_documentation_example() {
    let s = SigV4 {
        region: "us-east-1",
        service: "iam",
        access_key: "AKIDEXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
    };
    assert_eq!(
        hex(&s.signing_key("20150830")),
        "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
    );
}

// ---- mock server ----------------------------------------------------------

struct Req {
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Req {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

struct Reply(u16, String);

/// Serve connections one at a time on a loopback port. `h(n, req)` gets the 0-based
/// request number. Returns (base URL, request counter).
fn mock(h: impl Fn(usize, &Req) -> Reply + Send + 'static) -> (String, Arc<AtomicUsize>) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let c2 = count.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let (head_end, len) = loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= p + 4 + len {
                        break (p, len);
                    }
                }
            };
            if head_end == 0 {
                continue;
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.lines();
            let path = lines
                .next()
                .unwrap_or("")
                .split(' ')
                .nth(1)
                .unwrap_or("")
                .to_string();
            let headers = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                .collect();
            let req = Req {
                path,
                headers,
                body: buf[head_end + 4..head_end + 4 + len].to_vec(),
            };
            let n = c2.fetch_add(1, Ordering::SeqCst);
            let Reply(code, body) = h(n, &req);
            let _ = write!(
                s,
                "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, count)
}

const AK: &str = "AKIDTESTKEY";
const SK: &str = "test/secret+KEY/never-log-this";

fn fast() -> Retry {
    Retry {
        max_attempts: 4,
        base: Duration::from_millis(1),
    }
}

fn aws_at(url: &str) -> CloudKms {
    let mut k = CloudKms::aws(
        "alias/zorvia",
        "us-east-1",
        Some(url),
        AwsCredSource::Env {
            access_key: AK.into(),
            secret_key: SK.into(),
            token: Some("session-token".into()),
        },
    )
    .unwrap();
    k.retry = fast();
    CloudKms::Aws(k)
}

/// A fake AWS KMS: verifies the SigV4 signature, "encrypts" by embedding the context
/// and plaintext in the blob, and refuses a blob presented with another context.
/// Its error message deliberately contains the plaintext, to prove we never echo it.
fn aws_handler(n: usize, r: &Req) -> Reply {
    let auth = r.headers.get("authorization").cloned().unwrap_or_default();
    let signed: Vec<&str> = auth
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .unwrap_or("")
        .split(';')
        .collect();
    let amz_date = r.headers.get("x-amz-date").cloned().unwrap_or_default();
    let hdrs: Vec<(String, String)> = signed
        .iter()
        .map(|k| {
            (
                k.to_string(),
                r.headers.get(*k).cloned().unwrap_or_default(),
            )
        })
        .collect();
    let want = SigV4 {
        region: "us-east-1",
        service: "kms",
        access_key: AK,
        secret_key: SK,
    }
    .authorization("POST", "/", "", &hdrs, &r.body, &amz_date);
    if want != auth || r.path != "/" || !signed.contains(&"x-amz-security-token") {
        return Reply(
            400,
            json!({"__type":"com.amazon#InvalidSignatureException"}).to_string(),
        );
    }
    let v = r.json();
    match r.headers.get("x-amz-target").map(String::as_str) {
        Some("TrentService.Encrypt") => {
            let blob = B64.encode(
                json!({"c": v["EncryptionContext"], "p": v["Plaintext"], "n": n}).to_string(),
            );
            Reply(
                200,
                json!({"CiphertextBlob": blob, "KeyId": v["KeyId"]}).to_string(),
            )
        }
        Some("TrentService.Decrypt") => {
            let stored: Value = B64
                .decode(v["CiphertextBlob"].as_str().unwrap_or(""))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            if stored["c"] != v["EncryptionContext"] {
                return Reply(
                    400,
                    json!({"__type":"InvalidCiphertextException",
                           "message": format!("context mismatch for {}", stored["p"])})
                    .to_string(),
                );
            }
            Reply(200, json!({"Plaintext": stored["p"]}).to_string())
        }
        _ => Reply(
            400,
            json!({"__type":"UnknownOperationException"}).to_string(),
        ),
    }
}

#[test]
fn aws_wrap_roundtrips_signs_requests_and_binds_the_context() {
    let (url, _) = mock(aws_handler);
    let p = KeyProvider::Cloud(aws_at(&url));
    assert_eq!(p.provider_name(), "aws-kms");
    let wrapped = new_wrapped(&p, "prod/db1/data").unwrap();
    assert!(wrapped.starts_with("enc:v1:aws:"));
    // Same context: opens. Another volume's context: the KMS refuses.
    p.unwrap("prod/db1/data", &wrapped).unwrap();
    let e = p
        .unwrap("prod/db1/other", &wrapped)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("InvalidCiphertextException") && e.contains("400"),
        "{e}"
    );
    // The KMS error message (which here contains the key) is never echoed.
    assert!(!e.contains("context mismatch"), "{e}");
    // Re-wrap goes through unwrap + wrap + verify.
    let again = rewrap(&p, "prod/db1/data", &wrapped).unwrap();
    assert!(!p.needs_rewrap(&again));
}

#[test]
fn aws_error_bodies_and_credentials_never_appear_in_errors() {
    let key = DataKey::from_bytes([7u8; 32]);
    let (url, _) = mock(|_, _| {
        Reply(
            403,
            json!({"__type":"AccessDeniedException",
                   "message": format!("secret key {SK} plaintext {}", B64.encode([7u8;32]))})
            .to_string(),
        )
    });
    let p = KeyProvider::Cloud(aws_at(&url));
    let e = format!("{:#}", p.wrap("a/b/c", &key).unwrap_err());
    assert!(
        e.contains("AccessDeniedException") && e.contains("403"),
        "{e}"
    );
    assert!(
        !e.contains(SK) && !e.contains(AK) && !e.contains("session-token"),
        "{e}"
    );
    assert!(!e.contains(&B64.encode([7u8; 32])), "{e}");
    // A hostile code is not echoed either.
    let (url, _) = mock(|_, _| {
        Reply(
            403,
            json!({"__type":"<script>leak key material</script>"}).to_string(),
        )
    });
    let e = KeyProvider::Cloud(aws_at(&url))
        .wrap("a/b/c", &key)
        .unwrap_err()
        .to_string();
    assert!(e.contains("(unknown)") && !e.contains("script"), "{e}");
}

#[test]
fn aws_retries_throttling_with_backoff_then_succeeds() {
    let (url, count) = mock(|n, r| {
        if n < 2 {
            Reply(400, json!({"__type":"ThrottlingException"}).to_string())
        } else {
            aws_handler(n, r)
        }
    });
    let p = KeyProvider::Cloud(aws_at(&url));
    p.wrap("a/b/c", &DataKey::from_bytes([1; 32])).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[test]
fn retries_are_bounded_and_client_errors_are_not_retried() {
    let (url, count) = mock(|_, _| Reply(503, "{}".into()));
    let e = KeyProvider::Cloud(aws_at(&url))
        .wrap("a/b/c", &DataKey::from_bytes([1; 32]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("503"), "{e}");
    assert_eq!(count.load(Ordering::SeqCst), 4);

    let (url, count) =
        mock(|_, _| Reply(403, json!({"__type":"AccessDeniedException"}).to_string()));
    assert!(KeyProvider::Cloud(aws_at(&url))
        .wrap("a/b/c", &DataKey::from_bytes([1; 32]))
        .is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn oversized_responses_are_refused_and_redirects_not_followed() {
    let (url, count) = mock(|_, _| Reply(200, "x".repeat(MAX_RESPONSE + 10)));
    let e = KeyProvider::Cloud(aws_at(&url))
        .wrap("a/b/c", &DataKey::from_bytes([1; 32]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("larger than"), "{e}");
    assert_eq!(count.load(Ordering::SeqCst), 1);

    let (url, _) = mock(|_, _| Reply(302, "{}".into()));
    let e = KeyProvider::Cloud(aws_at(&url))
        .wrap("a/b/c", &DataKey::from_bytes([1; 32]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("302"), "{e}");
}

#[test]
fn aws_credentials_file_is_reread_and_profile_selected() {
    let (url, _) = mock(|n, r| {
        // The file is rewritten with new keys between the two calls; only the
        // signature of the current keys is accepted.
        let sk = if n == 0 {
            "first-secret"
        } else {
            "second-secret"
        };
        let auth = r.headers.get("authorization").cloned().unwrap_or_default();
        let signed: Vec<String> = auth
            .split("SignedHeaders=")
            .nth(1)
            .and_then(|s| s.split(',').next())
            .unwrap_or("")
            .split(';')
            .map(String::from)
            .collect();
        let hdrs: Vec<(String, String)> = signed
            .iter()
            .map(|k| (k.clone(), r.headers.get(k).cloned().unwrap_or_default()))
            .collect();
        let want = SigV4 {
            region: "us-east-1",
            service: "kms",
            access_key: "AKFILE",
            secret_key: sk,
        }
        .authorization(
            "POST",
            "/",
            "",
            &hdrs,
            &r.body,
            r.headers.get("x-amz-date").unwrap(),
        );
        if want == auth {
            Reply(200, json!({"CiphertextBlob":"YQ=="}).to_string())
        } else {
            Reply(
                403,
                json!({"__type":"InvalidSignatureException"}).to_string(),
            )
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("creds");
    let write = |sk: &str| {
        std::fs::write(
            &path,
            format!("[other]\naws_access_key_id=NO\naws_secret_access_key=NO\n[default]\naws_access_key_id = AKFILE\naws_secret_access_key = {sk}\n"),
        )
        .unwrap()
    };
    let mut k = CloudKms::aws(
        "alias/zorvia",
        "us-east-1",
        Some(&url),
        AwsCredSource::File {
            path: path.clone(),
            profile: "default".into(),
        },
    )
    .unwrap();
    k.retry = fast();
    let k = CloudKms::Aws(k);
    write("first-secret");
    assert!(k.seal("dek", "a/b/c", "x").is_ok());
    write("second-secret");
    assert!(k.seal("dek", "a/b/c", "x").is_ok());
    assert!(parse_aws_credentials_file("[x]\n", "default").is_err());
}

// ---- GCP ------------------------------------------------------------------

const GKEY: &str = "projects/p1/locations/global/keyRings/r1/cryptoKeys/k1";

fn gcp_with(url: &str, token: GcpTokenSource) -> CloudKms {
    let mut k = CloudKms::gcp(GKEY, Some(url), token).unwrap();
    k.retry = fast();
    CloudKms::Gcp(k)
}

fn gcp_handler(n: usize, r: &Req) -> Reply {
    if r.headers.get("authorization").map(String::as_str) != Some("Bearer tok-123") {
        return Reply(
            401,
            json!({"error":{"status":"UNAUTHENTICATED"}}).to_string(),
        );
    }
    let v = r.json();
    if r.path == format!("/v1/{GKEY}:encrypt") {
        let ct = B64.encode(
            json!({"a": v["additionalAuthenticatedData"], "p": v["plaintext"], "n": n}).to_string(),
        );
        Reply(200, json!({"ciphertext": ct}).to_string())
    } else if r.path == format!("/v1/{GKEY}:decrypt") {
        let stored: Value = B64
            .decode(v["ciphertext"].as_str().unwrap_or(""))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        if stored["a"] != v["additionalAuthenticatedData"] {
            return Reply(
                400,
                json!({"error":{"status":"INVALID_ARGUMENT",
                                "message": format!("AAD mismatch, plaintext {}", stored["p"])}})
                .to_string(),
            );
        }
        Reply(200, json!({"plaintext": stored["p"]}).to_string())
    } else {
        Reply(404, json!({"error":{"status":"NOT_FOUND"}}).to_string())
    }
}

#[test]
fn gcp_wrap_roundtrips_binds_aad_and_never_echoes_bodies() {
    let (url, _) = mock(gcp_handler);
    let p = KeyProvider::Cloud(gcp_with(&url, GcpTokenSource::Static("tok-123".into())));
    assert_eq!(p.provider_name(), "gcp-kms");
    let wrapped = new_wrapped(&p, "prod/db1/data").unwrap();
    assert!(wrapped.starts_with("enc:v1:gcp:"));
    let e = p
        .unwrap("prod/db1/other", &wrapped)
        .unwrap_err()
        .to_string();
    assert!(e.contains("INVALID_ARGUMENT") && e.contains("400"), "{e}");
    assert!(
        !e.contains("AAD mismatch") && !e.contains("plaintext"),
        "{e}"
    );
    p.unwrap("prod/db1/data", &wrapped).unwrap();
    // A bad token is a clear error, and the token is not in it.
    let bad = KeyProvider::Cloud(gcp_with(&url, GcpTokenSource::Static("bad-token".into())));
    let e = bad
        .wrap("a/b/c", &DataKey::from_bytes([1; 32]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("401") && !e.contains("bad-token"), "{e}");
}

#[test]
fn gcp_retries_429_and_reads_a_mounted_token_file() {
    let (url, count) = mock(|n, r| {
        if n == 0 {
            Reply(
                429,
                json!({"error":{"status":"RESOURCE_EXHAUSTED"}}).to_string(),
            )
        } else {
            gcp_handler(n, r)
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("token");
    std::fs::write(&f, "tok-123\n").unwrap();
    let p = KeyProvider::Cloud(gcp_with(&url, GcpTokenSource::File(f)));
    p.wrap("a/b/c", &DataKey::from_bytes([2; 32])).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn gcp_metadata_server_token_is_fetched_once_and_cached() {
    let meta_hits = Arc::new(AtomicUsize::new(0));
    let m2 = meta_hits.clone();
    let (meta_url, _) = mock(move |_, r| {
        m2.fetch_add(1, Ordering::SeqCst);
        if r.headers.get("metadata-flavor").map(String::as_str) != Some("Google") {
            return Reply(403, "{}".into());
        }
        Reply(
            200,
            json!({"access_token":"tok-123","expires_in":3600}).to_string(),
        )
    });
    let (url, _) = mock(gcp_handler);
    let p = KeyProvider::Cloud(gcp_with(
        &url,
        GcpTokenSource::Metadata(reqwest::Url::parse(&meta_url).unwrap()),
    ));
    new_wrapped(&p, "a/b/c").unwrap();
    assert_eq!(meta_hits.load(Ordering::SeqCst), 1);
}

// ---- configuration --------------------------------------------------------

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let m: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| m.get(k).cloned()
}

#[test]
fn configuration_is_validated_https_only_and_exclusive() {
    let ok = env_of(&[
        ("ZORVIA_AWS_KMS_KEY_ID", "alias/zorvia"),
        ("AWS_REGION", "eu-west-1"),
        ("AWS_ACCESS_KEY_ID", "AK"),
        ("AWS_SECRET_ACCESS_KEY", "SK"),
    ]);
    let k = CloudKms::from_env(&ok).unwrap().unwrap();
    assert_eq!(k.id(), "aws");
    assert!(CloudKms::from_env(&env_of(&[])).unwrap().is_none());

    let with = |extra: (&'static str, &'static str)| {
        let mut v = vec![
            ("ZORVIA_AWS_KMS_KEY_ID", "alias/zorvia"),
            ("AWS_REGION", "eu-west-1"),
            ("AWS_ACCESS_KEY_ID", "AK"),
            ("AWS_SECRET_ACCESS_KEY", "SK"),
        ];
        v.push(extra);
        CloudKms::from_env(&env_of(&v)).err().map(|e| e.to_string())
    };
    assert!(with(("ZORVIA_AWS_KMS_ENDPOINT", "http://kms.example.com"))
        .unwrap()
        .contains("https"));
    assert!(with(("ZORVIA_AWS_KMS_ENDPOINT", "https://u:p@kms.example.com")).is_some());
    assert!(with(("ZORVIA_AWS_KMS_ENDPOINT", "https://kms.example.com/x")).is_some());
    assert!(with(("ZORVIA_AWS_KMS_ENDPOINT", "http://127.0.0.1:9")).is_none());
    assert!(with(("ZORVIA_AWS_REGION", "../etc")).is_some());
    assert!(with(("ZORVIA_GCP_KMS_KEY", GKEY))
        .unwrap()
        .contains("only one"));
    assert!(with(("AWS_SESSION_TOKEN", "a b")).is_some());

    // Missing credentials is a configuration error, not a silent default.
    assert!(CloudKms::from_env(&env_of(&[
        ("ZORVIA_AWS_KMS_KEY_ID", "alias/zorvia"),
        ("AWS_REGION", "eu-west-1"),
    ]))
    .is_err());

    let gcp = |extra: &[(&str, &str)]| {
        let mut v = vec![("ZORVIA_GCP_KMS_KEY", GKEY)];
        v.extend_from_slice(extra);
        CloudKms::from_env(&env_of(&v))
    };
    assert!(gcp(&[]).is_err()); // no token source
    assert!(gcp(&[("ZORVIA_GCP_ACCESS_TOKEN", "t")]).unwrap().is_some());
    assert!(gcp(&[("ZORVIA_GCP_USE_METADATA", "1")]).unwrap().is_some());
    assert!(gcp(&[
        ("ZORVIA_GCP_USE_METADATA", "1"),
        ("ZORVIA_GCP_ACCESS_TOKEN", "t")
    ])
    .is_err());
    assert!(gcp(&[
        ("ZORVIA_GCP_ACCESS_TOKEN", "t"),
        ("ZORVIA_GCP_KMS_ENDPOINT", "http://evil.example")
    ])
    .is_err());
    assert!(CloudKms::from_env(&env_of(&[
        ("ZORVIA_GCP_KMS_KEY", "projects/p/keys/k"),
        ("ZORVIA_GCP_ACCESS_TOKEN", "t")
    ]))
    .is_err());
}

#[test]
fn sealed_values_are_recognised_by_provider_kind() {
    let (url, _) = mock(aws_handler);
    let p = KeyProvider::Cloud(aws_at(&url));
    assert!(!p.needs_seal("enc:v1:aws:abc"));
    assert!(p.needs_seal("enc:v1:gcp:abc"));
    assert!(p.needs_seal("enc:v1:k1:abc"));
    assert!(p.needs_seal("plaintext"));
    let e = p
        .open_for("dek", "a/b/c", "enc:v1:vault:abc")
        .unwrap_err()
        .to_string();
    assert!(e.contains("different kind"), "{e}");
}
