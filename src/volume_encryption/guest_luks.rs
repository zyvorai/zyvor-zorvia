// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Guest LUKS delivery (Experimental, partial). See docs/VOLUME_ENCRYPTION.md.
//!
//! The data key stays wrapped in its Secret. A guest obtains the unlock passphrase
//! (the data key, base64) from Zorvia only by redeeming a one-time token over HTTPS:
//!
//! * [`TokenService`] issues tokens bound to `namespace/vm/volume`, short-lived and
//!   single-use; only a SHA-256 of the token secret is held, compared in constant
//!   time; failures are rate limited. State is in memory (single replica).
//! * [`cloud_config`] renders the guest-side cloud-init (script, unit, crypttab/fstab
//!   lines). It contains the token (a bearer credential that is useless after one
//!   use) but never any key material.
//! * [`redeem_with`] consumes a token and only then unwraps the key.

use super::{context, DataKey, DataKeyWrapper};
use anyhow::{bail, Result};
use base64::{
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL},
    Engine,
};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use subtle::ConstantTimeEq;

pub const MIN_TTL_SECS: i64 = 30;
pub const DEFAULT_TTL_SECS: i64 = 600;
pub const MAX_TTL_SECS: i64 = 900;
const MAX_OUTSTANDING: usize = 256;
const FAIL_WINDOW_SECS: i64 = 60;
const MAX_FAILS_PER_BINDING: u32 = 5;
const MAX_ATTEMPTS_GLOBAL: u32 = 120;
const TOKEN_PREFIX: &str = "zlt1";
pub const REDEEM_PATH: &str = "/api/volume-encryption/guest-luks/redeem";

/// What a token is good for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binding {
    pub namespace: String,
    pub vm: String,
    pub volume: String,
}

impl Binding {
    pub fn new(namespace: &str, vm: &str, volume: &str) -> Self {
        Self {
            namespace: namespace.into(),
            vm: vm.into(),
            volume: volume.into(),
        }
    }
    pub fn ctx(&self) -> String {
        context(&self.namespace, &self.vm, &self.volume)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemError {
    Malformed,
    Unknown,
    Expired,
    AlreadyUsed,
    WrongBinding,
    Throttled,
    UnwrapFailed,
}

impl RedeemError {
    /// Audit-only detail. API callers get a generic denial so the reason is no oracle.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Malformed => "malformed token",
            Self::Unknown => "unknown token",
            Self::Expired => "token expired",
            Self::AlreadyUsed => "token already used",
            Self::WrongBinding => "token bound to a different volume",
            Self::Throttled => "rate limited",
            Self::UnwrapFailed => "key could not be unwrapped",
        }
    }
}

struct Record {
    hash: [u8; 32],
    binding: Binding,
    expires: i64,
    used: bool,
}

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, Record>,
    fails: HashMap<Binding, (i64, u32)>,
    global: (i64, u32),
}

/// Issued token. The value is shown once to the issuer and never stored.
pub struct IssuedToken {
    pub token: String,
    pub expires_at: i64,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IssuedToken(<redacted>, expires_at={})", self.expires_at)
    }
}

#[derive(Default)]
pub struct TokenService {
    inner: Mutex<Inner>,
}

fn sha(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

impl TokenService {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Issue a token for `binding`, replacing any outstanding one for it.
    pub fn issue(&self, binding: &Binding, ttl_secs: i64, now: i64) -> Result<IssuedToken> {
        if !(MIN_TTL_SECS..=MAX_TTL_SECS).contains(&ttl_secs) {
            bail!("ttl_seconds must be between {MIN_TTL_SECS} and {MAX_TTL_SECS}");
        }
        let mut g = self.lock();
        g.tokens.retain(|_, r| r.expires > now && !r.used);
        g.tokens.retain(|_, r| r.binding != *binding);
        if g.tokens.len() >= MAX_OUTSTANDING {
            bail!("too many outstanding unlock tokens");
        }
        let mut id = [0u8; 9];
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut id);
        rand::rng().fill_bytes(&mut secret);
        let id = B64URL.encode(id);
        let secret = B64URL.encode(secret);
        g.tokens.insert(
            id.clone(),
            Record {
                hash: sha(&secret),
                binding: binding.clone(),
                expires: now + ttl_secs,
                used: false,
            },
        );
        Ok(IssuedToken {
            token: format!("{TOKEN_PREFIX}.{id}.{secret}"),
            expires_at: now + ttl_secs,
        })
    }

    /// Number of live (unexpired, unused) tokens.
    pub fn outstanding(&self, now: i64) -> usize {
        self.lock()
            .tokens
            .values()
            .filter(|r| r.expires > now && !r.used)
            .count()
    }

    /// Validate and consume `token` for `binding`. Success burns it; a token presented
    /// for the wrong volume is burned too (fail closed).
    pub fn consume(&self, token: &str, binding: &Binding, now: i64) -> Result<(), RedeemError> {
        let mut g = self.lock();
        // Rate limits are checked before the token is looked at, so they are no oracle.
        if now - g.global.0 >= FAIL_WINDOW_SECS {
            g.global = (now, 0);
        }
        g.global.1 += 1;
        if g.global.1 > MAX_ATTEMPTS_GLOBAL {
            return Err(RedeemError::Throttled);
        }
        g.fails.retain(|_, (t, _)| now - *t < FAIL_WINDOW_SECS);
        if matches!(g.fails.get(binding), Some((_, n)) if *n >= MAX_FAILS_PER_BINDING) {
            return Err(RedeemError::Throttled);
        }
        let r = Self::check(&mut g, token, binding, now);
        if r.is_err() {
            let e = g.fails.entry(binding.clone()).or_insert((now, 0));
            e.1 += 1;
        }
        r
    }

    fn check(g: &mut Inner, token: &str, binding: &Binding, now: i64) -> Result<(), RedeemError> {
        let parts: Vec<&str> = token.split('.').collect();
        let (id, secret) = match parts.as_slice() {
            [p, id, s] if *p == TOKEN_PREFIX && token.len() <= 128 => (*id, *s),
            _ => return Err(RedeemError::Malformed),
        };
        let presented = sha(secret);
        // Compare against a dummy when the id is unknown, to keep timing alike.
        let dummy = [0u8; 32];
        let stored = g.tokens.get(id).map(|r| r.hash).unwrap_or(dummy);
        let hash_ok: bool = presented.ct_eq(&stored).into();
        let Some(rec) = g.tokens.get_mut(id) else {
            return Err(RedeemError::Unknown);
        };
        if !hash_ok {
            return Err(RedeemError::Unknown);
        }
        if rec.used {
            return Err(RedeemError::AlreadyUsed);
        }
        if rec.expires <= now {
            g.tokens.remove(id);
            return Err(RedeemError::Expired);
        }
        if rec.binding != *binding {
            rec.used = true;
            return Err(RedeemError::WrongBinding);
        }
        rec.used = true;
        Ok(())
    }
}

/// The LUKS passphrase handed to a guest. No `Debug`/`Serialize`; zeroed on drop.
pub struct Passphrase(String);

impl Passphrase {
    /// The one place the passphrase is read: the HTTP body for the redeeming guest.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Passphrase {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        super::zero(&mut bytes);
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

pub fn passphrase_from(key: &DataKey) -> Passphrase {
    Passphrase(B64.encode(key.0))
}

/// Consume the token, and only if it is valid unwrap the key.
pub fn redeem_with(
    svc: &TokenService,
    w: &dyn DataKeyWrapper,
    token: &str,
    binding: &Binding,
    wrapped: &str,
    now: i64,
) -> Result<Passphrase, RedeemError> {
    svc.consume(token, binding, now)?;
    let key = w
        .unwrap(&binding.ctx(), wrapped)
        .map_err(|_| RedeemError::UnwrapFailed)?;
    Ok(passphrase_from(&key))
}

// ── guest cloud-init ────────────────────────────────────────────────────────

pub struct GuestParams<'a> {
    pub namespace: &'a str,
    pub vm: &'a str,
    pub volume: &'a str,
    /// Public base URL guests use to reach Zorvia (`https://host[:port]`).
    pub base_url: &'a str,
    /// Optional PEM CA bundle (public certificate) to trust for that URL.
    pub ca_pem: Option<&'a str>,
}

/// virtio disk serial for the LUKS volume (KubeVirt `disk.serial`, max 20 chars).
pub fn disk_serial(volume: &str) -> String {
    let s = format!("zv{volume}").replace('-', "");
    s.chars().take(20).collect()
}

pub fn mapper_name(volume: &str) -> String {
    format!("zv-{volume}")
}

/// Validate a base URL: https only, no credentials/query/fragment, safe characters.
pub fn validate_base_url(u: &str) -> Result<()> {
    let Some(rest) = u.strip_prefix("https://") else {
        bail!("guest LUKS unlock URL must start with https://");
    };
    if rest.is_empty()
        || u.len() > 200
        || !rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:/".contains(c))
        || rest.starts_with('/')
        || rest.contains("//")
    {
        bail!("guest LUKS unlock URL has unsupported characters or shape");
    }
    Ok(())
}

fn validate_pem(p: &str) -> Result<()> {
    let ok = p.len() <= 16_384
        && p.trim_start().starts_with("-----BEGIN CERTIFICATE-----")
        && p.trim_end().ends_with("-----END CERTIFICATE-----")
        && p.lines().all(|l| {
            l.starts_with("-----")
                || l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        });
    if !ok {
        bail!("CA bundle is not a PEM certificate chain");
    }
    Ok(())
}

pub fn unlock_url(p: &GuestParams<'_>) -> String {
    format!(
        "{}{REDEEM_PATH}/{}/{}/{}",
        p.base_url.trim_end_matches('/'),
        p.namespace,
        p.vm,
        p.volume
    )
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render the guest `#cloud-config`. `token` (if any) is written to a tmpfs file; the
/// key is never part of the output. All interpolated values are validated first.
pub fn cloud_config(p: &GuestParams<'_>, token: Option<&str>) -> Result<String> {
    for (k, v) in [
        ("namespace", p.namespace),
        ("vm", p.vm),
        ("volume", p.volume),
    ] {
        crate::kube::lifecycle::validate_k8s_name(k, v)?;
    }
    validate_base_url(p.base_url)?;
    if let Some(c) = p.ca_pem {
        validate_pem(c)?;
    }
    if let Some(t) = token {
        let ok = t.len() <= 128
            && t.starts_with(TOKEN_PREFIX)
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        if !ok {
            bail!("token has an unexpected shape");
        }
    }
    let serial = disk_serial(p.volume);
    let name = mapper_name(p.volume);
    let url = unlock_url(p);
    let cacert = if p.ca_pem.is_some() {
        " --cacert /etc/zorvia/luks-ca.pem"
    } else {
        ""
    };
    let script = format!(
        r#"#!/bin/bash
# Generated by Zorvia. Contains NO key material. The passphrase is fetched with a
# one-time token, held only in this shell's memory and piped to cryptsetup.
set -eu
NAME={name}
DEV=/dev/disk/by-id/virtio-{serial}
MNT=/mnt/{vol}
URL={url}
TOKEN_FILE=/run/zorvia-luks/token
umask 077
[ -b "$DEV" ] || {{ echo "zorvia-luks: $DEV not present" >&2; exit 1; }}
cryptsetup status "$NAME" >/dev/null 2>&1 && exit 0
[ -r "$TOKEN_FILE" ] || {{ echo "zorvia-luks: no unlock token (single-use; issue a new one)" >&2; exit 1; }}
# curl reads URL and token from a config on stdin, so neither is in argv.
KEY=$({{ printf url = %sn "$URL"; printf header = Authorization: Bearer %sn "$(cat "$TOKEN_FILE")"; }} \
  | curl -K - --fail --silent --show-error --proto =https --tlsv1.2 --max-time 20 -X POST{cacert})
rm -f "$TOKEN_FILE"
[ -n "$KEY" ] || {{ echo "zorvia-luks: empty key response" >&2; exit 1; }}
if cryptsetup isLuks "$DEV"; then
  :
elif [ -z "$(blkid -p -o value -s TYPE "$DEV" 2>/dev/null)" ]; then
  printf %s "$KEY" | cryptsetup luksFormat --batch-mode --key-file=- "$DEV"
  FRESH=1
else
  echo "zorvia-luks: $DEV holds a non-LUKS filesystem; refusing to format it" >&2; exit 1
fi
printf %s "$KEY" | cryptsetup luksOpen --key-file=- "$DEV" "$NAME"
unset KEY
if [ "${{FRESH:-}}" = 1 ]; then mkfs.ext4 -q -L '{vol}' "/dev/mapper/$NAME"; fi
mkdir -p "$MNT"
mountpoint -q "$MNT" || mount "/dev/mapper/$NAME" "$MNT"
"#,
        vol = p.volume
    );
    let unit = "[Unit]\nDescription=Unlock Zorvia LUKS volume\nAfter=network-online.target\nWants=network-online.target\nConditionPathExists=/run/zorvia-luks/token\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=/usr/local/sbin/zorvia-luks-unlock\n\n[Install]\nWantedBy=multi-user.target\n";
    let mut out = String::from("#cloud-config\n# Generated by Zorvia guest LUKS. No key material.\npackages:\n  - cryptsetup\n  - curl\nwrite_files:\n");
    out += "  - path: /usr/local/sbin/zorvia-luks-unlock\n    permissions: '0700'\n    owner: root:root\n    content: |\n";
    out += &indent(&script, 6);
    out += "\n  - path: /etc/systemd/system/zorvia-luks-unlock.service\n    permissions: '0644'\n    content: |\n";
    out += &indent(unit, 6);
    if let Some(ca) = p.ca_pem {
        out += "\n  - path: /etc/zorvia/luks-ca.pem\n    permissions: '0644'\n    content: |\n";
        out += &indent(ca.trim_end(), 6);
    }
    if let Some(t) = token {
        out += &format!(
            "\n  - path: /run/zorvia-luks/token\n    permissions: '0600'\n    owner: root:root\n    content: '{t}'"
        );
    }
    out += &format!(
        "\nruncmd:\n  - grep -q '^{name} ' /etc/crypttab 2>/dev/null || echo '{name} /dev/disk/by-id/virtio-{serial} none luks,noauto,nofail' >> /etc/crypttab\n  - grep -q ' /mnt/{vol} ' /etc/fstab 2>/dev/null || echo '/dev/mapper/{name} /mnt/{vol} ext4 noauto,nofail 0 2' >> /etc/fstab\n  - systemctl daemon-reload\n  - systemctl enable zorvia-luks-unlock.service\n  - systemctl start zorvia-luks-unlock.service || true\n",
        vol = p.volume
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyProvider;

    const NOW: i64 = 1_000_000;

    fn b() -> Binding {
        Binding::new("prod", "db1", "data")
    }
    fn provider() -> KeyProvider {
        let mut m = HashMap::new();
        m.insert("k1".to_string(), [7u8; 32]);
        KeyProvider::local("k1", m).unwrap()
    }
    fn wrapped() -> (String, DataKey) {
        let p = provider();
        let k = DataKey::from_bytes([0x42; 32]);
        (p.wrap("prod/db1/data", &k).unwrap(), k)
    }

    #[test]
    fn token_redeems_once_then_replay_fails() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        assert_eq!(svc.outstanding(NOW), 1);
        let (w, k) = wrapped();
        let p = redeem_with(&svc, &provider(), &t.token, &b(), &w, NOW + 1).unwrap();
        assert_eq!(p.expose(), B64.encode(k.0));
        let again = redeem_with(&svc, &provider(), &t.token, &b(), &w, NOW + 2);
        assert_eq!(again.unwrap_err(), RedeemError::AlreadyUsed);
        assert_eq!(svc.outstanding(NOW + 2), 0);
    }

    #[test]
    fn expired_token_is_refused() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        assert_eq!(
            svc.consume(&t.token, &b(), NOW + 60).unwrap_err(),
            RedeemError::Expired
        );
    }

    #[test]
    fn wrong_volume_is_refused_and_burns_the_token() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        let other = Binding::new("prod", "db1", "other");
        assert_eq!(
            svc.consume(&t.token, &other, NOW).unwrap_err(),
            RedeemError::WrongBinding
        );
        assert_eq!(
            svc.consume(&t.token, &b(), NOW).unwrap_err(),
            RedeemError::AlreadyUsed
        );
    }

    #[test]
    fn forged_and_malformed_tokens_are_refused() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        let parts: Vec<&str> = t.token.split('.').collect();
        let forged = format!("zlt1.{}.{}", parts[1], "A".repeat(43));
        assert_eq!(
            svc.consume(&forged, &b(), NOW).unwrap_err(),
            RedeemError::Unknown
        );
        for bad in ["", "x", "zlt1.a", "zlt2.a.b", &"z".repeat(500)] {
            // A fresh service each time: failures count against the binding and would throttle it.
            let svc = TokenService::new();
            assert_eq!(
                svc.consume(bad, &b(), NOW).unwrap_err(),
                RedeemError::Malformed
            );
        }
        // The real token still works: failures do not consume it.
        assert!(svc.consume(&t.token, &b(), NOW).is_ok());
    }

    #[test]
    fn ttl_bounds_and_reissue_supersedes() {
        let svc = TokenService::new();
        assert!(svc.issue(&b(), 5, NOW).is_err());
        assert!(svc.issue(&b(), MAX_TTL_SECS + 1, NOW).is_err());
        let t1 = svc.issue(&b(), 60, NOW).unwrap();
        let t2 = svc.issue(&b(), 60, NOW).unwrap();
        assert_ne!(t1.token, t2.token);
        assert_eq!(svc.outstanding(NOW), 1);
        assert!(svc.consume(&t1.token, &b(), NOW).is_err());
        assert!(svc.consume(&t2.token, &b(), NOW).is_ok());
    }

    #[test]
    fn failures_are_rate_limited_per_volume() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 600, NOW).unwrap();
        for _ in 0..MAX_FAILS_PER_BINDING {
            assert_eq!(
                svc.consume("zlt1.a.b", &b(), NOW).unwrap_err(),
                RedeemError::Unknown
            );
        }
        // Even the valid token is refused while throttled (fail closed) ...
        assert_eq!(
            svc.consume(&t.token, &b(), NOW).unwrap_err(),
            RedeemError::Throttled
        );
        // ... other volumes are unaffected, and the window expires.
        let o = Binding::new("prod", "db2", "data");
        assert_eq!(
            svc.consume("zlt1.a.b", &o, NOW).unwrap_err(),
            RedeemError::Unknown
        );
        assert!(svc.consume(&t.token, &b(), NOW + FAIL_WINDOW_SECS).is_ok());
    }

    #[test]
    fn global_rate_limit() {
        let svc = TokenService::new();
        for i in 0..MAX_ATTEMPTS_GLOBAL {
            let bind = Binding::new("n", &format!("v{i}"), "d");
            let _ = svc.consume("zlt1.a.b", &bind, NOW);
        }
        assert_eq!(
            svc.consume("zlt1.a.b", &b(), NOW).unwrap_err(),
            RedeemError::Throttled
        );
    }

    #[test]
    fn unwrap_failure_after_consume_returns_no_key() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        let r = redeem_with(&svc, &provider(), &t.token, &b(), "enc:v1:k1:AAAA", NOW);
        assert_eq!(r.unwrap_err(), RedeemError::UnwrapFailed);
        // A wrap bound to another volume does not open for this binding either.
        let t = svc.issue(&b(), 60, NOW).unwrap();
        let other = provider()
            .wrap("prod/db1/other", &DataKey::generate())
            .unwrap();
        let r = redeem_with(&svc, &provider(), &t.token, &b(), &other, NOW);
        assert_eq!(r.unwrap_err(), RedeemError::UnwrapFailed);
    }

    #[test]
    fn debug_output_hides_secrets() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        assert!(!format!("{t:?}").contains(&t.token));
        let (w, k) = wrapped();
        let p = redeem_with(&svc, &provider(), &t.token, &b(), &w, NOW).unwrap();
        let dbg = format!("{p:?}");
        assert!(dbg.contains("redacted") && !dbg.contains(&B64.encode(k.0)));
    }

    fn params<'a>(ca: Option<&'a str>) -> GuestParams<'a> {
        GuestParams {
            namespace: "prod",
            vm: "db1",
            volume: "data",
            base_url: "https://zorvia.example.com:8443",
            ca_pem: ca,
        }
    }

    #[test]
    fn cloud_config_never_contains_key_material() {
        let svc = TokenService::new();
        let t = svc.issue(&b(), 60, NOW).unwrap();
        let (w, k) = wrapped();
        let cfg = cloud_config(&params(None), Some(&t.token)).unwrap();
        let b64 = B64.encode(k.0);
        let hex = "42".repeat(32);
        assert!(!cfg.contains(&b64) && !cfg.contains(&hex) && !cfg.contains(&w));
        assert!(cfg.starts_with("#cloud-config"));
        assert!(cfg.contains("cryptsetup luksOpen --key-file=-"));
        assert!(cfg.contains("/run/zorvia-luks/token"));
        assert!(cfg.contains(&t.token));
        assert!(cfg.contains("noauto,nofail"));
        assert!(cfg.contains(
            "https://zorvia.example.com:8443/api/volume-encryption/guest-luks/redeem/prod/db1/data"
        ));
        // Without a token the template carries no credential at all.
        let bare = cloud_config(&params(None), None).unwrap();
        assert!(!bare.contains("zlt1") && !bare.contains("content: 'zlt"));
        // The token is only passed via the stdin curl config, never in a command line.
        assert!(!cfg.contains("Bearer zlt1"));
    }

    #[test]
    fn cloud_config_parses_as_yaml_cloud_config() {
        let cfg = cloud_config(&params(None), Some("zlt1.abc.def")).unwrap();
        let v: serde_json::Value = serde_yaml::from_str(&cfg).unwrap();
        assert_eq!(v["packages"][0], "cryptsetup");
        let files = v["write_files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert!(files[0]["content"]
            .as_str()
            .unwrap()
            .starts_with("#!/bin/bash"));
        assert_eq!(files[2]["content"], "zlt1.abc.def");
        assert_eq!(files[2]["permissions"], "0600");
    }

    #[test]
    fn template_rejects_injection_and_bad_inputs() {
        let mut p = params(None);
        p.vm = "db1'; rm -rf /";
        assert!(cloud_config(&p, None).is_err());
        let mut p = params(None);
        p.base_url = "http://zorvia.example.com";
        assert!(cloud_config(&p, None).is_err());
        for u in [
            "https://",
            "https://a b",
            "https://u:p@h/'x",
            "https://h/?q=1",
            "https://h//x",
            "https:///x",
        ] {
            assert!(validate_base_url(u).is_err(), "{u}");
        }
        assert!(cloud_config(&params(None), Some("zlt1.a$(id).b")).is_err());
        assert!(cloud_config(&params(None), Some("not-a-token")).is_err());
        assert!(cloud_config(&params(Some("not a cert")), None).is_err());
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB+/=\n-----END CERTIFICATE-----\n";
        let cfg = cloud_config(&params(Some(pem)), None).unwrap();
        assert!(cfg.contains("--cacert /etc/zorvia/luks-ca.pem"));
        assert!(serde_yaml::from_str::<serde_json::Value>(&cfg).is_ok());
    }

    #[test]
    fn serial_fits_virtio_limit() {
        assert!(disk_serial(&"a-".repeat(30)).len() <= 20);
        assert_eq!(disk_serial("data"), "zvdata");
        assert_eq!(mapper_name("data"), "zv-data");
    }
}
