// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Backup agent core: streams VM disk images to S3-compatible storage in
//! fixed-size multipart parts (optionally AES-256-GCM encrypted per part),
//! records per-part and whole-disk SHA-256 in a manifest, applies Object
//! Lock retention, uploads the manifest last, then reads everything back
//! and re-verifies the checksums before reporting success.
//!
//! The `backup-agent` binary is a thin wrapper that reads its configuration
//! from environment variables (`AgentConfig::from_env`) and prints progress
//! and result as JSON lines.

use super::crypto;
use super::manifest::{
    disk_key, manifest_key, object_prefix, BackupManifest, DiskDigest, DiskEntry, EncryptionInfo,
    MANIFEST_VERSION,
};
use super::s3::sigv4::{sha256_hex, Credentials};
use super::s3::{LockMode, ObjectLock, S3Client, S3Config};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

pub const MIN_PART_BYTES: usize = 5 * 1024 * 1024;
pub const DEFAULT_PART_MB: usize = 64;
const PART_RETRIES: u32 = 3;

#[derive(Debug, Clone, Deserialize)]
pub struct DiskInput {
    pub name: String,
    pub pvc: String,
    /// Path of the disk image inside the pod (a mounted, read-only PVC).
    pub path: String,
    /// The PVC is `volumeMode: Block` and `path` is its raw device.
    #[serde(default)]
    pub block: bool,
}

#[derive(Clone)]
pub struct AgentConfig {
    pub s3: S3Config,
    pub prefix: String,
    pub backup_id: String,
    pub vm_name: String,
    pub namespace: String,
    pub snapshot_name: String,
    pub vm_spec: serde_json::Value,
    pub disks: Vec<DiskInput>,
    pub lock: Option<ObjectLock>,
    pub encryption_key: Option<[u8; crypto::KEY_LEN]>,
    pub encryption_key_id: String,
    pub part_bytes: usize,
    pub verify_readback: bool,
}

#[derive(Debug, Serialize)]
pub struct DiskSummary {
    pub name: String,
    pub size_bytes: u64,
    pub stored_bytes: u64,
    pub sha256: String,
    /// The source volume was Block mode (restore defaults to the same mode).
    pub source_block: bool,
}

#[derive(Debug, Serialize)]
pub struct AgentResult {
    pub backup_id: String,
    pub manifest_key: String,
    pub disks: Vec<DiskSummary>,
    pub encrypted: bool,
    pub locked_until: Option<String>,
    pub verified: bool,
    /// The VM spec captured in the manifest, so a restore can be planned
    /// without reading the object store.
    pub vm_spec: serde_json::Value,
}

fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

impl AgentConfig {
    /// Build from environment-style lookups (injectable for tests).
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let req = |k: &str| {
            get(k)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow!("{k} is not set"))
        };
        let opt = |k: &str| get(k).filter(|v| !v.trim().is_empty());

        let disks: Vec<DiskInput> = serde_json::from_str(&req("BACKUP_DISKS")?)
            .context("BACKUP_DISKS is not valid JSON")?;
        if disks.is_empty() {
            bail!("BACKUP_DISKS lists no disks");
        }
        for d in &disks {
            if !dns_label(&d.name) {
                bail!("invalid disk name '{}'", d.name);
            }
            // A Block-mode source is a raw device at exactly this node (see
            // `kube::backup_job::block_device_path`); anything else must be a
            // disk image under /disks/.
            let is_device = d.path == format!("/dev/zorvia-disk-{}", d.name);
            if !is_device && (!d.path.starts_with("/disks/") || d.path.contains("..")) {
                bail!(
                    "disk path '{}' must be under /disks/ or be /dev/zorvia-disk-{}",
                    d.path,
                    d.name
                );
            }
        }

        let lock = match opt("BACKUP_LOCK_MODE").as_deref() {
            None | Some("none") => None,
            Some(mode) => {
                let mode = match mode.to_ascii_uppercase().as_str() {
                    "GOVERNANCE" => LockMode::Governance,
                    "COMPLIANCE" => LockMode::Compliance,
                    other => bail!("unknown BACKUP_LOCK_MODE '{other}'"),
                };
                let days: i64 = req("BACKUP_RETAIN_DAYS")?
                    .parse()
                    .context("BACKUP_RETAIN_DAYS must be a number")?;
                if !(1..=36500).contains(&days) {
                    bail!("BACKUP_RETAIN_DAYS must be between 1 and 36500");
                }
                Some(ObjectLock {
                    mode,
                    retain_until: chrono::Utc::now() + chrono::Duration::days(days),
                })
            }
        };

        let part_mb: usize = opt("BACKUP_PART_MB")
            .map(|v| v.parse())
            .transpose()
            .context("BACKUP_PART_MB must be a number")?
            .unwrap_or(DEFAULT_PART_MB);
        let part_bytes = part_mb * 1024 * 1024;
        if part_bytes < MIN_PART_BYTES {
            bail!("BACKUP_PART_MB must be at least 5");
        }

        let encryption_key = opt("BACKUP_ENCRYPTION_KEY")
            .map(|k| crypto::parse_key_hex(&k))
            .transpose()?;

        Ok(Self {
            s3: S3Config {
                endpoint: req("BACKUP_S3_ENDPOINT")?,
                region: opt("BACKUP_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                bucket: req("BACKUP_S3_BUCKET")?,
                credentials: Credentials {
                    access_key: req("AWS_ACCESS_KEY_ID")?,
                    secret_key: req("AWS_SECRET_ACCESS_KEY")?,
                    session_token: opt("AWS_SESSION_TOKEN"),
                },
                send_checksums: opt("BACKUP_S3_CHECKSUMS").as_deref() != Some("false"),
            },
            prefix: opt("BACKUP_S3_PREFIX").unwrap_or_default(),
            backup_id: req("BACKUP_ID")?,
            vm_name: req("BACKUP_VM")?,
            namespace: req("BACKUP_NAMESPACE")?,
            snapshot_name: req("BACKUP_SNAPSHOT")?,
            vm_spec: opt("BACKUP_VM_SPEC")
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .context("BACKUP_VM_SPEC is not valid JSON")?
                .unwrap_or(serde_json::Value::Null),
            disks,
            lock,
            encryption_key,
            encryption_key_id: opt("BACKUP_ENCRYPTION_KEY_ID").unwrap_or_else(|| "default".into()),
            part_bytes,
            verify_readback: opt("BACKUP_VERIFY").as_deref() != Some("false"),
        })
    }
}

/// Fill `buf` from `r` until it is full or EOF; returns bytes read.
async fn read_full(r: &mut (impl AsyncReadExt + Unpin), buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let got = r.read(&mut buf[n..]).await?;
        if got == 0 {
            break;
        }
        n += got;
    }
    Ok(n)
}

async fn upload_part_with_retry(
    client: &S3Client,
    key: &str,
    upload_id: &str,
    number: u32,
    data: &[u8],
) -> Result<String> {
    let mut last = anyhow!("no attempt made");
    for attempt in 1..=PART_RETRIES {
        match client
            .upload_part(key, upload_id, number, data.to_vec())
            .await
        {
            Ok(etag) => return Ok(etag),
            Err(e) => {
                last = e;
                if attempt < PART_RETRIES {
                    tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
                }
            }
        }
    }
    Err(last.context(format!(
        "part {number} failed after {PART_RETRIES} attempts"
    )))
}

/// Length of a disk image. A raw block device reports a metadata length of 0,
/// so measure by seeking to the end (also correct for a regular file), then
/// rewind for reading.
async fn disk_len(file: &mut tokio::fs::File) -> Result<u64> {
    use tokio::io::AsyncSeekExt;
    let end = file.seek(std::io::SeekFrom::End(0)).await?;
    file.seek(std::io::SeekFrom::Start(0)).await?;
    Ok(end)
}

async fn upload_disk(
    cfg: &AgentConfig,
    client: &S3Client,
    disk: &DiskInput,
    key: &str,
    progress: &dyn Fn(u8, &str),
    pct_base: u8,
    pct_span: u8,
) -> Result<DiskEntry> {
    let mut file = tokio::fs::File::open(&disk.path)
        .await
        .with_context(|| format!("open disk image {}", disk.path))?;
    let total = disk_len(&mut file).await?;
    if total == 0 {
        bail!("disk image {} is empty", disk.path);
    }

    let upload_id = client.create_multipart(key, cfg.lock.as_ref()).await?;
    let result = async {
        let mut digest = DiskDigest::new();
        let mut etags: Vec<(u32, String)> = Vec::new();
        let mut buf = vec![0u8; cfg.part_bytes];
        let mut done = 0u64;
        loop {
            let n = read_full(&mut file, &mut buf).await?;
            if n == 0 {
                break;
            }
            let plain = &buf[..n];
            let stored = match &cfg.encryption_key {
                Some(k) => crypto::seal(k, plain)?,
                None => plain.to_vec(),
            };
            digest.add_part(plain, &stored);
            let number = etags.len() as u32 + 1;
            let etag = upload_part_with_retry(client, key, &upload_id, number, &stored).await?;
            etags.push((number, etag));
            done += n as u64;
            let pct = pct_base as u64 + pct_span as u64 * done / total.max(1);
            progress(pct.min(99) as u8, &format!("uploading {}", disk.name));
            if n < cfg.part_bytes {
                break;
            }
        }
        if done != total {
            bail!(
                "disk {} changed size during backup (read {done} of {total} bytes)",
                disk.path
            );
        }
        client.complete_multipart(key, &upload_id, &etags).await?;
        let mut entry = digest.finish(&disk.name, &disk.pvc, key);
        entry.source_block = disk.block;
        Ok::<_, anyhow::Error>(entry)
    }
    .await;

    if result.is_err() {
        if let Err(e) = client.abort_multipart(key, &upload_id).await {
            eprintln!("warning: could not abort multipart upload for {key}: {e}");
        }
    }
    result
}

/// Stream a stored disk object, verifying it against its manifest entry as it
/// arrives, and hand each verified plaintext part to `sink` in order. Checks
/// the object size, every part's SHA-256 (before decrypting or using it), and
/// finally the whole-disk plaintext SHA-256. Shared by verification, restore
/// and recovery drills.
pub async fn stream_disk(
    client: &S3Client,
    disk: &DiskEntry,
    key: Option<&[u8; crypto::KEY_LEN]>,
    mut sink: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    use sha2::{Digest, Sha256};

    let meta = client
        .head_object(&disk.object_key)
        .await?
        .ok_or_else(|| anyhow!("object {} is missing", disk.object_key))?;
    if meta.size != disk.stored_bytes {
        bail!(
            "object {} is {} bytes, manifest says {}",
            disk.object_key,
            meta.size,
            disk.stored_bytes
        );
    }

    let mut resp = client.get_object_response(&disk.object_key).await?;
    let mut whole = Sha256::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut idx = 0usize;

    let mut handle = |part: &[u8], idx: usize| -> Result<()> {
        let entry = disk
            .parts
            .get(idx)
            .ok_or_else(|| anyhow!("object has more data than the manifest lists"))?;
        if part.len() as u64 != entry.size || sha256_hex(part) != entry.sha256 {
            bail!("part {} of {} is corrupted", entry.number, disk.object_key);
        }
        let plain: std::borrow::Cow<[u8]> = match key {
            Some(k) => std::borrow::Cow::Owned(crypto::open(k, part)?),
            None => std::borrow::Cow::Borrowed(part),
        };
        whole.update(&*plain);
        sink(&plain)
    };

    while let Some(chunk) = resp.chunk().await? {
        pending.extend_from_slice(&chunk);
        while idx < disk.parts.len() && pending.len() as u64 >= disk.parts[idx].size {
            let size = disk.parts[idx].size as usize;
            let part: Vec<u8> = pending.drain(..size).collect();
            handle(&part, idx)?;
            idx += 1;
        }
    }
    if idx != disk.parts.len() || !pending.is_empty() {
        bail!(
            "object {} does not match its manifest parts",
            disk.object_key
        );
    }
    let digest: String = whole
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != disk.sha256 {
        bail!("whole-disk checksum mismatch for {}", disk.name);
    }
    Ok(())
}

/// Check one stored part against the manifest and, when `key` is given, prove
/// that the key decrypts it. Returns the plaintext length. This is what makes a
/// key check cheap: a single part per disk, not a whole-disk restore.
pub fn check_first_part(
    disk: &DiskEntry,
    part: &[u8],
    key: Option<&[u8; crypto::KEY_LEN]>,
) -> Result<usize> {
    let entry = disk
        .parts
        .first()
        .ok_or_else(|| anyhow!("disk '{}' lists no parts", disk.name))?;
    if part.len() as u64 != entry.size || sha256_hex(part) != entry.sha256 {
        bail!("first part of disk '{}' is corrupted", disk.name);
    }
    match key {
        Some(k) => Ok(crypto::open(k, part)
            .with_context(|| format!("disk '{}'", disk.name))?
            .len()),
        None => Ok(part.len()),
    }
}

#[derive(Clone)]
pub struct VerifyKeyConfig {
    pub s3: S3Config,
    pub manifest_key: String,
    pub encryption_key: Option<[u8; crypto::KEY_LEN]>,
}

#[derive(Debug, Serialize)]
pub struct VerifyKeyResult {
    pub manifest_key: String,
    pub encrypted: bool,
    /// Always true on success: every disk's first part matched the manifest and,
    /// for an encrypted backup, decrypted with the supplied key.
    pub key_ok: bool,
    pub disks_checked: usize,
    pub bytes_read: u64,
}

impl VerifyKeyConfig {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let req = |k: &str| {
            get(k)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow!("{k} is not set"))
        };
        let opt = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let manifest_key = req("RESTORE_MANIFEST_KEY")?;
        if manifest_key.starts_with('/') || manifest_key.contains("..") {
            bail!("invalid RESTORE_MANIFEST_KEY");
        }
        Ok(Self {
            s3: S3Config {
                endpoint: req("BACKUP_S3_ENDPOINT")?,
                region: opt("BACKUP_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                bucket: req("BACKUP_S3_BUCKET")?,
                credentials: Credentials {
                    access_key: req("AWS_ACCESS_KEY_ID")?,
                    secret_key: req("AWS_SECRET_ACCESS_KEY")?,
                    session_token: opt("AWS_SESSION_TOKEN"),
                },
                send_checksums: true,
            },
            manifest_key,
            encryption_key: opt("BACKUP_ENCRYPTION_KEY")
                .map(|k| crypto::parse_key_hex(&k))
                .transpose()?,
        })
    }
}

/// Prove the encryption key still opens a backup without restoring it: read the
/// manifest, then the first part of each disk object (the response is dropped as
/// soon as that part has arrived) and check it against the manifest and the key.
/// A wrong key fails here, long before anyone needs the data.
pub async fn run_verify_key(
    cfg: &VerifyKeyConfig,
    client: &S3Client,
    progress: &dyn Fn(u8, &str),
) -> Result<VerifyKeyResult> {
    progress(1, "reading manifest");
    let manifest = BackupManifest::from_json(&client.get_object(&cfg.manifest_key).await?)
        .context("backup manifest is invalid")?;
    let encrypted = manifest.encryption.is_some();
    if encrypted && cfg.encryption_key.is_none() {
        bail!("this backup is encrypted but no BACKUP_ENCRYPTION_KEY was provided");
    }
    if manifest.disks.is_empty() {
        bail!("the backup lists no disks");
    }
    let n = manifest.disks.len() as u64;
    let mut bytes_read = 0u64;
    for (i, disk) in manifest.disks.iter().enumerate() {
        progress(
            (5 + 90 * i as u64 / n) as u8,
            &format!("checking disk {}", disk.name),
        );
        let first = disk
            .parts
            .first()
            .ok_or_else(|| anyhow!("disk '{}' lists no parts", disk.name))?;
        let mut resp = client.get_object_response(&disk.object_key).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(first.size as usize);
        while (buf.len() as u64) < first.size {
            match resp.chunk().await? {
                Some(c) => buf.extend_from_slice(&c),
                None => bail!("object {} ended before its first part", disk.object_key),
            }
        }
        buf.truncate(first.size as usize);
        drop(resp);
        check_first_part(disk, &buf, cfg.encryption_key.as_ref())?;
        bytes_read += first.size;
    }
    Ok(VerifyKeyResult {
        manifest_key: cfg.manifest_key.clone(),
        encrypted,
        key_ok: true,
        disks_checked: manifest.disks.len(),
        bytes_read,
    })
}

/// Verify a stored disk without keeping its contents.
pub async fn verify_disk(
    client: &S3Client,
    disk: &DiskEntry,
    key: Option<&[u8; crypto::KEY_LEN]>,
) -> Result<()> {
    stream_disk(client, disk, key, |_| Ok(())).await
}

/// Run the backup. `progress(percent, phase)` is called as work advances.
pub async fn run(
    cfg: &AgentConfig,
    client: &S3Client,
    progress: &dyn Fn(u8, &str),
) -> Result<AgentResult> {
    let prefix = object_prefix(&cfg.prefix, &cfg.vm_name, &cfg.backup_id);
    let mfst_key = manifest_key(&prefix);
    if client.head_object(&mfst_key).await?.is_some() {
        bail!("backup {} already exists in the bucket", cfg.backup_id);
    }

    let n = cfg.disks.len() as u64;
    let upload_span = if cfg.verify_readback { 60u64 } else { 95 };
    let mut entries = Vec::new();
    for (i, disk) in cfg.disks.iter().enumerate() {
        let base = 2 + (upload_span * i as u64 / n) as u8;
        let span = (upload_span / n).max(1) as u8;
        let key = disk_key(&prefix, &disk.name);
        entries.push(upload_disk(cfg, client, disk, &key, progress, base, span).await?);
    }

    let manifest = BackupManifest {
        version: MANIFEST_VERSION,
        backup_id: cfg.backup_id.clone(),
        vm_name: cfg.vm_name.clone(),
        namespace: cfg.namespace.clone(),
        created: chrono::Utc::now().to_rfc3339(),
        zorvia_version: env!("CARGO_PKG_VERSION").to_string(),
        snapshot_name: cfg.snapshot_name.clone(),
        vm_spec: cfg.vm_spec.clone(),
        disks: entries,
        encryption: cfg.encryption_key.map(|_| EncryptionInfo {
            algorithm: crypto::ALGORITHM.to_string(),
            key_id: cfg.encryption_key_id.clone(),
        }),
    };
    manifest.validate()?;

    // Read everything back before the manifest exists: a manifest is only
    // published for a backup that verified.
    if cfg.verify_readback {
        for (i, disk) in manifest.disks.iter().enumerate() {
            progress(
                (62 + 35 * i as u64 / n) as u8,
                &format!("verifying {}", disk.name),
            );
            verify_disk(client, disk, cfg.encryption_key.as_ref())
                .await
                .with_context(|| format!("read-back verification of {}", disk.name))?;
        }
    }

    progress(98, "writing manifest");
    client
        .put_object(
            &mfst_key,
            manifest.to_json()?,
            "application/json",
            cfg.lock.as_ref(),
        )
        .await?;

    Ok(AgentResult {
        backup_id: cfg.backup_id.clone(),
        manifest_key: mfst_key,
        disks: manifest
            .disks
            .iter()
            .map(|d| DiskSummary {
                name: d.name.clone(),
                size_bytes: d.size_bytes,
                stored_bytes: d.stored_bytes,
                sha256: d.sha256.clone(),
                source_block: d.source_block,
            })
            .collect(),
        encrypted: cfg.encryption_key.is_some(),
        locked_until: cfg.lock.as_ref().map(|l| l.retain_until.to_rfc3339()),
        verified: cfg.verify_readback,
        vm_spec: manifest.vm_spec.clone(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreDisk {
    /// Disk name as recorded in the manifest.
    pub name: String,
    /// Where to write it: a `disk.img` on a freshly provisioned, writable
    /// Filesystem PVC mount, or (`block`) the raw device of a Block PVC.
    pub path: String,
    #[serde(default)]
    pub block: bool,
}

/// Device node a Block-mode restore target is attached at (see
/// `kube::backup_job::restore_device_path`).
fn restore_device_path(name: &str) -> String {
    format!("/dev/zorvia-restore-{name}")
}

/// Open the destination of one disk. A file is created and must not exist; a Block
/// device already exists, is opened for writing without truncation and must be at
/// least `need` bytes. The bool says whether the target is a device (never deleted
/// on failure).
fn open_restore_target(path: &str, device: bool, need: u64) -> Result<std::fs::File> {
    use std::io::{Seek, SeekFrom};
    if !device {
        return std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("create {path} (must not already exist)"));
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("open block device {path}"))?;
    let size = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(0))?;
    if size < need {
        bail!("block device {path} is {size} bytes, the backup needs {need}");
    }
    Ok(f)
}

#[derive(Clone)]
pub struct RestoreConfig {
    pub s3: S3Config,
    pub manifest_key: String,
    pub disks: Vec<RestoreDisk>,
    pub encryption_key: Option<[u8; crypto::KEY_LEN]>,
}

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub backup_id: String,
    pub restored: Vec<DiskSummary>,
}

impl RestoreConfig {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let req = |k: &str| {
            get(k)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow!("{k} is not set"))
        };
        let opt = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let disks: Vec<RestoreDisk> = serde_json::from_str(&req("RESTORE_DISKS")?)
            .context("RESTORE_DISKS is not valid JSON")?;
        if disks.is_empty() {
            bail!("RESTORE_DISKS lists no disks");
        }
        for d in &disks {
            if !dns_label(&d.name) {
                bail!("invalid disk name '{}'", d.name);
            }
            let device_ok = d.block && d.path == restore_device_path(&d.name);
            if !device_ok && (d.block || !d.path.starts_with("/restore/") || d.path.contains(".."))
            {
                bail!(
                    "restore path '{}' must be under /restore/ (or /dev/zorvia-restore-{} for a Block target)",
                    d.path,
                    d.name
                );
            }
        }
        let manifest_key = req("RESTORE_MANIFEST_KEY")?;
        if manifest_key.starts_with('/') || manifest_key.contains("..") {
            bail!("invalid RESTORE_MANIFEST_KEY");
        }
        Ok(Self {
            s3: S3Config {
                endpoint: req("BACKUP_S3_ENDPOINT")?,
                region: opt("BACKUP_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                bucket: req("BACKUP_S3_BUCKET")?,
                credentials: Credentials {
                    access_key: req("AWS_ACCESS_KEY_ID")?,
                    secret_key: req("AWS_SECRET_ACCESS_KEY")?,
                    session_token: opt("AWS_SESSION_TOKEN"),
                },
                send_checksums: true,
            },
            manifest_key,
            disks,
            encryption_key: opt("BACKUP_ENCRYPTION_KEY")
                .map(|k| crypto::parse_key_hex(&k))
                .transpose()?,
        })
    }
}

/// Restore the requested disks of a backup onto writable paths. Every part is
/// verified before it is written and the whole-disk checksum is checked at the
/// end; on any failure the partially written file is removed. Refuses to
/// overwrite an existing file.
pub async fn run_restore(
    cfg: &RestoreConfig,
    client: &S3Client,
    progress: &dyn Fn(u8, &str),
) -> Result<RestoreResult> {
    use std::io::Write;

    progress(1, "reading manifest");
    let manifest = BackupManifest::from_json(&client.get_object(&cfg.manifest_key).await?)
        .context("backup manifest is invalid")?;
    if manifest.encryption.is_some() && cfg.encryption_key.is_none() {
        bail!("this backup is encrypted but no BACKUP_ENCRYPTION_KEY was provided");
    }

    let n = cfg.disks.len() as u64;
    let mut restored = Vec::new();
    for (i, want) in cfg.disks.iter().enumerate() {
        let disk = manifest
            .disks
            .iter()
            .find(|d| d.name == want.name)
            .ok_or_else(|| anyhow!("backup has no disk named '{}'", want.name))?;
        progress(
            (2 + 96 * i as u64 / n) as u8,
            &format!("restoring {}", disk.name),
        );

        let mut file = open_restore_target(&want.path, want.block, disk.size_bytes)?;
        let mut written = 0u64;
        // This disk's slice of the 2..98 progress range.
        let base = 2 + 96 * i as u64 / n;
        let span = (96 / n).max(1);
        let result = stream_disk(client, disk, cfg.encryption_key.as_ref(), |plain| {
            file.write_all(plain)?;
            written += plain.len() as u64;
            let pct = base + span * written / disk.size_bytes.max(1);
            progress(pct.min(98) as u8, &format!("restoring {}", disk.name));
            Ok(())
        })
        .await
        .and_then(|_| {
            file.sync_all()?;
            if written != disk.size_bytes {
                bail!(
                    "wrote {written} bytes for '{}', expected {}",
                    disk.name,
                    disk.size_bytes
                );
            }
            Ok(())
        });
        drop(file);
        if let Err(e) = result {
            // A created file is removed; a Block device is the volume itself and stays.
            if !want.block {
                let _ = std::fs::remove_file(&want.path);
            }
            return Err(e.context(format!("restoring disk '{}'", disk.name)));
        }
        restored.push(DiskSummary {
            name: disk.name.clone(),
            size_bytes: disk.size_bytes,
            stored_bytes: disk.stored_bytes,
            sha256: disk.sha256.clone(),
            source_block: disk.source_block,
        });
    }
    progress(99, "done");
    Ok(RestoreResult {
        backup_id: manifest.backup_id,
        restored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn disk_with_first_part(stored: &[u8]) -> DiskEntry {
        DiskEntry {
            name: "root".into(),
            pvc: "p".into(),
            object_key: "vm/op/root".into(),
            size_bytes: 0,
            stored_bytes: stored.len() as u64,
            sha256: String::new(),
            parts: vec![super::super::manifest::PartEntry {
                number: 1,
                size: stored.len() as u64,
                sha256: sha256_hex(stored),
            }],
            source_block: false,
        }
    }

    #[test]
    fn first_part_check_proves_the_key_and_catches_corruption() {
        let key = [7u8; crypto::KEY_LEN];
        let stored = crypto::seal(&key, b"first part of a disk").unwrap();
        let disk = disk_with_first_part(&stored);
        assert_eq!(check_first_part(&disk, &stored, Some(&key)).unwrap(), 20);
        // The manifest hash matches, but the wrong key cannot open it.
        let wrong = [8u8; crypto::KEY_LEN];
        let e = check_first_part(&disk, &stored, Some(&wrong)).unwrap_err();
        assert!(format!("{e:#}").contains("wrong key or corrupted"), "{e:#}");
        // A flipped byte is caught by the manifest checksum before decryption.
        let mut bad = stored.clone();
        bad[20] ^= 1;
        let e = check_first_part(&disk, &bad, Some(&key)).unwrap_err();
        assert!(format!("{e:#}").contains("corrupted"));
        // Unencrypted backups still get the integrity check.
        let plain = b"plain part".to_vec();
        let d = disk_with_first_part(&plain);
        assert_eq!(check_first_part(&d, &plain, None).unwrap(), plain.len());
    }

    #[test]
    fn verify_key_config_needs_a_safe_manifest_key() {
        let base = |extra: &[(&str, &str)]| {
            let mut m: HashMap<String, String> = [
                ("BACKUP_S3_ENDPOINT", "http://s3"),
                ("BACKUP_S3_BUCKET", "b"),
                ("AWS_ACCESS_KEY_ID", "AK"),
                ("AWS_SECRET_ACCESS_KEY", "SK"),
                ("RESTORE_MANIFEST_KEY", "vm/op/manifest.json"),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
            for (k, v) in extra {
                m.insert(k.to_string(), v.to_string());
            }
            VerifyKeyConfig::from_env(move |k| m.get(k).cloned())
        };
        assert!(base(&[]).is_ok());
        assert!(base(&[("RESTORE_MANIFEST_KEY", "/etc/passwd")]).is_err());
        assert!(base(&[("RESTORE_MANIFEST_KEY", "a/../b")]).is_err());
        assert!(base(&[("BACKUP_ENCRYPTION_KEY", "short")]).is_err());
        assert!(base(&[("BACKUP_ENCRYPTION_KEY", &"ab".repeat(32))]).is_ok());
    }

    fn env(extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let mut m: HashMap<String, String> = [
            ("BACKUP_S3_ENDPOINT", "http://minio:9000"),
            ("BACKUP_S3_BUCKET", "vm-backups"),
            ("AWS_ACCESS_KEY_ID", "AK"),
            ("AWS_SECRET_ACCESS_KEY", "SK"),
            ("BACKUP_ID", "bk-1"),
            ("BACKUP_VM", "web-1"),
            ("BACKUP_NAMESPACE", "default"),
            ("BACKUP_SNAPSHOT", "backup-web-1-abc"),
            (
                "BACKUP_DISKS",
                r#"[{"name":"root","pvc":"web-1-root","path":"/disks/root/disk.img"}]"#,
            ),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        for (k, v) in extra {
            m.insert(k.to_string(), v.to_string());
        }
        move |k| m.get(k).cloned()
    }

    #[test]
    fn parses_minimal_config_with_defaults() {
        let cfg = AgentConfig::from_env(env(&[])).unwrap();
        assert_eq!(cfg.part_bytes, 64 * 1024 * 1024);
        assert!(cfg.lock.is_none() && cfg.encryption_key.is_none());
        assert!(cfg.verify_readback && cfg.s3.send_checksums);
        assert_eq!(cfg.s3.region, "us-east-1");
        assert_eq!(cfg.disks[0].path, "/disks/root/disk.img");
    }

    #[test]
    fn parses_lock_and_encryption() {
        let cfg = AgentConfig::from_env(env(&[
            ("BACKUP_LOCK_MODE", "compliance"),
            ("BACKUP_RETAIN_DAYS", "30"),
            ("BACKUP_ENCRYPTION_KEY", &"ab".repeat(32)),
        ]))
        .unwrap();
        let lock = cfg.lock.unwrap();
        assert_eq!(lock.mode, LockMode::Compliance);
        assert!(lock.retain_until > chrono::Utc::now() + chrono::Duration::days(29));
        assert_eq!(cfg.encryption_key, Some([0xab; 32]));
    }

    #[test]
    fn rejects_bad_config() {
        let bad = |extra: &[(&str, &str)]| AgentConfig::from_env(env(extra)).is_err();
        assert!(bad(&[("BACKUP_LOCK_MODE", "COMPLIANCE")])); // needs retain days
        assert!(bad(&[
            ("BACKUP_LOCK_MODE", "WORM"),
            ("BACKUP_RETAIN_DAYS", "1")
        ]));
        assert!(bad(&[("BACKUP_PART_MB", "1")]));
        assert!(bad(&[("BACKUP_ENCRYPTION_KEY", "tooshort")]));
        assert!(bad(&[("BACKUP_DISKS", "[]")]));
        assert!(bad(&[(
            "BACKUP_DISKS",
            r#"[{"name":"root","pvc":"p","path":"/etc/passwd"}]"#
        )]));
        assert!(bad(&[(
            "BACKUP_DISKS",
            r#"[{"name":"Bad_Name","pvc":"p","path":"/disks/x/disk.img"}]"#
        )]));
        assert!(bad(&[(
            "BACKUP_DISKS",
            r#"[{"name":"root","pvc":"p","path":"/disks/../etc/shadow"}]"#
        )]));
        // A device node is accepted only at the exact path for that disk name.
        assert!(bad(&[(
            "BACKUP_DISKS",
            r#"[{"name":"data","pvc":"p","path":"/dev/sda"}]"#
        )]));
        assert!(bad(&[(
            "BACKUP_DISKS",
            r#"[{"name":"data","pvc":"p","path":"/dev/zorvia-disk-other"}]"#
        )]));
    }

    #[test]
    fn accepts_the_block_device_path_for_its_own_disk() {
        let cfg = AgentConfig::from_env(env(&[(
            "BACKUP_DISKS",
            r#"[{"name":"data","pvc":"p","path":"/dev/zorvia-disk-data"}]"#,
        )]))
        .unwrap();
        assert_eq!(cfg.disks[0].path, "/dev/zorvia-disk-data");
    }

    #[test]
    fn restore_accepts_only_the_exact_block_target_for_a_block_disk() {
        let base = |disks: &str| RestoreConfig::from_env(restore_env(&[("RESTORE_DISKS", disks)]));
        assert!(
            base(r#"[{"name":"data","path":"/dev/zorvia-restore-data","block":true}]"#).is_ok()
        );
        // Block flag without the exact device path, device path without the flag, or any other device.
        assert!(base(r#"[{"name":"data","path":"/restore/data/disk.img","block":true}]"#).is_err());
        assert!(base(r#"[{"name":"data","path":"/dev/zorvia-restore-data"}]"#).is_err());
        assert!(base(r#"[{"name":"data","path":"/dev/sda","block":true}]"#).is_err());
        assert!(
            base(r#"[{"name":"data","path":"/dev/zorvia-restore-other","block":true}]"#).is_err()
        );
    }

    #[test]
    fn block_target_opening_checks_size_and_never_creates_or_truncates() {
        let dir = std::env::temp_dir().join(format!("zorvia-blk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dev = dir.join("device");
        std::fs::write(&dev, vec![9u8; 4096]).unwrap();
        let p = dev.to_str().unwrap();
        // Big enough: opened, contents kept (no truncation).
        drop(open_restore_target(p, true, 4096).unwrap());
        assert_eq!(std::fs::metadata(&dev).unwrap().len(), 4096);
        // Too small for the backup: refused.
        let e = open_restore_target(p, true, 8192).unwrap_err();
        assert!(format!("{e:#}").contains("needs 8192"), "{e:#}");
        // A missing device is an error, not a new file.
        assert!(open_restore_target(dir.join("nope").to_str().unwrap(), true, 1).is_err());
        // Filesystem targets must not exist yet.
        assert!(open_restore_target(p, false, 1).is_err());
        let fresh = dir.join("fresh.img");
        assert!(open_restore_target(fresh.to_str().unwrap(), false, 1).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn restore_env(extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let mut m: HashMap<String, String> = [
            ("BACKUP_S3_ENDPOINT", "http://minio:9000"),
            ("BACKUP_S3_BUCKET", "vm-backups"),
            ("AWS_ACCESS_KEY_ID", "AK"),
            ("AWS_SECRET_ACCESS_KEY", "SK"),
            ("RESTORE_MANIFEST_KEY", "web-1/op-1/manifest.json"),
            (
                "RESTORE_DISKS",
                r#"[{"name":"rootdisk","path":"/restore/rootdisk/disk.img"}]"#,
            ),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        for (k, v) in extra {
            m.insert(k.to_string(), v.to_string());
        }
        move |k| m.get(k).cloned()
    }

    #[test]
    fn parses_restore_config_and_rejects_bad_paths() {
        let cfg = RestoreConfig::from_env(restore_env(&[])).unwrap();
        assert_eq!(cfg.disks[0].path, "/restore/rootdisk/disk.img");
        assert!(cfg.encryption_key.is_none());
        let bad = |extra: &[(&str, &str)]| RestoreConfig::from_env(restore_env(extra)).is_err();
        assert!(bad(&[("RESTORE_MANIFEST_KEY", "../x")]));
        assert!(bad(&[("RESTORE_MANIFEST_KEY", "/abs")]));
        assert!(bad(&[("RESTORE_DISKS", "[]")]));
        assert!(bad(&[(
            "RESTORE_DISKS",
            r#"[{"name":"root","path":"/disks/root/disk.img"}]"#
        )]));
        assert!(bad(&[(
            "RESTORE_DISKS",
            r#"[{"name":"root","path":"/restore/../etc/x"}]"#
        )]));
        assert!(bad(&[("BACKUP_ENCRYPTION_KEY", "short")]));
    }

    #[tokio::test]
    async fn disk_len_measures_by_seeking_and_rewinds() {
        let path = std::env::temp_dir().join(format!("zorvia-disk-len-{}", std::process::id()));
        std::fs::write(&path, vec![1u8; 4096]).unwrap();
        let mut f = tokio::fs::File::open(&path).await.unwrap();
        assert_eq!(disk_len(&mut f).await.unwrap(), 4096);
        let mut first = [0u8; 4];
        f.read_exact(&mut first).await.unwrap();
        assert_eq!(
            first, [1; 4],
            "reading must start at offset 0 after measuring"
        );
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn read_full_fills_buffer_across_short_reads() {
        let data = [7u8; 10];
        let mut r = &data[..];
        let mut buf = [0u8; 4];
        assert_eq!(read_full(&mut r, &mut buf).await.unwrap(), 4);
        let mut big = [0u8; 20];
        assert_eq!(read_full(&mut r, &mut big).await.unwrap(), 6);
    }
}
