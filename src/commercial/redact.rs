// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Redaction for diagnostic bundles. Over-redacting is acceptable;
//! under-redacting is not. Applied to every string and to any JSON key that
//! looks sensitive.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

pub const REDACTED: &str = "[REDACTED]";

/// Substrings (lowercase) that mark a JSON key as sensitive.
const SENSITIVE_KEY_PARTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "authorization",
    "credential",
    "privatekey",
    "private_key",
    "private-key",
    "client-key",
    "clientkey",
    "client-certificate",
    "certificate-authority-data",
    "kubeconfig",
    "userdata",
    "user_data",
    "cloudinit",
    "networkdata",
    "sshkey",
    "ssh-key",
    "ssh_key",
    "publickeys",
];

static PEM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"-----BEGIN [A-Z0-9 ]+-----[\s\S]*?(-----END [A-Z0-9 ]+-----|$)")
        .expect("pem regex")
});
static AUTH_SCHEME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}").expect("auth regex"));
static JWT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*").expect("jwt regex")
});
static KEY_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)\b([A-Za-z0-9_.-]*(?:password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|client-key-data|client-certificate-data|certificate-authority-data)[A-Za-z0-9_.-]*)(\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;&]+)"#,
    )
    .expect("kv regex")
});
static URL_CREDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/:@]+:[^\s/@]+@").expect("url regex")
});
static AWS_KEY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b").expect("aws regex"));
static BIG_BLOB: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z0-9+/]{80,}={0,2}").expect("blob regex"));

/// Redact sensitive values from free-form text such as logs and events.
pub fn redact_text(input: &str) -> String {
    let s = PEM.replace_all(input, "[REDACTED PEM BLOCK]");
    let s = AUTH_SCHEME.replace_all(&s, "$1 [REDACTED]");
    let s = JWT.replace_all(&s, "[REDACTED JWT]");
    let s = KEY_VALUE.replace_all(&s, "$1$2[REDACTED]");
    let s = URL_CREDS.replace_all(&s, "$1[REDACTED]@");
    let s = AWS_KEY.replace_all(&s, "[REDACTED AWS KEY]");
    let s = BIG_BLOB.replace_all(&s, "[REDACTED BLOB]");
    s.into_owned()
}

fn sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    SENSITIVE_KEY_PARTS.iter().any(|p| k.contains(p))
}

/// Redact a JSON value in place: sensitive keys lose their value, every
/// string goes through [`redact_text`], and `{name: "DB_PASSWORD",
/// value: ...}` pairs (env vars, annotations) lose the value.
pub fn redact_json(v: &mut Value) {
    match v {
        Value::String(s) => *s = redact_text(s),
        Value::Array(a) => a.iter_mut().for_each(redact_json),
        Value::Object(m) => {
            let named_sensitive = m
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(sensitive_key);
            for (k, val) in m.iter_mut() {
                if sensitive_key(k) || (named_sensitive && k == "value") {
                    *val = Value::String(REDACTED.into());
                } else {
                    redact_json(val);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Fixtures are assembled at runtime so secret scanners do not flag
    // made-up values in the source. None of these are real credentials.
    fn fake_aws_key() -> String {
        ["AK", "IA", "ABCDEFGHIJKLMNOP"].concat()
    }
    fn fake_jwt() -> String {
        [
            "ey",
            "JhbGciOiJIUzI1NiJ9",
            ".",
            "eyJzdWIiOiIxMjM0In0",
            ".",
            "c2lnbmF0dXJl",
        ]
        .concat()
    }
    fn fake_pem(body: &str) -> String {
        let label = ["RSA ", "PRIV", "ATE KEY"].concat();
        format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----")
    }

    #[test]
    fn text_secrets_are_removed() {
        let aws = fake_aws_key();
        let cases = [
            "Authorization: Bearer abcdef1234567890".to_string(),
            "password=hunter2".to_string(),
            "db_password: \"p@ss w0rd\"".to_string(),
            "token: s3cr3tvalue".to_string(),
            "AWS_SECRET_ACCESS_KEY=abcd1234".to_string(),
            format!("key {aws} end"),
            "postgres://admin:hunter2@db.internal:5432/x".to_string(),
            fake_jwt(),
            "client-key-data: LS0tLS1CRUdJTg==".to_string(),
            fake_pem("MIIabc"),
        ];
        let leaks = [
            "hunter2",
            "abcdef1234567890",
            "s3cr3tvalue",
            "p@ss",
            aws.as_str(),
            "MIIabc",
            "c2lnbmF0dXJl",
            "LS0tLS1CRUdJTg",
        ];
        for c in &cases {
            let out = redact_text(c);
            assert!(out.contains("REDACTED"), "not redacted: {c} -> {out}");
            for leak in leaks {
                assert!(!out.contains(leak), "leaked {leak} from {c}: {out}");
            }
        }
    }

    #[test]
    fn long_blobs_are_removed_and_plain_text_survives() {
        let blob = "A".repeat(120);
        assert!(redact_text(&format!("data {blob}")).contains("REDACTED BLOB"));
        let plain = "VM web-1 failed to schedule: 0/3 nodes are available";
        assert_eq!(redact_text(plain), plain);
    }

    #[test]
    fn json_keys_and_env_pairs_are_redacted() {
        let mut v = json!({
            "spec": {
                "volumes": [{"cloudInitNoCloud": {"userData": "#cloud-config\nusers: [x]"}}],
                "template": {"env": [{"name": "DB_PASSWORD", "value": "topsecret"}, {"name": "MODE", "value": "prod"}]},
                "secretRef": {"name": "s"},
                "note": "token=abc123"
            },
            "kubeconfig": "apiVersion: v1"
        });
        redact_json(&mut v);
        let s = v.to_string();
        for leak in [
            "cloud-config",
            "topsecret",
            "abc123",
            "apiVersion",
            "users: [x]",
        ] {
            assert!(!s.contains(leak), "leaked {leak}: {s}");
        }
        assert!(
            s.contains("prod"),
            "non-sensitive env value must survive: {s}"
        );
    }
}
