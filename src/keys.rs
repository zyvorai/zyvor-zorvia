// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Key provider for secrets Zorvia must store but also read back (today: TOTP
//! secrets), so a copy of the database alone does not give them away.
//!
//! Two providers, chosen by environment (none configured = plaintext, as before):
//!
//! * **Local**: AES-256-GCM with a 32-byte key from `ZORVIA_KEY_FILE` (64 hex chars,
//!   mount it from a Secret) or `ZORVIA_KEY_HEX`. `ZORVIA_KEY_ID` (default `k1`)
//!   labels it; `ZORVIA_KEY_PREVIOUS_FILES` (`id=path,id=path`) lists older keys kept
//!   to decrypt rows not yet re-sealed, so a key can be rotated.
//! * **Vault Transit**: `ZORVIA_VAULT_ADDR`, `ZORVIA_VAULT_TRANSIT_KEY`,
//!   `ZORVIA_VAULT_TOKEN` or `ZORVIA_VAULT_TOKEN_FILE`, optional
//!   `ZORVIA_VAULT_MOUNT` (default `transit`). The key never leaves Vault.
//! * **Cloud KMS** (Experimental): AWS KMS (`ZORVIA_AWS_KMS_KEY_ID`) or Google Cloud
//!   KMS (`ZORVIA_GCP_KMS_KEY`); see `volume_encryption::cloud_kms` and
//!   docs/VOLUME_ENCRYPTION.md. Every seal/open is a network call to the KMS.
//!
//! A sealed value is `enc:v1:<key id>:<payload>`. It is bound to its owner (the user
//! id is authenticated data) so a ciphertext copied onto another row does not
//! decrypt. A value that is not sealed is read as legacy plaintext, and a startup
//! pass seals those. There is never a silent fallback to plaintext once a provider is
//! configured: a failed seal is an error.

use crate::volume_encryption::cloud_kms::{self, CloudKms};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::Rng;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

pub const PREFIX: &str = "enc:v1:";
const VAULT_ID: &str = "vault";
/// Ids that name a kind of provider and so cannot be a local key id.
const RESERVED_IDS: [&str; 3] = [VAULT_ID, cloud_kms::AWS_ID, cloud_kms::GCP_ID];

pub fn is_sealed(s: &str) -> bool {
    s.starts_with(PREFIX)
}

pub enum KeyProvider {
    Local {
        current: String,
        keys: HashMap<String, [u8; 32]>,
    },
    Vault(VaultTransit),
    /// AWS KMS or Google Cloud KMS (Experimental).
    Cloud(CloudKms),
}

pub struct VaultTransit {
    addr: String,
    mount: String,
    key: String,
    token: String,
}

fn parse_key(hex: &str) -> Result<[u8; 32]> {
    let h = hex.trim();
    if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("a Zorvia key must be 64 hex characters (32 bytes)");
    }
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)?;
    }
    Ok(k)
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn aad(purpose: &str, owner: &str) -> Vec<u8> {
    format!("zorvia:{purpose}:{owner}").into_bytes()
}

/// Run a future from sync code (the user database is synchronous). Inside a
/// multi-thread runtime this blocks the current worker; elsewhere it uses a
/// throwaway runtime.
pub(crate) fn block_on<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| h.block_on(fut))
        }
        Ok(_) => std::thread::scope(|s| {
            s.spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime")
                    .block_on(fut)
            })
            .join()
            .expect("vault thread")
        }),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(fut),
    }
}

impl VaultTransit {
    pub fn new(addr: &str, mount: &str, key: &str, token: &str) -> Result<Self> {
        let addr = addr.trim().trim_end_matches('/').to_string();
        let local = addr.starts_with("http://127.0.0.1") || addr.starts_with("http://localhost");
        if !(addr.starts_with("https://") || local) {
            bail!("ZORVIA_VAULT_ADDR must be https:// (plain http is only accepted for localhost)");
        }
        if !valid_id(mount) || !valid_id(key) {
            bail!("the Vault mount and key names may only contain letters, digits, '-' and '_'");
        }
        if token.trim().is_empty() {
            bail!("no Vault token configured");
        }
        Ok(Self {
            addr,
            mount: mount.to_string(),
            key: key.to_string(),
            token: token.trim().to_string(),
        })
    }

    async fn call(&self, op: &str, body: Value) -> Result<Value> {
        let url = format!("{}/v1/{}/{op}/{}", self.addr, self.mount, self.key);
        let resp = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?
            .post(&url)
            .header("X-Vault-Token", &self.token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Vault {op} request"))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let why = v["errors"][0].as_str().unwrap_or("no detail");
            bail!("Vault {op} failed: HTTP {status}: {why}");
        }
        Ok(v)
    }

    /// The owner id is authenticated by putting it in front of the secret inside
    /// the encrypted payload and checking it on the way out.
    fn seal(&self, purpose: &str, owner: &str, secret: &str) -> Result<String> {
        let mut plain = aad(purpose, owner);
        plain.push(0);
        plain.extend_from_slice(secret.as_bytes());
        let v = block_on(self.call("encrypt", json!({"plaintext": B64.encode(plain)})))?;
        v["data"]["ciphertext"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Vault returned no ciphertext"))
    }

    fn open(&self, purpose: &str, owner: &str, ciphertext: &str) -> Result<String> {
        let v = block_on(self.call("decrypt", json!({"ciphertext": ciphertext})))?;
        let raw = B64.decode(
            v["data"]["plaintext"]
                .as_str()
                .ok_or_else(|| anyhow!("Vault returned no plaintext"))?,
        )?;
        let prefix = {
            let mut p = aad(purpose, owner);
            p.push(0);
            p
        };
        let secret = raw
            .strip_prefix(prefix.as_slice())
            .ok_or_else(|| anyhow!("sealed value belongs to a different owner"))?;
        Ok(String::from_utf8(secret.to_vec())?)
    }
}

impl KeyProvider {
    pub fn local(current: &str, keys: HashMap<String, [u8; 32]>) -> Result<Self> {
        if !keys.contains_key(current) {
            bail!("the current key id '{current}' has no key");
        }
        Ok(Self::Local {
            current: current.to_string(),
            keys,
        })
    }

    /// `Ok(None)` when nothing is configured.
    pub fn from_env() -> Result<Option<Self>> {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let local = env("ZORVIA_KEY_FILE").is_some() || env("ZORVIA_KEY_HEX").is_some();
        let vault = env("ZORVIA_VAULT_ADDR").is_some();
        let cloud = !CloudKms::selected(&env).is_empty();
        if [local, vault, cloud].iter().filter(|b| **b).count() > 1 {
            bail!(
                "configure only one key provider: a local key (ZORVIA_KEY_*), Vault (ZORVIA_VAULT_*) or a cloud KMS (ZORVIA_AWS_KMS_* / ZORVIA_GCP_KMS_*)"
            );
        }
        if let Some(c) = CloudKms::from_env(&env)? {
            return Ok(Some(Self::Cloud(c)));
        }
        if vault {
            let token = match (env("ZORVIA_VAULT_TOKEN"), env("ZORVIA_VAULT_TOKEN_FILE")) {
                (Some(t), _) => t,
                (None, Some(f)) => {
                    std::fs::read_to_string(&f).with_context(|| format!("read {f}"))?
                }
                _ => bail!("set ZORVIA_VAULT_TOKEN or ZORVIA_VAULT_TOKEN_FILE"),
            };
            return Ok(Some(Self::Vault(VaultTransit::new(
                &env("ZORVIA_VAULT_ADDR").unwrap_or_default(),
                &env("ZORVIA_VAULT_MOUNT").unwrap_or_else(|| "transit".into()),
                &env("ZORVIA_VAULT_TRANSIT_KEY")
                    .ok_or_else(|| anyhow!("set ZORVIA_VAULT_TRANSIT_KEY"))?,
                &token,
            )?)));
        }
        if !local {
            return Ok(None);
        }
        let id = env("ZORVIA_KEY_ID").unwrap_or_else(|| "k1".into());
        if !valid_id(&id) || RESERVED_IDS.contains(&id.as_str()) {
            bail!(
                "ZORVIA_KEY_ID must be letters, digits, '-' or '_' and not 'vault', 'aws' or 'gcp'"
            );
        }
        let hex = match (env("ZORVIA_KEY_FILE"), env("ZORVIA_KEY_HEX")) {
            (Some(f), _) => std::fs::read_to_string(&f).with_context(|| format!("read {f}"))?,
            (None, Some(h)) => h,
            _ => unreachable!(),
        };
        let mut keys = HashMap::new();
        keys.insert(id.clone(), parse_key(&hex)?);
        if let Some(prev) = env("ZORVIA_KEY_PREVIOUS_FILES") {
            for item in prev.split(',').filter(|s| !s.trim().is_empty()) {
                let (pid, path) = item
                    .split_once('=')
                    .ok_or_else(|| anyhow!("ZORVIA_KEY_PREVIOUS_FILES entries are id=path"))?;
                let pid = pid.trim();
                if !valid_id(pid) || RESERVED_IDS.contains(&pid) || keys.contains_key(pid) {
                    bail!("previous key id '{pid}' is invalid or repeated");
                }
                let hex =
                    std::fs::read_to_string(path.trim()).with_context(|| format!("read {path}"))?;
                keys.insert(pid.to_string(), parse_key(&hex)?);
            }
        }
        Ok(Some(Self::local(&id, keys)?))
    }

    /// Seal `secret` for `owner` (a user id).
    pub fn seal(&self, owner: &str, secret: &str) -> Result<String> {
        self.seal_for("totp", owner, secret)
    }

    /// Open a sealed value belonging to `owner`.
    pub fn open(&self, owner: &str, stored: &str) -> Result<String> {
        self.open_for("totp", owner, stored)
    }

    /// Seal `secret` for `owner` under a purpose label that is authenticated data,
    /// so a value sealed for one purpose never opens as another (envelope data keys
    /// use `"dek"`).
    pub fn seal_for(&self, purpose: &str, owner: &str, secret: &str) -> Result<String> {
        match self {
            Self::Local { current, keys } => {
                let cipher = Aes256Gcm::new_from_slice(&keys[current])
                    .map_err(|e| anyhow!("bad key: {e}"))?;
                let mut nonce = [0u8; 12];
                rand::rng().fill_bytes(&mut nonce);
                let ct = cipher
                    .encrypt(
                        &Nonce::from(nonce),
                        Payload {
                            msg: secret.as_bytes(),
                            aad: &aad(purpose, owner),
                        },
                    )
                    .map_err(|_| anyhow!("encryption failed"))?;
                let mut blob = nonce.to_vec();
                blob.extend_from_slice(&ct);
                Ok(format!("{PREFIX}{current}:{}", B64.encode(blob)))
            }
            Self::Vault(v) => Ok(format!(
                "{PREFIX}{VAULT_ID}:{}",
                v.seal(purpose, owner, secret)?
            )),
            Self::Cloud(c) => Ok(format!(
                "{PREFIX}{}:{}",
                c.id(),
                c.seal(purpose, owner, secret)?
            )),
        }
    }

    /// Counterpart of [`Self::seal_for`].
    pub fn open_for(&self, purpose: &str, owner: &str, stored: &str) -> Result<String> {
        let rest = stored
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow!("value is not sealed"))?;
        let (id, payload) = rest
            .split_once(':')
            .ok_or_else(|| anyhow!("malformed sealed value"))?;
        match (self, id) {
            (Self::Vault(v), VAULT_ID) => v.open(purpose, owner, payload),
            (Self::Cloud(c), id) if id == c.id() => c.open(purpose, owner, payload),
            (Self::Local { keys, .. }, id) if !RESERVED_IDS.contains(&id) => {
                let key = keys
                    .get(id)
                    .ok_or_else(|| anyhow!("no key '{id}' is configured (was it rotated out?)"))?;
                let blob = B64.decode(payload)?;
                if blob.len() < 12 + 16 {
                    bail!("sealed value is too short");
                }
                let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("bad key: {e}"))?;
                let mut nonce = [0u8; 12];
                nonce.copy_from_slice(&blob[..12]);
                let plain = cipher
                    .decrypt(
                        &Nonce::from(nonce),
                        Payload {
                            msg: &blob[12..],
                            aad: &aad(purpose, owner),
                        },
                    )
                    .map_err(|_| {
                        anyhow!("decryption failed (wrong key, wrong owner or corrupted value)")
                    })?;
                Ok(String::from_utf8(plain)?)
            }
            _ => bail!("value was sealed by a different kind of key provider ('{id}')"),
        }
    }

    /// True when `stored` is plaintext or sealed under a key other than the current one.
    pub fn needs_seal(&self, stored: &str) -> bool {
        match (self, stored.strip_prefix(PREFIX)) {
            (_, None) => true,
            (Self::Local { current, .. }, Some(rest)) => {
                rest.split(':').next() != Some(current.as_str())
            }
            (Self::Vault(_), Some(rest)) => rest.split(':').next() != Some(VAULT_ID),
            (Self::Cloud(c), Some(rest)) => rest.split(':').next() != Some(c.id()),
        }
    }
}

static GLOBAL: OnceLock<Option<KeyProvider>> = OnceLock::new();

/// Read the configuration once, at startup. A broken configuration fails startup.
pub fn init_from_env() -> Result<()> {
    let p = KeyProvider::from_env()?;
    match &p {
        Some(KeyProvider::Local { current, keys }) => {
            log::info!(
                "key provider: local key '{current}' ({} key(s) known)",
                keys.len()
            )
        }
        Some(KeyProvider::Vault(v)) => log::info!(
            "key provider: Vault Transit {}/{}/{}",
            v.addr,
            v.mount,
            v.key
        ),
        Some(KeyProvider::Cloud(c)) => log::info!(
            "key provider: {} KMS key {} (Experimental)",
            c.id(),
            c.key_label()
        ),
        None => log::info!("key provider: none (TOTP secrets are stored unencrypted)"),
    }
    let _ = GLOBAL.set(p);
    Ok(())
}

/// Install a provider for a unit test (the global is set once per process; tests only
/// rely on the database API, which behaves the same sealed or not).
#[cfg(test)]
pub fn set_provider_for_tests(p: KeyProvider) {
    let _ = GLOBAL.set(Some(p));
}

pub fn provider() -> Option<&'static KeyProvider> {
    GLOBAL.get().and_then(|p| p.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn local(id: &str, byte: u8) -> KeyProvider {
        let mut keys = HashMap::new();
        keys.insert(id.to_string(), [byte; 32]);
        KeyProvider::local(id, keys).unwrap()
    }

    #[test]
    fn local_roundtrip_is_randomised_bound_to_the_owner_and_tamper_evident() {
        let p = local("k1", 7);
        let a = p.seal("user-1", "JBSWY3DPEHPK3PXP").unwrap();
        let b = p.seal("user-1", "JBSWY3DPEHPK3PXP").unwrap();
        assert!(
            a.starts_with("enc:v1:k1:") && a != b,
            "fresh nonce each time"
        );
        assert!(!a.contains("JBSWY3DPEHPK3PXP"));
        assert_eq!(p.open("user-1", &a).unwrap(), "JBSWY3DPEHPK3PXP");
        // Copied onto another user's row: refused.
        assert!(p.open("user-2", &a).is_err());
        // Flipped byte / wrong key: refused.
        let mut t = a.clone().into_bytes();
        let n = t.len() - 3;
        t[n] = if t[n] == b'A' { b'B' } else { b'A' };
        assert!(p.open("user-1", &String::from_utf8(t).unwrap()).is_err());
        assert!(local("k1", 8).open("user-1", &a).is_err());
        assert!(p.open("user-1", "plain").is_err());
    }

    #[test]
    fn rotation_reads_old_keys_and_flags_what_needs_resealing() {
        let old = local("k1", 1);
        let sealed_old = old.seal("u", "secret").unwrap();
        let mut keys = HashMap::new();
        keys.insert("k2".to_string(), [2u8; 32]);
        keys.insert("k1".to_string(), [1u8; 32]);
        let new = KeyProvider::local("k2", keys).unwrap();
        assert_eq!(
            new.open("u", &sealed_old).unwrap(),
            "secret",
            "old key still decrypts"
        );
        assert!(
            new.needs_seal(&sealed_old),
            "sealed under k1, current is k2"
        );
        assert!(new.needs_seal("legacy-plaintext"));
        assert!(!new.needs_seal(&new.seal("u", "x").unwrap()));
        // A key that was rotated out entirely cannot be read, with a clear reason.
        let e = local("k3", 3)
            .open("u", &sealed_old)
            .unwrap_err()
            .to_string();
        assert!(e.contains("no key 'k1'"), "{e}");
    }

    #[test]
    fn key_material_and_ids_are_validated() {
        assert!(parse_key(&"ab".repeat(32)).is_ok());
        for bad in ["", "abc", &"zz".repeat(32), &"ab".repeat(31)] {
            assert!(parse_key(bad).is_err(), "{bad}");
        }
        assert!(KeyProvider::local("k9", HashMap::new()).is_err());
        assert!(
            VaultTransit::new("http://vault.example", "transit", "k", "t").is_err(),
            "plain http is refused"
        );
        assert!(VaultTransit::new("https://vault.example", "tra/nsit", "k", "t").is_err());
        assert!(VaultTransit::new("https://vault.example", "transit", "k", "  ").is_err());
        assert!(VaultTransit::new("http://127.0.0.1:8200", "transit", "k", "t").is_ok());
    }

    /// A one-endpoint-pair fake Vault: "encrypts" by reversing base64 text behind a
    /// `vault:v1:` marker, which is enough to exercise the wire protocol and the
    /// owner binding. Requests are checked for the token header.
    fn fake_vault(requests: usize) -> (String, std::thread::JoinHandle<()>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
                let ok = req
                    .to_ascii_lowercase()
                    .contains("x-vault-token: s.test-token");
                let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                let reply = if !ok {
                    (403, json!({"errors": ["permission denied"]}))
                } else if req.starts_with("POST /v1/transit/encrypt/zorvia ") {
                    let pt = v["plaintext"].as_str().unwrap_or("");
                    (
                        200,
                        json!({"data": {"ciphertext": format!("vault:v1:{}", pt.chars().rev().collect::<String>())}}),
                    )
                } else if req.starts_with("POST /v1/transit/decrypt/zorvia ") {
                    let ct = v["ciphertext"]
                        .as_str()
                        .unwrap_or("")
                        .trim_start_matches("vault:v1:");
                    (
                        200,
                        json!({"data": {"plaintext": ct.chars().rev().collect::<String>()}}),
                    )
                } else {
                    (404, json!({"errors": ["unsupported path"]}))
                };
                let text = reply.1.to_string();
                let _ = write!(
                    s,
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    reply.0,
                    text.len(),
                    text
                );
            }
        });
        (addr, h)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vault_transit_seals_binds_to_the_owner_and_reports_vault_errors() {
        let (addr, server) = fake_vault(4);
        let p = KeyProvider::Vault(
            VaultTransit::new(&addr, "transit", "zorvia", "s.test-token").unwrap(),
        );
        let sealed = p.seal("user-1", "JBSWY3DPEHPK3PXP").unwrap();
        assert!(sealed.starts_with("enc:v1:vault:vault:v1:"));
        assert_eq!(p.open("user-1", &sealed).unwrap(), "JBSWY3DPEHPK3PXP");
        let e = p.open("user-2", &sealed).unwrap_err().to_string();
        assert!(e.contains("different owner"), "{e}");
        assert!(!p.needs_seal(&sealed));
        // A wrong token is a clear Vault error, not a panic.
        let bad =
            KeyProvider::Vault(VaultTransit::new(&addr, "transit", "zorvia", "s.wrong").unwrap());
        let e = bad.seal("user-1", "x").unwrap_err().to_string();
        assert!(e.contains("403") && e.contains("permission denied"), "{e}");
        server.join().unwrap();
        // A local provider cannot open Vault values and vice versa.
        assert!(local("k1", 1).open("user-1", &sealed).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vault_seals_data_keys_by_purpose() {
        let (addr, server) = fake_vault(2);
        let p = KeyProvider::Vault(
            VaultTransit::new(&addr, "transit", "zorvia", "s.test-token").unwrap(),
        );
        let w = p.seal_for("dek", "ns/vm/vol", "a2V5").unwrap();
        assert_eq!(p.open_for("dek", "ns/vm/vol", &w).unwrap(), "a2V5");
        server.join().unwrap();
    }
}
