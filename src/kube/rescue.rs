// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Rescue mode: resolves a stopped VM's own PVC and builds the Kubernetes
//! Job that mounts it and runs `rescue-agent` (GuestKit) against it. Zorvia
//! owns this Job directly -- it does not integrate with GuestKit's own
//! fleet job-routing protocol (`guestkit-job-spec`/`guestkit-worker`),
//! which is a distributed system in its own right. See docs/RESCUE.md.

use super::types::VirtualMachine;
use anyhow::{anyhow, Result};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PersistentVolumeClaimVolumeSource, PodSpec, PodTemplateSpec, Secret,
    SecretVolumeSource, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

#[path = "rescue_logic.rs"]
pub mod logic;

/// Where the reset-password Secret is mounted in the Job container.
const SECRET_MOUNT_PATH: &str = "/run/rescue-secret";
/// Key (and file name) of the password inside the Secret.
pub const SECRET_PASSWORD_KEY: &str = "password";

/// A password in flight. `Debug` is redacted and there is no `Display` or
/// `Serialize`, so it cannot reach logs or a serialized spec by accident.
#[derive(Clone)]
pub struct RescuePassword(String);

impl RescuePassword {
    pub fn new(password: String) -> Self {
        Self(password)
    }
    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for RescuePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RescuePassword([redacted])")
    }
}

/// The rescue-agent image, overridable so a Helm/deploy value can point at
/// a private registry mirror without a code change.
pub fn rescue_agent_image() -> String {
    std::env::var("ZORVIA_RESCUE_AGENT_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/zyvorai/zorvia-rescue-agent:latest".to_string())
}

/// The operations rescue mode supports. `enable-ssh` takes none -- it always
/// targets ssh.service/sshd.service, never a client-supplied unit name
/// (fixed-argv precedent, same as pod exec). `ResetPassword` is the only
/// variant carrying a secret: it travels in a Kubernetes Secret, never in the
/// Job spec. `InstallPackages` names are pre-validated by `logic`.
#[derive(Debug, Clone)]
pub enum RescueOperation {
    SetHostname {
        hostname: String,
    },
    InjectSshKey {
        user: String,
        key: String,
    },
    EnableSsh,
    ResetPassword {
        user: String,
        password: RescuePassword,
    },
    InstallPackages {
        packages: Vec<String>,
    },
    Inspect,
}

impl RescueOperation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetHostname { .. } => "set-hostname",
            Self::InjectSshKey { .. } => "inject-ssh-key",
            Self::EnableSsh => "enable-ssh",
            Self::ResetPassword { .. } => "reset-password",
            Self::InstallPackages { .. } => "install-packages",
            Self::Inspect => "inspect",
        }
    }

    /// True for the operations that are Experimental-gated (the original
    /// three stay Beta).
    pub fn is_experimental(&self) -> bool {
        matches!(
            self,
            Self::ResetPassword { .. } | Self::InstallPackages { .. } | Self::Inspect
        )
    }

    /// The password to place in the per-Job Secret, if any.
    pub fn secret_password(&self) -> Option<&str> {
        match self {
            Self::ResetPassword { password, .. } => Some(password.expose()),
            _ => None,
        }
    }

    fn env_vars(&self) -> Vec<(String, String)> {
        let mut env = vec![("RESCUE_OPERATION".to_string(), self.name().to_string())];
        match self {
            Self::SetHostname { hostname } => {
                env.push(("RESCUE_HOSTNAME".to_string(), hostname.clone()));
            }
            Self::InjectSshKey { user, key } => {
                env.push(("RESCUE_SSH_USER".to_string(), user.clone()));
                env.push(("RESCUE_SSH_KEY".to_string(), key.clone()));
            }
            Self::EnableSsh | Self::Inspect => {}
            Self::ResetPassword { user, .. } => {
                env.push(("RESCUE_USER".to_string(), user.clone()));
                // A path, not the password: the value is mounted from the
                // per-Job Secret.
                env.push((
                    "RESCUE_PASSWORD_FILE".to_string(),
                    format!("{SECRET_MOUNT_PATH}/{SECRET_PASSWORD_KEY}"),
                ));
            }
            Self::InstallPackages { packages } => {
                env.push(("RESCUE_PACKAGES".to_string(), packages.join(" ")));
            }
        }
        env
    }
}

/// Name of the per-Job Secret carrying a reset-password password.
pub fn rescue_secret_name(job_name: &str) -> String {
    format!("{job_name}-secret")
}

fn rescue_labels() -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "zorvia-rescue".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "zorvia".to_string(),
    );
    labels
}

/// The Secret holding the password for one rescue Job. Created before the
/// Job, deleted when the Job finishes/is deleted (and garbage-collected via
/// an ownerReference added after the Job exists, see
/// [`secret_owner_patch`]).
pub fn build_rescue_secret(job_name: &str, namespace: &str, password: &str) -> Secret {
    let mut data = BTreeMap::new();
    data.insert(SECRET_PASSWORD_KEY.to_string(), password.to_string());
    Secret {
        metadata: ObjectMeta {
            name: Some(rescue_secret_name(job_name)),
            namespace: Some(namespace.to_string()),
            labels: Some(rescue_labels()),
            ..Default::default()
        },
        type_: Some("Opaque".to_string()),
        string_data: Some(data),
        ..Default::default()
    }
}

/// Merge patch making the Job the Secret's owner so Kubernetes garbage
/// collection removes the Secret whenever the Job goes away (TTL expiry
/// included) even if Zorvia never gets to delete it explicitly.
pub fn secret_owner_patch(job_name: &str, job_uid: &str) -> serde_json::Value {
    serde_json::json!({
        "metadata": { "ownerReferences": [{
            "apiVersion": "batch/v1",
            "kind": "Job",
            "name": job_name,
            "uid": job_uid,
            "blockOwnerDeletion": false,
        }]}
    })
}

/// Finds the PVC (or DataVolume, which realizes as a same-named PVC) backing
/// one of the VM's volumes. Same resolution logic already used read-only in
/// `storage_handlers.rs` for the PVC->VM reverse index -- `disk_name: None`
/// takes the first PVC/DataVolume-backed volume (the common single-disk
/// case); `Some(name)` requires an exact volume-name match for multi-disk
/// VMs.
pub fn resolve_vm_disk_pvc(vm: &VirtualMachine, disk_name: Option<&str>) -> Result<String> {
    let volumes = vm
        .spec
        .template
        .spec
        .volumes
        .as_ref()
        .ok_or_else(|| anyhow!("VM has no volumes"))?;

    for vol in volumes {
        if let Some(disk_name) = disk_name {
            if vol.name != disk_name {
                continue;
            }
        }
        let claim_name = vol
            .persistent_volume_claim
            .as_ref()
            .map(|p| p.claim_name.clone())
            .or_else(|| vol.data_volume.as_ref().map(|dv| dv.name.clone()));
        if let Some(claim_name) = claim_name {
            return Ok(claim_name);
        }
    }

    Err(anyhow!(
        "VM has no PVC- or DataVolume-backed disk to rescue \
         (emptyDisk/containerDisk volumes have nothing to mount)"
    ))
}

/// Builds the Job spec. Security context starts from GuestKit's own worker
/// DaemonSet template (`SYS_ADMIN`+`SYS_RESOURCE`) rather than full
/// `privileged: true` -- narrow this further only if it proves
/// insufficient on a real cluster (GuestKit's disk-mount path shells out to
/// `losetup`/`qemu-nbd`/`mount`/`chroot`, none of which need `/dev/kvm` or
/// `NET_ADMIN`).
pub fn build_rescue_job(
    job_name: &str,
    namespace: &str,
    pvc_name: &str,
    operation: &RescueOperation,
) -> Job {
    const DISK_MOUNT_PATH: &str = "/disk";
    // Verified on-cluster before relying on this for a real VM: the exact
    // file within the mounted PVC that holds the raw/qcow2 disk image
    // depends on the CSI driver and whether KubeVirt provisioned it via a
    // DataVolume or a raw PVC. Not assumed here -- see docs/RESCUE.md.
    let disk_path = format!("{DISK_MOUNT_PATH}/disk.img");

    let mut env: Vec<EnvVar> = operation
        .env_vars()
        .into_iter()
        .map(|(name, value)| EnvVar {
            name,
            value: Some(value),
            ..Default::default()
        })
        .collect();
    env.push(EnvVar {
        name: "RESCUE_DISK_PATH".to_string(),
        value: Some(disk_path),
        ..Default::default()
    });

    let labels = rescue_labels();

    let needs_secret = operation.secret_password().is_some();
    let mut volumes = vec![Volume {
        name: "rescue-disk".to_string(),
        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
            claim_name: pvc_name.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }];
    let mut mounts = vec![VolumeMount {
        name: "rescue-disk".to_string(),
        mount_path: DISK_MOUNT_PATH.to_string(),
        ..Default::default()
    }];
    if needs_secret {
        volumes.push(Volume {
            name: "rescue-secret".to_string(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(rescue_secret_name(job_name)),
                default_mode: Some(0o400),
                ..Default::default()
            }),
            ..Default::default()
        });
        mounts.push(VolumeMount {
            name: "rescue-secret".to_string(),
            mount_path: SECRET_MOUNT_PATH.to_string(),
            read_only: Some(true),
            ..Default::default()
        });
    }

    Job {
        metadata: ObjectMeta {
            name: Some(job_name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(0),
            ttl_seconds_after_finished: Some(3600),
            active_deadline_seconds: Some(match operation {
                // Package downloads + postinst scripts need more headroom.
                RescueOperation::InstallPackages { .. } => 900,
                _ => 300,
            }),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    restart_policy: Some("Never".to_string()),
                    containers: vec![Container {
                        name: "rescue-agent".to_string(),
                        image: Some(rescue_agent_image()),
                        env: Some(env),
                        security_context: Some(SecurityContext {
                            privileged: Some(false),
                            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                                add: Some(vec![
                                    "SYS_ADMIN".to_string(),
                                    "SYS_RESOURCE".to_string(),
                                ]),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        volume_mounts: Some(mounts),
                        ..Default::default()
                    }],
                    volumes: Some(volumes),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(op: &RescueOperation) -> Job {
        build_rescue_job("zorvia-rescue-vm-abc", "ns", "vm-pvc", op)
    }

    fn envs(job: &Job) -> BTreeMap<String, String> {
        job.spec
            .as_ref()
            .unwrap()
            .template
            .spec
            .as_ref()
            .unwrap()
            .containers[0]
            .env
            .clone()
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.value.unwrap_or_default()))
            .collect()
    }

    #[test]
    fn reset_password_job_never_contains_the_password() {
        let pw = "S3cr3t-Pa55 w0rd!";
        let op = RescueOperation::ResetPassword {
            user: "root".into(),
            password: RescuePassword::new(pw.into()),
        };
        let j = job(&op);
        let json = serde_json::to_string(&j).unwrap();
        assert!(!json.contains(pw), "password leaked into Job spec");
        assert!(!format!("{op:?}").contains(pw), "password leaked via Debug");
        assert!(!format!("{j:?}").contains(pw));
        let e = envs(&j);
        assert_eq!(e["RESCUE_OPERATION"], "reset-password");
        assert_eq!(e["RESCUE_USER"], "root");
        assert_eq!(e["RESCUE_PASSWORD_FILE"], "/run/rescue-secret/password");

        // Secret volume: 0400, read-only mount, per-Job secret name.
        let pod = j.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        let vol = pod
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "rescue-secret")
            .unwrap();
        let sv = vol.secret.as_ref().unwrap();
        assert_eq!(
            sv.secret_name.as_deref(),
            Some("zorvia-rescue-vm-abc-secret")
        );
        assert_eq!(sv.default_mode, Some(0o400));
        let m = pod.containers[0]
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .find(|m| m.name == "rescue-secret")
            .unwrap();
        assert_eq!(m.read_only, Some(true));
    }

    #[test]
    fn secret_holds_password_and_is_job_scoped() {
        let s = build_rescue_secret("zorvia-rescue-vm-abc", "ns", "pw-123");
        assert_eq!(
            s.metadata.name.as_deref(),
            Some("zorvia-rescue-vm-abc-secret")
        );
        assert_eq!(
            s.string_data.as_ref().unwrap()[SECRET_PASSWORD_KEY],
            "pw-123"
        );
        let patch = secret_owner_patch("zorvia-rescue-vm-abc", "uid-1");
        let o = &patch["metadata"]["ownerReferences"][0];
        assert_eq!(o["kind"], "Job");
        assert_eq!(o["uid"], "uid-1");
        assert!(!patch.to_string().contains("pw-123"));
    }

    #[test]
    fn other_operations_have_no_secret_volume() {
        for op in [
            RescueOperation::Inspect,
            RescueOperation::EnableSsh,
            RescueOperation::InstallPackages {
                packages: vec!["curl".into(), "git".into()],
            },
        ] {
            assert!(op.secret_password().is_none());
            let j = job(&op);
            let pod = j.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
            assert_eq!(pod.volumes.as_ref().unwrap().len(), 1);
        }
        let e = envs(&job(&RescueOperation::InstallPackages {
            packages: vec!["curl".into(), "git".into()],
        }));
        assert_eq!(e["RESCUE_OPERATION"], "install-packages");
        assert_eq!(e["RESCUE_PACKAGES"], "curl git");
        assert_eq!(
            envs(&job(&RescueOperation::Inspect))["RESCUE_OPERATION"],
            "inspect"
        );
    }

    #[test]
    fn job_is_bounded_and_never_retried() {
        let j = job(&RescueOperation::Inspect);
        let spec = j.spec.unwrap();
        assert_eq!(spec.backoff_limit, Some(0));
        assert_eq!(spec.active_deadline_seconds, Some(300));
    }

    #[test]
    fn experimental_flag_marks_only_new_ops() {
        assert!(!RescueOperation::EnableSsh.is_experimental());
        assert!(!RescueOperation::SetHostname {
            hostname: "h".into()
        }
        .is_experimental());
        assert!(RescueOperation::Inspect.is_experimental());
    }
}
