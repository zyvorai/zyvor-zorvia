// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Off-cluster backup manifest: what was uploaded, where, and the checksums
//! a restore (or a recovery drill) verifies against. Stored next to the disk
//! objects as `manifest.json`, uploaded last so a manifest's presence means
//! every disk it lists was fully written.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartEntry {
    pub number: u32,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskEntry {
    pub name: String,
    pub pvc: String,
    pub object_key: String,
    /// Size of the disk image itself (plaintext).
    pub size_bytes: u64,
    /// Bytes stored in the object store: equals `size_bytes` unless the
    /// backup is encrypted (each part then carries nonce + tag overhead).
    pub stored_bytes: u64,
    /// SHA-256 of the whole plaintext disk image (hex).
    pub sha256: String,
    /// Parts as stored: `size` and `sha256` describe the stored bytes
    /// (ciphertext when encrypted).
    pub parts: Vec<PartEntry>,
    /// The source volume was `volumeMode: Block` (read as a raw device). A restore
    /// defaults to the same mode. Absent in older manifests, which means Filesystem.
    #[serde(default)]
    pub source_block: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncryptionInfo {
    pub algorithm: String,
    /// Operator-chosen label for the key (never the key itself).
    pub key_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: u32,
    pub backup_id: String,
    pub vm_name: String,
    pub namespace: String,
    pub created: String,
    pub zorvia_version: String,
    /// The cluster-local VirtualMachineSnapshot this was taken from.
    pub snapshot_name: String,
    /// The VirtualMachine spec at snapshot time, for recreating the VM.
    pub vm_spec: serde_json::Value,
    pub disks: Vec<DiskEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionInfo>,
}

/// Object key layout: `<prefix>/<vm>/<backup-id>/…`.
pub fn object_prefix(prefix: &str, vm_name: &str, backup_id: &str) -> String {
    let p = prefix.trim_matches('/');
    if p.is_empty() {
        format!("{vm_name}/{backup_id}")
    } else {
        format!("{p}/{vm_name}/{backup_id}")
    }
}

pub fn manifest_key(object_prefix: &str) -> String {
    format!("{object_prefix}/manifest.json")
}

pub fn disk_key(object_prefix: &str, disk_name: &str) -> String {
    format!("{object_prefix}/disks/{disk_name}.raw")
}

impl BackupManifest {
    pub fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn from_json(bytes: &[u8]) -> anyhow::Result<Self> {
        let m: Self = serde_json::from_slice(bytes)?;
        if m.version != MANIFEST_VERSION {
            anyhow::bail!(
                "unsupported manifest version {} (expected {MANIFEST_VERSION})",
                m.version
            );
        }
        m.validate()?;
        Ok(m)
    }

    /// Internal consistency: part sizes add up to the disk size, parts are
    /// numbered 1..n, digests are well-formed.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.disks.is_empty() {
            anyhow::bail!("manifest lists no disks");
        }
        for d in &self.disks {
            if !is_sha256_hex(&d.sha256) {
                anyhow::bail!("disk '{}' has a malformed sha256", d.name);
            }
            if d.parts.is_empty() {
                anyhow::bail!("disk '{}' has no parts", d.name);
            }
            let mut total = 0u64;
            for (i, p) in d.parts.iter().enumerate() {
                if p.number as usize != i + 1 {
                    anyhow::bail!("disk '{}' parts are not numbered 1..n", d.name);
                }
                if !is_sha256_hex(&p.sha256) {
                    anyhow::bail!("disk '{}' part {} has a malformed sha256", d.name, p.number);
                }
                total += p.size;
            }
            if total != d.stored_bytes {
                anyhow::bail!(
                    "disk '{}' parts sum to {total} bytes but stored_bytes is {}",
                    d.name,
                    d.stored_bytes
                );
            }
            if self.encryption.is_none() && d.stored_bytes != d.size_bytes {
                anyhow::bail!(
                    "disk '{}' is unencrypted but stored != plaintext size",
                    d.name
                );
            }
        }
        Ok(())
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Incrementally digests a disk while it is read in fixed-size parts,
/// producing the `DiskEntry` checksums without holding the disk in memory.
pub struct DiskDigest {
    whole: Sha256,
    parts: Vec<PartEntry>,
    total: u64,
    stored: u64,
}

impl Default for DiskDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskDigest {
    pub fn new() -> Self {
        Self {
            whole: Sha256::new(),
            parts: Vec::new(),
            total: 0,
            stored: 0,
        }
    }

    /// Record the next part (in order). `plain` feeds the whole-disk
    /// digest; `stored` (the same bytes, or their ciphertext) is what the
    /// part entry describes. Returns the stored part's hex digest.
    pub fn add_part(&mut self, plain: &[u8], stored: &[u8]) -> String {
        self.whole.update(plain);
        let digest: String = Sha256::digest(stored)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.total += plain.len() as u64;
        self.stored += stored.len() as u64;
        self.parts.push(PartEntry {
            number: self.parts.len() as u32 + 1,
            size: stored.len() as u64,
            sha256: digest.clone(),
        });
        digest
    }

    pub fn finish(self, name: &str, pvc: &str, object_key: &str) -> DiskEntry {
        let sha256: String = self
            .whole
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        DiskEntry {
            name: name.to_string(),
            pvc: pvc.to_string(),
            object_key: object_key.to_string(),
            size_bytes: self.total,
            stored_bytes: self.stored,
            sha256,
            parts: self.parts,
            source_block: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(disk: DiskEntry) -> BackupManifest {
        BackupManifest {
            version: MANIFEST_VERSION,
            backup_id: "bk-1".into(),
            vm_name: "web-1".into(),
            namespace: "default".into(),
            created: "2026-09-30T00:00:00Z".into(),
            zorvia_version: "0.3.4".into(),
            snapshot_name: "backup-web-1-abc".into(),
            vm_spec: serde_json::json!({"spec": {}}),
            disks: vec![disk],
            encryption: None,
        }
    }

    fn sample_disk() -> DiskEntry {
        let mut d = DiskDigest::new();
        d.add_part(b"hello ", b"hello ");
        d.add_part(b"world", b"world");
        d.finish("root", "web-1-root", "web-1/bk-1/disks/root.raw")
    }

    #[test]
    fn digest_of_parts_equals_digest_of_whole() {
        let disk = sample_disk();
        let whole: String = Sha256::digest(b"hello world")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(disk.sha256, whole);
        assert_eq!(disk.size_bytes, 11);
        assert_eq!(disk.parts.len(), 2);
        assert_eq!(disk.parts[1].number, 2);
    }

    #[test]
    fn roundtrips_and_validates() {
        let m = manifest(sample_disk());
        let back = BackupManifest::from_json(&m.to_json().unwrap()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn rejects_inconsistent_manifests() {
        let mut bad_size = sample_disk();
        bad_size.stored_bytes += 1;
        assert!(manifest(bad_size).validate().is_err());

        let mut bad_number = sample_disk();
        bad_number.parts[0].number = 5;
        assert!(manifest(bad_number).validate().is_err());

        let mut bad_digest = sample_disk();
        bad_digest.sha256 = "xyz".into();
        assert!(manifest(bad_digest).validate().is_err());

        let mut empty = manifest(sample_disk());
        empty.disks.clear();
        assert!(empty.validate().is_err());

        let mut wrong_version = manifest(sample_disk());
        wrong_version.version = 99;
        assert!(BackupManifest::from_json(&wrong_version.to_json().unwrap()).is_err());
    }

    #[test]
    fn encrypted_disks_may_store_more_than_they_hold() {
        let mut d = DiskDigest::new();
        d.add_part(b"plain", b"plain+28bytes-of-overhead....");
        let disk = d.finish("root", "pvc", "k");
        assert_eq!(disk.size_bytes, 5);
        assert_eq!(disk.stored_bytes, 29);
        let mut m = manifest(disk);
        assert!(
            m.validate().is_err(),
            "unencrypted must have stored == size"
        );
        m.encryption = Some(EncryptionInfo {
            algorithm: "AES-256-GCM".into(),
            key_id: "k1".into(),
        });
        assert!(m.validate().is_ok());
    }

    #[test]
    fn key_layout() {
        assert_eq!(object_prefix("/prod/", "web-1", "bk-1"), "prod/web-1/bk-1");
        assert_eq!(object_prefix("", "web-1", "bk-1"), "web-1/bk-1");
        assert_eq!(manifest_key("a/b"), "a/b/manifest.json");
        assert_eq!(disk_key("a/b", "root"), "a/b/disks/root.raw");
    }
}
