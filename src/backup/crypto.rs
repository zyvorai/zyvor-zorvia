// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Client-side encryption for off-cluster backups: AES-256-GCM per part.
//! Each stored part is `nonce(12) || ciphertext || tag(16)` with a fresh
//! random nonce, so parts decrypt independently (and a restore can verify
//! each one before applying any of it).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use std::io::Read;

pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const KEY_LEN: usize = 32;
pub const ALGORITHM: &str = "AES-256-GCM";

/// Parse a 64-hex-character key (as stored in a Kubernetes Secret).
pub fn parse_key_hex(s: &str) -> Result<[u8; KEY_LEN]> {
    let s = s.trim();
    if s.len() != KEY_LEN * 2 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("encryption key must be {} hex characters", KEY_LEN * 2);
    }
    let mut key = [0u8; KEY_LEN];
    for (i, b) in key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(key)
}

fn random_nonce() -> Result<[u8; NONCE_LEN]> {
    let mut n = [0u8; NONCE_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut n)?;
    Ok(n)
}

/// Encrypt one part.
pub fn seal(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("bad key: {e}"))?;
    let nonce_bytes = random_nonce()?;
    let ct = cipher
        .encrypt(&Nonce::from(nonce_bytes), plaintext)
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt one stored part; fails if it was tampered with or the key is
/// wrong.
pub fn open(key: &[u8; KEY_LEN], stored: &[u8]) -> Result<Vec<u8>> {
    if stored.len() < NONCE_LEN + TAG_LEN {
        bail!("encrypted part is too short");
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("bad key: {e}"))?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&stored[..NONCE_LEN]);
    cipher
        .decrypt(&Nonce::from(nonce), &stored[NONCE_LEN..])
        .map_err(|_| anyhow!("decryption failed (wrong key or corrupted part)"))
}

/// Bytes added to each part by `seal`.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> [u8; KEY_LEN] {
        [b; KEY_LEN]
    }

    #[test]
    fn roundtrips_and_adds_fixed_overhead() {
        let data = b"some disk bytes".to_vec();
        let sealed = seal(&key(1), &data).unwrap();
        assert_eq!(sealed.len(), data.len() + OVERHEAD);
        assert_eq!(open(&key(1), &sealed).unwrap(), data);
    }

    #[test]
    fn nonces_differ_between_parts() {
        let a = seal(&key(1), b"same").unwrap();
        let b = seal(&key(1), b"same").unwrap();
        assert_ne!(a[..NONCE_LEN], b[..NONCE_LEN]);
        assert_ne!(a, b);
    }

    #[test]
    fn rejects_tampering_wrong_key_and_truncation() {
        let mut sealed = seal(&key(1), b"payload").unwrap();
        assert!(open(&key(2), &sealed).is_err());
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(open(&key(1), &sealed).is_err());
        assert!(open(&key(1), &[0u8; 5]).is_err());
    }

    #[test]
    fn parses_hex_keys() {
        let k = parse_key_hex(&"ab".repeat(32)).unwrap();
        assert_eq!(k, [0xab; 32]);
        assert!(parse_key_hex("short").is_err());
        assert!(parse_key_hex(&"zz".repeat(32)).is_err());
    }
}
