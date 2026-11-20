// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! CDI DataVolume helpers for data-preserving PVC clones and image imports.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::kube::lifecycle::validate_k8s_name;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CdiPvcCloneSpec {
    pub source_namespace: String,
    pub source_pvc: String,
    pub target_namespace: String,
    pub target_name: String,
    pub size: String,
    pub storage_class: Option<String>,
}

impl CdiPvcCloneSpec {
    pub fn validate(&self) -> Result<()> {
        validate_k8s_name("source namespace", &self.source_namespace)?;
        validate_k8s_name("source PVC", &self.source_pvc)?;
        validate_k8s_name("target namespace", &self.target_namespace)?;
        validate_k8s_name("target DataVolume", &self.target_name)?;
        if self.size.trim().is_empty() {
            bail!("clone size must not be empty");
        }
        Ok(())
    }
}

/// CDI DataVolume that clones an existing PVC (bit-preserving when CDI is installed).
pub fn data_volume_clone_manifest(spec: &CdiPvcCloneSpec) -> Result<Value> {
    spec.validate()?;
    let mut storage = json!({
        "resources": { "requests": { "storage": spec.size } },
        // Same fix as golden_images::mod.rs's data_volume_manifest: CDI can
        // normally infer this from the target StorageClass's StorageProfile,
        // but not every cluster has one configured (e.g. k3s's built-in
        // "local-path" ships without a StorageProfile access mode) -- CDI
        // then rejects the clone with ErrClaimNotValid instead of cloning,
        // so set it explicitly rather than depending on cluster-specific
        // setup. Confirmed against a real cluster: cloning a PVC-backed VM's
        // disk failed with exactly this error before this was added.
        "accessModes": ["ReadWriteOnce"]
    });
    if let Some(sc) = &spec.storage_class {
        storage["storageClassName"] = json!(sc);
    }
    Ok(json!({
        "apiVersion": "cdi.kubevirt.io/v1beta1",
        "kind": "DataVolume",
        "metadata": {
            "name": spec.target_name,
            "namespace": spec.target_namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "zorvia",
                "zorvia.io/clone-source": spec.source_pvc,
            },
            // Clone now, don't wait for a consumer (see
            // golden_images::BIND_IMMEDIATE_ANNOTATION): otherwise a capture
            // job never completes on a WaitForFirstConsumer StorageClass.
            "annotations": {
                "cdi.kubevirt.io/storage.bind.immediate.requested": "true"
            }
        },
        "spec": {
            "source": {
                "pvc": {
                    "namespace": spec.source_namespace,
                    "name": spec.source_pvc
                }
            },
            "storage": storage
        }
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataVolumeWait {
    Ready,
    Pending,
    Failed,
}

/// Classify a CDI DataVolume status.phase.
pub fn classify_data_volume_phase(phase: Option<&str>) -> DataVolumeWait {
    match phase.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("succeeded") | Some("ready") => DataVolumeWait::Ready,
        Some("failed") | Some("paused") | Some("unknown") => DataVolumeWait::Failed,
        _ => DataVolumeWait::Pending,
    }
}

pub fn data_volume_phase_from_object(obj: &serde_json::Value) -> Option<String> {
    obj.get("status")
        .and_then(|s| s.get("phase"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

/// Point-in-time view of a DataVolume: phase class, CDI's own transfer
/// progress (`status.progress`, e.g. "45.3%"), and -- when it failed -- the
/// most useful condition message.
#[derive(Debug, Clone, PartialEq)]
pub struct DataVolumeProgress {
    pub wait: DataVolumeWait,
    pub percent: Option<u8>,
    pub failure_reason: Option<String>,
}

pub fn data_volume_progress_from_object(obj: &serde_json::Value) -> DataVolumeProgress {
    let phase = data_volume_phase_from_object(obj);
    let wait = classify_data_volume_phase(phase.as_deref());
    let status = obj.get("status");
    let percent = status
        .and_then(|s| s.get("progress"))
        .and_then(|p| p.as_str())
        .and_then(|p| p.trim().trim_end_matches('%').parse::<f64>().ok())
        .map(|p| p.clamp(0.0, 100.0) as u8);
    let failure_reason = if wait == DataVolumeWait::Failed {
        let conditions = status
            .and_then(|s| s.get("conditions"))
            .and_then(|c| c.as_array());
        let msg = |c: &serde_json::Value| {
            c.get("message")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty())
                .map(|m| m.to_string())
        };
        conditions
            .and_then(|cs| {
                cs.iter()
                    .filter(|c| c.get("status").and_then(|s| s.as_str()) == Some("False"))
                    .find_map(msg)
                    .or_else(|| cs.iter().find_map(msg))
            })
            .or_else(|| phase.map(|p| format!("DataVolume phase {p}")))
    } else {
        None
    };
    DataVolumeProgress {
        wait,
        percent,
        failure_reason,
    }
}

/// Suggested DataVolume name for a cloned disk.
pub fn clone_dv_name(target_vm: &str, disk: &str) -> String {
    let raw = format!("{target_vm}-{disk}");
    raw.chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(63)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CdiPvcCloneSpec {
        CdiPvcCloneSpec {
            source_namespace: "default".into(),
            source_pvc: "prod-db-root".into(),
            target_namespace: "default".into(),
            target_name: "staging-db-root".into(),
            size: "40Gi".into(),
            storage_class: Some("fast".into()),
        }
    }

    #[test]
    fn clone_manifest_points_at_source_pvc() {
        let dv = data_volume_clone_manifest(&spec()).unwrap();
        assert_eq!(dv["kind"], "DataVolume");
        assert_eq!(dv["spec"]["source"]["pvc"]["name"], "prod-db-root");
        assert_eq!(dv["spec"]["storage"]["storageClassName"], "fast");
        assert_eq!(
            dv["spec"]["storage"]["resources"]["requests"]["storage"],
            "40Gi"
        );
    }

    #[test]
    fn clone_manifest_requests_immediate_binding() {
        let dv = data_volume_clone_manifest(&spec()).unwrap();
        assert_eq!(
            dv["metadata"]["annotations"]["cdi.kubevirt.io/storage.bind.immediate.requested"],
            "true"
        );
    }

    #[test]
    fn clone_manifest_sets_an_explicit_access_mode() {
        // Regression: verified against a real cluster -- CDI rejects a clone
        // DataVolume with ErrClaimNotValid on any StorageClass whose
        // StorageProfile doesn't declare an access mode (e.g. k3s's built-in
        // "local-path"), unless one is set explicitly here.
        let dv = data_volume_clone_manifest(&spec()).unwrap();
        assert_eq!(
            dv["spec"]["storage"]["accessModes"],
            serde_json::json!(["ReadWriteOnce"])
        );
    }

    #[test]
    fn rejects_bad_names() {
        let mut bad = spec();
        bad.target_name = "NOPE_UPPER".into();
        assert!(data_volume_clone_manifest(&bad).is_err());
    }

    #[test]
    fn classifies_dv_phases() {
        assert_eq!(
            classify_data_volume_phase(Some("Succeeded")),
            DataVolumeWait::Ready
        );
        assert_eq!(
            classify_data_volume_phase(Some("ImportInProgress")),
            DataVolumeWait::Pending
        );
        assert_eq!(
            classify_data_volume_phase(Some("Failed")),
            DataVolumeWait::Failed
        );
        let obj = serde_json::json!({"status": {"phase": "Succeeded"}});
        assert_eq!(
            data_volume_phase_from_object(&obj).as_deref(),
            Some("Succeeded")
        );
    }

    #[test]
    fn progress_parses_percent_and_failure_reason() {
        let running =
            serde_json::json!({"status": {"phase": "CloneInProgress", "progress": "45.3%"}});
        let p = data_volume_progress_from_object(&running);
        assert_eq!(p.wait, DataVolumeWait::Pending);
        assert_eq!(p.percent, Some(45));
        assert!(p.failure_reason.is_none());

        let failed = serde_json::json!({"status": {"phase": "Failed", "conditions": [
            {"type": "Bound", "status": "True", "message": "ok"},
            {"type": "Running", "status": "False", "message": "clone pod crashed"}
        ]}});
        let p = data_volume_progress_from_object(&failed);
        assert_eq!(p.wait, DataVolumeWait::Failed);
        assert_eq!(p.failure_reason.as_deref(), Some("clone pod crashed"));

        let bare = serde_json::json!({});
        assert_eq!(data_volume_progress_from_object(&bare).percent, None);
    }

    #[test]
    fn dv_name_is_dns_safe() {
        let name = clone_dv_name("Web_01", "root disk");
        assert!(name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert!(name.len() <= 63);
    }
}
