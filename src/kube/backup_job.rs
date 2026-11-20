// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Builds the Kubernetes Job that runs `backup-agent` against a VM's disks:
//! each source PVC (restored from the VM snapshot) is mounted read-only under
//! `/disks/<name>/`, S3 settings arrive as plain env vars, and credentials and
//! the optional encryption key are injected from a Secret -- never placed in
//! the Job spec as values.

use anyhow::{bail, Result};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, EnvVarSource, PersistentVolumeClaimVolumeSource, PodSecurityContext,
    PodSpec, PodTemplateSpec, ResourceRequirements, SecretKeySelector, SecurityContext, Volume,
    VolumeDevice, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

/// Where each disk's PVC is mounted; the agent reads `<mount>/disk.img`.
pub const DISKS_ROOT: &str = "/disks";
/// Where restore Jobs mount the (writable) target PVCs.
pub const RESTORE_ROOT: &str = "/restore";

pub fn backup_agent_image() -> String {
    std::env::var("ZORVIA_BACKUP_AGENT_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/zyvorai/zorvia-backup-agent:latest".to_string())
}

/// UID that owns CDI-provisioned disk images (`qemu`).
fn agent_uid() -> i64 {
    std::env::var("ZORVIA_BACKUP_AGENT_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(107)
}

#[derive(Debug, Clone)]
pub struct BackupDisk {
    pub name: String,
    pub pvc: String,
    /// The PVC is `volumeMode: Block`: attach it as a raw device
    /// (`/dev/zorvia-disk-<name>`) instead of mounting a filesystem. Backup only.
    pub block: bool,
}

/// Device node a Block-mode source PVC is attached at (backup).
fn block_device_path(name: &str) -> String {
    format!("/dev/zorvia-disk-{name}")
}

/// Device node a Block-mode restore target PVC is attached at (read-write).
fn restore_device_path(name: &str) -> String {
    format!("/dev/zorvia-restore-{name}")
}

/// Device node of a Block disk for this Job direction.
fn device_path_for(mode: Mode, name: &str) -> String {
    match mode {
        Mode::Restore { .. } => restore_device_path(name),
        _ => block_device_path(name),
    }
}

/// Names of the Secret keys holding credentials.
#[derive(Debug, Clone)]
pub struct BackupSecretRef {
    pub secret_name: String,
    pub access_key_key: String,
    pub secret_key_key: String,
    /// Key holding the 64-hex-char encryption key, when encrypting.
    pub encryption_key_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BackupJobSpec {
    pub backup_id: String,
    pub vm_name: String,
    pub snapshot_name: String,
    pub disks: Vec<BackupDisk>,
    /// Non-secret `BACKUP_*` env (endpoint, bucket, prefix, lock mode, ...).
    pub env: Vec<(String, String)>,
    pub secrets: BackupSecretRef,
    pub active_deadline_secs: i64,
}

fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn secret_env(name: &str, secret: &str, key: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value_from: Some(EnvVarSource {
            secret_key_ref: Some(SecretKeySelector {
                name: secret.to_string(),
                key: key.to_string(),
                optional: Some(false),
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
enum Mode<'a> {
    Backup,
    Restore {
        manifest_key: &'a str,
    },
    /// Reads one part per disk from the object store; mounts no volumes.
    VerifyKey {
        manifest_key: &'a str,
    },
}

/// Job that proves an encryption key still opens a stored backup (see
/// `agent::run_verify_key`). It needs no PVCs, so `spec.disks` is ignored.
pub fn build_verify_key_job(
    job_name: &str,
    namespace: &str,
    manifest_key: &str,
    spec: &BackupJobSpec,
) -> Result<Job> {
    if manifest_key.is_empty() || manifest_key.starts_with('/') || manifest_key.contains("..") {
        bail!("invalid manifest key");
    }
    build_job(job_name, namespace, spec, Mode::VerifyKey { manifest_key })
}

pub fn build_backup_job(job_name: &str, namespace: &str, spec: &BackupJobSpec) -> Result<Job> {
    build_job(job_name, namespace, spec, Mode::Backup)
}

/// Job that reads a backup back onto freshly provisioned PVCs (`spec.disks`
/// are the *target* PVCs, mounted writable under `/restore/<name>/`).
pub fn build_restore_job(
    job_name: &str,
    namespace: &str,
    manifest_key: &str,
    spec: &BackupJobSpec,
) -> Result<Job> {
    if manifest_key.is_empty() || manifest_key.starts_with('/') || manifest_key.contains("..") {
        bail!("invalid manifest key");
    }
    build_job(job_name, namespace, spec, Mode::Restore { manifest_key })
}

fn build_job(job_name: &str, namespace: &str, spec: &BackupJobSpec, mode: Mode) -> Result<Job> {
    let (root, read_only) = match mode {
        Mode::Backup => (DISKS_ROOT, true),
        Mode::Restore { .. } | Mode::VerifyKey { .. } => (RESTORE_ROOT, false),
    };
    // A key check reads from the object store only: no volumes at all.
    let disks: &[BackupDisk] = match mode {
        Mode::VerifyKey { .. } => &[],
        _ => &spec.disks,
    };
    if disks.is_empty() && !matches!(mode, Mode::VerifyKey { .. }) {
        bail!("backup has no disks");
    }
    for d in disks {
        if !dns_label(&d.name) {
            bail!("invalid disk name '{}'", d.name);
        }
    }

    let disks_json = serde_json::to_string(
        &disks
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "pvc": d.pvc,
                    "block": d.block,
                    "path": if d.block {
                        device_path_for(mode, &d.name)
                    } else {
                        format!("{root}/{}/disk.img", d.name)
                    },
                })
            })
            .collect::<Vec<_>>(),
    )?;

    let mut env: Vec<EnvVar> = spec
        .env
        .iter()
        .map(|(k, v)| EnvVar {
            name: k.clone(),
            value: Some(v.clone()),
            ..Default::default()
        })
        .collect();
    for (k, v) in [
        ("BACKUP_ID", spec.backup_id.as_str()),
        ("BACKUP_VM", spec.vm_name.as_str()),
        ("BACKUP_NAMESPACE", namespace),
        ("BACKUP_SNAPSHOT", spec.snapshot_name.as_str()),
        (
            match mode {
                Mode::Backup => "BACKUP_DISKS",
                Mode::Restore { .. } | Mode::VerifyKey { .. } => "RESTORE_DISKS",
            },
            disks_json.as_str(),
        ),
    ] {
        env.push(EnvVar {
            name: k.to_string(),
            value: Some(v.to_string()),
            ..Default::default()
        });
    }
    if let Mode::Restore { manifest_key } | Mode::VerifyKey { manifest_key } = mode {
        let backup_mode = if matches!(mode, Mode::VerifyKey { .. }) {
            "verify-key"
        } else {
            "restore"
        };
        for (k, v) in [
            ("BACKUP_MODE", backup_mode),
            ("RESTORE_MANIFEST_KEY", manifest_key),
        ] {
            env.push(EnvVar {
                name: k.to_string(),
                value: Some(v.to_string()),
                ..Default::default()
            });
        }
    }
    env.push(secret_env(
        "AWS_ACCESS_KEY_ID",
        &spec.secrets.secret_name,
        &spec.secrets.access_key_key,
    ));
    env.push(secret_env(
        "AWS_SECRET_ACCESS_KEY",
        &spec.secrets.secret_name,
        &spec.secrets.secret_key_key,
    ));
    if let Some(k) = &spec.secrets.encryption_key_key {
        env.push(secret_env(
            "BACKUP_ENCRYPTION_KEY",
            &spec.secrets.secret_name,
            k,
        ));
    }

    // A raw block device is node-owned (root:disk, 0660), unreadable by the
    // unprivileged CDI UID. Only Jobs that attach a Block volume (a Block backup source
    // or a Block restore target) run the agent as root, still unprivileged with every
    // capability dropped; filesystem-only jobs keep UID 107.
    let uses_block = disks.iter().any(|d| d.block);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        match mode {
            Mode::Backup => "zorvia-backup",
            Mode::Restore { .. } => "zorvia-restore",
            Mode::VerifyKey { .. } => "zorvia-verify-key",
        }
        .to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "zorvia".to_string(),
    );
    labels.insert("zorvia.io/backup-id".to_string(), spec.backup_id.clone());

    let mut requests = BTreeMap::new();
    requests.insert("cpu".to_string(), Quantity("250m".into()));
    // A part (default 64 MiB) is held in memory, plus its ciphertext copy.
    requests.insert("memory".to_string(), Quantity("512Mi".into()));
    let mut limits = BTreeMap::new();
    limits.insert("memory".to_string(), Quantity("1Gi".into()));

    Ok(Job {
        metadata: ObjectMeta {
            name: Some(job_name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            // The agent aborts its multipart upload on failure and refuses to
            // overwrite an existing backup, so a retry starts clean.
            // A restore refuses to overwrite files, so a pod-level retry
            // could never succeed; the operation layer decides on retries.
            backoff_limit: Some(match mode {
                Mode::Backup => 2,
                Mode::Restore { .. } | Mode::VerifyKey { .. } => 0,
            }),
            ttl_seconds_after_finished: Some(3600),
            active_deadline_seconds: Some(spec.active_deadline_secs),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("Never".to_string()),
                    security_context: Some(PodSecurityContext {
                        run_as_user: Some(if uses_block { 0 } else { agent_uid() }),
                        run_as_non_root: Some(!uses_block),
                        fs_group: Some(agent_uid()),
                        ..Default::default()
                    }),
                    containers: vec![Container {
                        name: "backup-agent".to_string(),
                        image: Some(backup_agent_image()),
                        env: Some(env),
                        resources: Some(ResourceRequirements {
                            requests: Some(requests),
                            limits: Some(limits),
                            ..Default::default()
                        }),
                        security_context: Some(SecurityContext {
                            privileged: Some(false),
                            allow_privilege_escalation: Some(false),
                            read_only_root_filesystem: Some(true),
                            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        volume_devices: {
                            let devs: Vec<VolumeDevice> = spec
                                .disks
                                .iter()
                                .filter(|d| d.block)
                                .map(|d| VolumeDevice {
                                    name: format!("disk-{}", d.name),
                                    device_path: device_path_for(mode, &d.name),
                                })
                                .collect();
                            (!devs.is_empty()).then_some(devs)
                        },
                        volume_mounts: Some(
                            disks
                                .iter()
                                .filter(|d| !d.block)
                                .map(|d| VolumeMount {
                                    name: format!("disk-{}", d.name),
                                    mount_path: format!("{root}/{}", d.name),
                                    read_only: Some(read_only),
                                    ..Default::default()
                                })
                                .collect(),
                        ),
                        ..Default::default()
                    }],
                    volumes: Some(
                        disks
                            .iter()
                            .map(|d| Volume {
                                name: format!("disk-{}", d.name),
                                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                                    claim_name: d.pvc.clone(),
                                    read_only: Some(read_only),
                                }),
                                ..Default::default()
                            })
                            .collect(),
                    ),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> BackupJobSpec {
        BackupJobSpec {
            backup_id: "bk-1".into(),
            vm_name: "web-1".into(),
            snapshot_name: "backup-web-1-abc".into(),
            disks: vec![
                BackupDisk {
                    name: "root".into(),
                    pvc: "bk-1-root".into(),
                    block: false,
                },
                BackupDisk {
                    name: "data".into(),
                    pvc: "bk-1-data".into(),
                    block: false,
                },
            ],
            env: vec![("BACKUP_S3_BUCKET".into(), "vm-backups".into())],
            secrets: BackupSecretRef {
                secret_name: "backup-s3".into(),
                access_key_key: "access-key".into(),
                secret_key_key: "secret-key".into(),
                encryption_key_key: Some("enc-key".into()),
            },
            active_deadline_secs: 21600,
        }
    }

    fn pod(job: &Job) -> &PodSpec {
        job.spec.as_ref().unwrap().template.spec.as_ref().unwrap()
    }

    #[test]
    fn mounts_every_disk_read_only_under_disks_root() {
        let job = build_backup_job("bk-1", "default", &spec()).unwrap();
        let c = &pod(&job).containers[0];
        let mounts = c.volume_mounts.as_ref().unwrap();
        assert_eq!(mounts.len(), 2);
        assert!(mounts.iter().all(|m| m.read_only == Some(true)));
        assert_eq!(mounts[0].mount_path, "/disks/root");
        let vols = pod(&job).volumes.as_ref().unwrap();
        assert_eq!(
            vols[1].persistent_volume_claim.as_ref().unwrap().claim_name,
            "bk-1-data"
        );
        assert_eq!(
            vols[1].persistent_volume_claim.as_ref().unwrap().read_only,
            Some(true)
        );
    }

    #[test]
    fn credentials_come_from_the_secret_never_as_values() {
        let job = build_backup_job("bk-1", "default", &spec()).unwrap();
        let env = pod(&job).containers[0].env.as_ref().unwrap();
        for name in [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "BACKUP_ENCRYPTION_KEY",
        ] {
            let e = env.iter().find(|e| e.name == name).unwrap();
            assert!(e.value.is_none(), "{name} must not be a literal value");
            assert_eq!(
                e.value_from
                    .as_ref()
                    .unwrap()
                    .secret_key_ref
                    .as_ref()
                    .unwrap()
                    .name,
                "backup-s3"
            );
        }
        let disks = env.iter().find(|e| e.name == "BACKUP_DISKS").unwrap();
        assert!(disks
            .value
            .as_ref()
            .unwrap()
            .contains("/disks/root/disk.img"));
    }

    #[test]
    fn container_is_locked_down() {
        let job = build_backup_job("bk-1", "default", &spec()).unwrap();
        let sc = pod(&job).containers[0].security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(false));
        assert_eq!(sc.allow_privilege_escalation, Some(false));
        assert_eq!(sc.read_only_root_filesystem, Some(true));
        assert_eq!(
            sc.capabilities.as_ref().unwrap().drop,
            Some(vec!["ALL".to_string()])
        );
        assert_eq!(
            pod(&job).security_context.as_ref().unwrap().run_as_non_root,
            Some(true)
        );
    }

    #[test]
    fn omits_encryption_env_when_not_encrypting() {
        let mut s = spec();
        s.secrets.encryption_key_key = None;
        let job = build_backup_job("bk-1", "default", &s).unwrap();
        let env = pod(&job).containers[0].env.as_ref().unwrap();
        assert!(!env.iter().any(|e| e.name == "BACKUP_ENCRYPTION_KEY"));
    }

    #[test]
    fn restore_job_mounts_targets_writable_and_selects_restore_mode() {
        let job =
            build_restore_job("rs-1", "default", "web-1/op-1/manifest.json", &spec()).unwrap();
        let c = &pod(&job).containers[0];
        let mounts = c.volume_mounts.as_ref().unwrap();
        assert_eq!(mounts[0].mount_path, "/restore/root");
        assert!(mounts.iter().all(|m| m.read_only == Some(false)));
        let vols = pod(&job).volumes.as_ref().unwrap();
        assert_eq!(
            vols[0].persistent_volume_claim.as_ref().unwrap().read_only,
            Some(false)
        );
        let env = c.env.as_ref().unwrap();
        let val = |n: &str| {
            env.iter()
                .find(|e| e.name == n)
                .and_then(|e| e.value.clone())
        };
        assert_eq!(val("BACKUP_MODE").as_deref(), Some("restore"));
        assert_eq!(
            val("RESTORE_MANIFEST_KEY").as_deref(),
            Some("web-1/op-1/manifest.json")
        );
        assert!(val("RESTORE_DISKS")
            .unwrap()
            .contains("/restore/root/disk.img"));
        assert!(val("BACKUP_DISKS").is_none());
        assert_eq!(job.spec.as_ref().unwrap().backoff_limit, Some(0));
        // Still no privileges, and credentials still come from the Secret.
        let sc = c.security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(false));
        assert!(env
            .iter()
            .find(|e| e.name == "AWS_SECRET_ACCESS_KEY")
            .unwrap()
            .value
            .is_none());
    }

    #[test]
    fn restore_job_rejects_bad_manifest_keys() {
        for k in ["", "/abs", "a/../b"] {
            assert!(build_restore_job("rs-1", "default", k, &spec()).is_err());
        }
    }

    #[test]
    fn block_source_is_a_device_not_a_mount_and_runs_as_root_unprivileged() {
        let mut s = spec();
        s.disks[1].block = true; // "data" is Block, "root" stays Filesystem
        let job = build_backup_job("bk-1", "default", &s).unwrap();
        let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        let c = &pod.containers[0];
        let devs = c.volume_devices.as_ref().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].device_path, "/dev/zorvia-disk-data");
        assert_eq!(devs[0].name, "disk-data");
        let mounts = c.volume_mounts.as_ref().unwrap();
        assert!(mounts.iter().all(|m| m.name != "disk-data"));
        assert!(mounts.iter().any(|m| m.name == "disk-root"));
        let env = c.env.as_ref().unwrap();
        let disks = env
            .iter()
            .find(|e| e.name == "BACKUP_DISKS")
            .unwrap()
            .value
            .clone()
            .unwrap();
        assert!(disks.contains("/dev/zorvia-disk-data"));
        assert!(disks.contains("/disks/root/disk.img"));
        let psc = pod.security_context.as_ref().unwrap();
        assert_eq!(psc.run_as_user, Some(0));
        assert_eq!(psc.run_as_non_root, Some(false));
        let sc = c.security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(false));
        assert_eq!(sc.allow_privilege_escalation, Some(false));
        assert_eq!(
            sc.capabilities.as_ref().unwrap().drop.as_ref().unwrap(),
            &vec!["ALL".to_string()]
        );
    }

    #[test]
    fn filesystem_only_jobs_keep_the_unprivileged_cdi_uid() {
        let job = build_backup_job("bk-1", "default", &spec()).unwrap();
        let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        assert!(pod.containers[0].volume_devices.is_none());
        let psc = pod.security_context.as_ref().unwrap();
        assert_eq!(psc.run_as_user, Some(107));
        assert_eq!(psc.run_as_non_root, Some(true));
    }

    #[test]
    fn block_restore_target_is_a_writable_device_and_runs_as_root_unprivileged() {
        let mut s = spec();
        s.disks[1].block = true; // "data" restores into a Block volume, "root" into a file
        let job = build_restore_job("rs-1", "default", "web-1/op-1/manifest.json", &s).unwrap();
        let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        let c = &pod.containers[0];
        let devs = c.volume_devices.as_ref().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].device_path, "/dev/zorvia-restore-data");
        let mounts = c.volume_mounts.as_ref().unwrap();
        assert!(mounts.iter().all(|m| m.name != "disk-data"));
        let root = mounts.iter().find(|m| m.name == "disk-root").unwrap();
        assert_eq!(root.read_only, Some(false), "restore mounts are writable");
        let env = c.env.as_ref().unwrap();
        let disks = env
            .iter()
            .find(|e| e.name == "RESTORE_DISKS")
            .unwrap()
            .value
            .clone()
            .unwrap();
        assert!(disks.contains("\"block\":true") && disks.contains("/dev/zorvia-restore-data"));
        assert!(disks.contains("/restore/root/disk.img"));
        let psc = pod.security_context.as_ref().unwrap();
        assert_eq!(
            (psc.run_as_user, psc.run_as_non_root),
            (Some(0), Some(false))
        );
        let sc = c.security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(false));
        assert_eq!(sc.allow_privilege_escalation, Some(false));
    }

    #[test]
    fn verify_key_job_mounts_nothing_and_runs_in_verify_mode() {
        let mut s = spec();
        s.disks.clear();
        let job = build_verify_key_job("vk-1", "default", "web-1/op-1/manifest.json", &s).unwrap();
        let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        let c = &pod.containers[0];
        assert!(pod.volumes.as_ref().is_none_or(|v| v.is_empty()));
        assert!(c.volume_mounts.as_ref().is_none_or(|v| v.is_empty()));
        assert!(c.volume_devices.is_none());
        let env = c.env.as_ref().unwrap();
        let val = |n: &str| {
            env.iter()
                .find(|e| e.name == n)
                .and_then(|e| e.value.clone())
        };
        assert_eq!(val("BACKUP_MODE").as_deref(), Some("verify-key"));
        assert_eq!(
            val("RESTORE_MANIFEST_KEY").as_deref(),
            Some("web-1/op-1/manifest.json")
        );
        // Credentials and the key still come from the Secret, never as values.
        for n in ["AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID"] {
            assert!(env.iter().find(|e| e.name == n).unwrap().value.is_none());
        }
        // Unprivileged CDI UID, no Block-source root escalation.
        let psc = pod.security_context.as_ref().unwrap();
        assert_eq!(psc.run_as_user, Some(107));
        assert_eq!(psc.run_as_non_root, Some(true));
        assert_eq!(job.spec.as_ref().unwrap().backoff_limit, Some(0));
        for k in ["", "/abs", "a/../b"] {
            assert!(build_verify_key_job("vk-1", "default", k, &s).is_err());
        }
    }

    #[test]
    fn rejects_bad_disks() {
        let mut s = spec();
        s.disks[0].name = "Bad/Name".into();
        assert!(build_backup_job("bk-1", "default", &s).is_err());
        s.disks.clear();
        assert!(build_backup_job("bk-1", "default", &s).is_err());
    }
}
