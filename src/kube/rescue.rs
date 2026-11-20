// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Rescue mode: resolves a stopped VM's own PVC and builds the Kubernetes
//! Job that mounts it and runs `rescue-agent` (GuestKit) against it. Zorvia
//! owns this Job directly -- it does not integrate with GuestKit's own
//! fleet job-routing protocol (`guestkit-job-spec`/`guestkit-worker`),
//! which is a distributed system in its own right. See docs/RESCUE.md.

use super::types::VirtualMachine;
use anyhow::{anyhow, Result};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PersistentVolumeClaimVolumeSource, PodSpec, PodTemplateSpec,
    SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

/// The rescue-agent image, overridable so a Helm/deploy value can point at
/// a private registry mirror without a code change.
pub fn rescue_agent_image() -> String {
    std::env::var("ZORVIA_RESCUE_AGENT_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/zyvorai/zorvia-rescue-agent:latest".to_string())
}

/// The one operation-specific parameters this phase supports. `enable-ssh`
/// takes none -- it always targets ssh.service/sshd.service, never a
/// client-supplied unit name (fixed-argv precedent, same as pod exec).
#[derive(Debug, Clone)]
pub enum RescueOperation {
    SetHostname { hostname: String },
    InjectSshKey { user: String, key: String },
    EnableSsh,
}

impl RescueOperation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetHostname { .. } => "set-hostname",
            Self::InjectSshKey { .. } => "inject-ssh-key",
            Self::EnableSsh => "enable-ssh",
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
            Self::EnableSsh => {}
        }
        env
    }
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

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "zorvia-rescue".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "zorvia".to_string(),
    );

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
            active_deadline_seconds: Some(300),
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
                        volume_mounts: Some(vec![VolumeMount {
                            name: "rescue-disk".to_string(),
                            mount_path: DISK_MOUNT_PATH.to_string(),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    volumes: Some(vec![Volume {
                        name: "rescue-disk".to_string(),
                        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                            claim_name: pvc_name.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        status: None,
    }
}
