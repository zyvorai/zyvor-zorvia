//! Kubernetes Job that runs `h2kvmctl` for one VM import.
//!
//! h2kvm converts disks with `qemu-nbd`/`losetup`, which need real host
//! privilege, so the container is privileged and an init container loads the
//! `nbd` kernel module (same shape as h2kvm's own `k8s/cli/job-template.yaml`).
//! That is a large grant: the routes that create these Jobs are cluster.admin
//! only, and the namespace must permit privileged pods. vCenter credentials
//! come from a Secret via `secretKeyRef`; scratch space is a generic ephemeral
//! volume, deleted with the pod.

use super::{h2kvm_args, ImportMode, ImportOpParams};
use anyhow::{anyhow, Result};
use k8s_openapi::api::batch::v1::Job;
use serde_json::{json, Value};

/// The h2kvm CLI image. There is deliberately no default: h2kvm is licensed
/// separately and its registry location is the operator's choice.
pub fn h2kvm_image() -> Option<String> {
    std::env::var("ZORVIA_H2KVM_IMAGE")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Image for stub/local lab Jobs. Prefers `ZORVIA_H2KVM_IMAGE`, else busybox.
pub fn stub_image() -> String {
    h2kvm_image().unwrap_or_else(|| "busybox:1.36".into())
}

fn init_image() -> String {
    std::env::var("ZORVIA_H2KVM_INIT_IMAGE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "fedora:43".to_string())
}

fn deadline_secs() -> i64 {
    std::env::var("ZORVIA_IMPORT_JOB_DEADLINE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12 * 3600)
}

pub fn build_import_job(job_name: &str, image: &str, p: &ImportOpParams) -> Result<Job> {
    if p.mode.is_lab_stub() {
        return build_stub_import_job(job_name, image, p);
    }
    let user_key = p.source.username_key.as_deref().unwrap_or("username");
    let pass_key = p.source.password_key.as_deref().unwrap_or("password");

    let mut scratch_spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": {"requests": {"storage": p.scratch_size}},
    });
    if let Some(sc) = &p.scratch_storage_class {
        scratch_spec["storageClassName"] = json!(sc);
    }

    let labels: Value = json!({
        "app.kubernetes.io/name": "zorvia-import",
        "app.kubernetes.io/managed-by": "zorvia",
        "zorvia.io/wave": p.wave_id,
        "zorvia.io/target-vm": p.target_vm_name,
        "zorvia.io/import-mode": "vsphere",
    });

    let job = json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": job_name, "namespace": p.namespace, "labels": labels},
        "spec": {
            // An import is not safely repeatable (partial uploads, half-created
            // VMs); the operation layer reports the failure and cleans up.
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": 3600,
            "activeDeadlineSeconds": deadline_secs(),
            "template": {
                "metadata": {"labels": labels},
                "spec": {
                    "restartPolicy": "Never",
                    "serviceAccountName": p.service_account,
                    "initContainers": [{
                        "name": "nbd-loader",
                        "image": init_image(),
                        "command": ["modprobe", "nbd", "max_part=16", "nbds_max=16"],
                        "securityContext": {"privileged": true},
                        "volumeMounts": [{"name": "lib-modules", "mountPath": "/lib/modules", "readOnly": true}],
                    }],
                    "containers": [{
                        "name": "h2kvm",
                        "image": image,
                        "imagePullPolicy": "IfNotPresent",
                        "command": ["h2kvmctl"],
                        "args": h2kvm_args(p),
                        "env": [
                            {"name": "PYTHONUNBUFFERED", "value": "1"},
                            {"name": "H2KVM_MODE", "value": "cli"},
                            {"name": "VC_USER", "valueFrom": {"secretKeyRef": {
                                "name": p.source.secret_name, "key": user_key, "optional": false}}},
                            {"name": "VC_PASSWORD", "valueFrom": {"secretKeyRef": {
                                "name": p.source.secret_name, "key": pass_key, "optional": false}}},
                        ],
                        "securityContext": {"privileged": true},
                        "volumeMounts": [
                            {"name": "work", "mountPath": "/work"},
                            {"name": "dev", "mountPath": "/dev", "mountPropagation": "HostToContainer"},
                        ],
                        "resources": {
                            "requests": {"cpu": "2", "memory": "4Gi"},
                            "limits": {"memory": "16Gi"},
                        },
                    }],
                    "volumes": [
                        {"name": "lib-modules", "hostPath": {"path": "/lib/modules"}},
                        {"name": "dev", "hostPath": {"path": "/dev"}},
                        {"name": "work", "ephemeral": {"volumeClaimTemplate": {"spec": scratch_spec}}},
                    ],
                },
            },
        },
    });
    serde_json::from_value(job).map_err(|e| anyhow!("invalid import Job: {e}"))
}

/// Unprivileged Job that emits h2kvm-shaped stage logs so follow/cancel work
/// without vCenter or a licensed h2kvm image.
fn build_stub_import_job(job_name: &str, image: &str, p: &ImportOpParams) -> Result<Job> {
    let mode = match p.mode {
        ImportMode::Local => "local",
        _ => "stub",
    };
    let labels: Value = json!({
        "app.kubernetes.io/name": "zorvia-import",
        "app.kubernetes.io/managed-by": "zorvia",
        "zorvia.io/wave": p.wave_id,
        "zorvia.io/target-vm": p.target_vm_name,
        "zorvia.io/import-mode": mode,
    });
    let target = &p.target_vm_name;
    let script = format!(
        r#"set -e
TARGET="{target}"
echo "➡️  Stage: INSPECT"
echo "✅ inspect completed in 0.10s"
echo "➡️  Stage: EXPORT"
echo "✅ export completed in 0.10s"
echo "➡️  Stage: CONVERT"
echo "✅ convert completed in 0.10s"
echo "➡️  Stage: DEPLOY"
echo "✅ deploy completed in 0.10s"
echo "✅ Deployed: $TARGET"
"#
    );
    let job = json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": job_name, "namespace": p.namespace, "labels": labels},
        "spec": {
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": 600,
            "activeDeadlineSeconds": 600,
            "template": {
                "metadata": {"labels": labels},
                "spec": {
                    "restartPolicy": "Never",
                    "serviceAccountName": p.service_account,
                    "containers": [{
                        "name": "h2kvm-stub",
                        "image": image,
                        "imagePullPolicy": "IfNotPresent",
                        "command": ["sh", "-c", script],
                        "resources": {
                            "requests": {"cpu": "50m", "memory": "32Mi"},
                            "limits": {"memory": "128Mi"},
                        },
                    }],
                },
            },
        },
    });
    serde_json::from_value(job).map_err(|e| anyhow!("invalid stub import Job: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_import::{ImportMode, ImportVm, VsphereSource};

    fn params() -> ImportOpParams {
        ImportOpParams {
            wave_id: "wave-1".into(),
            source: VsphereSource {
                vcenter: "vc.example.com".into(),
                port: None,
                datacenter: None,
                insecure: false,
                secret_name: "vcenter-creds".into(),
                username_key: None,
                password_key: Some("pw".into()),
            },
            namespace: "vms".into(),
            vm: ImportVm {
                source_vm: "web-01".into(),
                target_vm_name: None,
                storage_class: None,
                pvc_size: None,
                cpu: None,
                memory: None,
                start: false,
                networks: Vec::new(),
                require_guest_agent: false,
                boot_timeout_secs: None,
            },
            target_vm_name: "web-01".into(),
            scratch_size: "300Gi".into(),
            scratch_storage_class: Some("fast".into()),
            service_account: "zorvia-h2kvm".into(),
            mode: ImportMode::Vsphere,
        }
    }

    #[test]
    fn stub_job_is_unprivileged_and_has_no_secrets() {
        let mut p = params();
        p.mode = ImportMode::Stub;
        let job = build_import_job("zorvia-import-stub", "busybox:1.36", &p).unwrap();
        let spec = pod(&job);
        assert!(spec.init_containers.as_ref().map(|c| c.is_empty()).unwrap_or(true));
        assert_eq!(spec.containers[0].name, "h2kvm-stub");
        assert!(spec.containers[0]
            .env
            .as_ref()
            .map(|e| e.is_empty())
            .unwrap_or(true));
        let sc = spec.containers[0].security_context.as_ref();
        assert!(sc.map(|c| c.privileged != Some(true)).unwrap_or(true));
    }

    fn pod(j: &Job) -> &k8s_openapi::api::core::v1::PodSpec {
        j.spec.as_ref().unwrap().template.spec.as_ref().unwrap()
    }

    #[test]
    fn credentials_come_from_the_secret_never_as_values() {
        let job = build_import_job("zorvia-import-x", "reg/h2kvm-cli:1.4.0", &params()).unwrap();
        let env = pod(&job).containers[0].env.as_ref().unwrap();
        for (name, key) in [("VC_USER", "username"), ("VC_PASSWORD", "pw")] {
            let e = env.iter().find(|e| e.name == name).unwrap();
            assert!(e.value.is_none(), "{name} must not be a literal");
            let r = e
                .value_from
                .as_ref()
                .unwrap()
                .secret_key_ref
                .as_ref()
                .unwrap();
            assert_eq!((r.name.as_str(), r.key.as_str()), ("vcenter-creds", key));
        }
        // The user name is passed by env expansion, the password by env name.
        let args = pod(&job).containers[0].args.as_ref().unwrap();
        assert!(args.contains(&"$(VC_USER)".to_string()));
        assert!(args.contains(&"VC_PASSWORD".to_string()));
    }

    #[test]
    fn runs_privileged_with_nbd_loader_and_ephemeral_scratch() {
        let job = build_import_job("j", "img", &params()).unwrap();
        let p = pod(&job);
        assert_eq!(p.service_account_name.as_deref(), Some("zorvia-h2kvm"));
        assert_eq!(p.restart_policy.as_deref(), Some("Never"));
        let init = &p.init_containers.as_ref().unwrap()[0];
        assert_eq!(init.command.as_ref().unwrap()[0], "modprobe");
        assert_eq!(
            p.containers[0]
                .security_context
                .as_ref()
                .unwrap()
                .privileged,
            Some(true)
        );
        let work = p
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "work")
            .unwrap();
        let tpl = &work
            .ephemeral
            .as_ref()
            .unwrap()
            .volume_claim_template
            .as_ref()
            .unwrap()
            .spec;
        assert_eq!(tpl.storage_class_name.as_deref(), Some("fast"));
        assert_eq!(
            tpl.resources.as_ref().unwrap().requests.as_ref().unwrap()["storage"].0,
            "300Gi"
        );
        let spec = job.spec.as_ref().unwrap();
        assert_eq!(spec.backoff_limit, Some(0));
        assert_eq!(spec.ttl_seconds_after_finished, Some(3600));
    }

    #[test]
    fn labels_identify_wave_and_target() {
        let job = build_import_job("j", "img", &params()).unwrap();
        let l = job.metadata.labels.as_ref().unwrap();
        assert_eq!(l["zorvia.io/wave"], "wave-1");
        assert_eq!(l["zorvia.io/target-vm"], "web-01");
        assert_eq!(l["app.kubernetes.io/managed-by"], "zorvia");
    }
}
