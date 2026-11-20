// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Operational change planning built on top of Drift Guard.
//!
//! Drift Guard answers "what changed?". Change Planner answers the next
//! operational question: "what does this change mean for a running VM?".
//! The planner deliberately stays conservative when KubeVirt capabilities
//! depend on cluster version or feature gates; uncertain operations are marked
//! for manual review instead of being presented as safely hot-pluggable.

use crate::gitops::drift::{DriftFinding, DriftReport, DriftSeverity};
use crate::gitops::reconciliation::DriftType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Runtime impact of a declarative change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeDisposition {
    /// Can be applied without intentionally stopping/recreating the VM.
    Online,
    /// Safest generic path is a controlled stop/start or restart.
    RestartRequired,
    /// The VM/VMI should be recreated after preserving a rollback point.
    RecreateRequired,
    /// Cluster capabilities or workload semantics must be checked by an operator.
    ManualReview,
}

impl ChangeDisposition {
    fn rank(self) -> u8 {
        match self {
            Self::Online => 0,
            Self::RestartRequired => 1,
            Self::ManualReview => 2,
            Self::RecreateRequired => 3,
        }
    }

    fn max(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

impl std::fmt::Display for ChangeDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Online => "online",
            Self::RestartRequired => "restart-required",
            Self::RecreateRequired => "recreate-required",
            Self::ManualReview => "manual-review",
        };
        f.write_str(value)
    }
}

/// One drift finding translated into an operational action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedChange {
    pub path: String,
    pub diff_type: DriftType,
    pub severity: DriftSeverity,
    pub disposition: ChangeDisposition,
    pub reason: String,
    pub desired_value: Option<String>,
    pub actual_value: Option<String>,
    pub action: String,
}

/// Complete plan for reconciling one VM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePlan {
    pub resource: String,
    pub risk_score: u8,
    pub highest_severity: Option<DriftSeverity>,
    pub overall_disposition: ChangeDisposition,
    pub requires_downtime: bool,
    pub requires_recreation: bool,
    pub manual_review_required: bool,
    pub changes: Vec<PlannedChange>,
    pub preflight_checks: Vec<String>,
    pub execution_steps: Vec<String>,
    pub postflight_checks: Vec<String>,
}

impl ChangePlan {
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn change_count(&self) -> usize {
        self.changes.len()
    }
}

/// Conservative planner that translates semantic drift into an execution plan.
pub struct ChangePlanner;

impl ChangePlanner {
    pub fn plan(report: &DriftReport) -> ChangePlan {
        let mut changes = Vec::with_capacity(report.findings.len());
        let mut disposition = ChangeDisposition::Online;
        let mut preflight = BTreeSet::new();
        let mut execution = BTreeSet::new();
        let mut postflight = BTreeSet::new();

        if !report.findings.is_empty() {
            preflight
                .insert("Confirm the target VM has a recent rollback point or backup".to_string());
            preflight.insert(
                "Confirm the VM is not already migrating or in an unhealthy state".to_string(),
            );
            postflight.insert(
                "Verify VM Ready state and workload health after reconciliation".to_string(),
            );
        }

        for finding in &report.findings {
            let planned = classify_finding(finding);
            disposition = disposition.max(planned.disposition);
            add_checks_and_steps(&planned, &mut preflight, &mut execution, &mut postflight);
            changes.push(planned);
        }

        let requires_recreation = changes
            .iter()
            .any(|change| change.disposition == ChangeDisposition::RecreateRequired);
        let manual_review_required = changes
            .iter()
            .any(|change| change.disposition == ChangeDisposition::ManualReview);
        let requires_downtime = changes.iter().any(|change| {
            matches!(
                change.disposition,
                ChangeDisposition::RestartRequired | ChangeDisposition::RecreateRequired
            )
        });

        ChangePlan {
            resource: report.resource.full_name(),
            risk_score: report.risk_score,
            highest_severity: report.highest_severity,
            overall_disposition: disposition,
            requires_downtime,
            requires_recreation,
            manual_review_required,
            changes,
            preflight_checks: preflight.into_iter().collect(),
            execution_steps: execution.into_iter().collect(),
            postflight_checks: postflight.into_iter().collect(),
        }
    }
}

fn classify_finding(finding: &DriftFinding) -> PlannedChange {
    let path = finding.path.to_ascii_lowercase();

    let (disposition, reason, action) = if path.starts_with("/metadata/labels")
        || path.starts_with("/metadata/annotations")
    {
        (
            ChangeDisposition::Online,
            "metadata-only change does not alter guest hardware",
            "Apply the metadata patch and verify selectors/automation that consume it",
        )
    } else if path.contains("cloudinit") || path.contains("cloud-init") {
        (
            ChangeDisposition::RecreateRequired,
            "cloud-init normally takes effect during first boot, not on an existing guest",
            "Preserve a rollback point and reprovision the guest to apply cloud-init deterministically",
        )
    } else if path.contains("/datavolumetemplates") || path.contains("/spec/template/spec/volumes")
    {
        if finding.desired_value.is_some() && finding.actual_value.is_none() {
            (
                ChangeDisposition::ManualReview,
                "volume attachment behavior depends on CDI/storage capabilities and guest support",
                "Validate storage class, access mode and hotplug support before attaching the new volume",
            )
        } else {
            (
                ChangeDisposition::RecreateRequired,
                "changing or removing a volume source can alter persistent guest data",
                "Create a snapshot/backup, validate the replacement source, then recreate under a controlled window",
            )
        }
    } else if path.contains("/spec/template/spec/domain/devices/disks") {
        if finding.desired_value.is_none() && finding.actual_value.is_some() {
            (
                ChangeDisposition::RecreateRequired,
                "removing a guest disk device can make the guest unbootable or detach data",
                "Protect the disk data, validate boot order, then recreate the VM/VMI under a maintenance window",
            )
        } else {
            (
                ChangeDisposition::RestartRequired,
                "disk bus, boot order or device properties are safest to change while the guest is stopped",
                "Stop the VM, apply the disk-device change, start it, then verify boot and filesystem health",
            )
        }
    } else if path.contains("/spec/template/spec/domain/firmware")
        || path.contains("/spec/template/spec/domain/machine")
        || path.contains("/tpm")
    {
        (
            ChangeDisposition::RecreateRequired,
            "firmware, machine type or TPM changes affect the virtual hardware contract",
            "Take a rollback point, validate guest compatibility, and recreate the VM/VMI during maintenance",
        )
    } else if path.contains("/spec/template/spec/domain/devices/interfaces")
        || path.contains("/spec/template/spec/networks")
    {
        (
            ChangeDisposition::RestartRequired,
            "network topology changes can alter MAC/IP identity and workload reachability",
            "Validate NetworkAttachmentDefinitions/policies, apply during maintenance, then verify connectivity",
        )
    } else if path.contains("/spec/template/spec/nodeselector")
        || path.contains("/spec/template/spec/affinity")
        || path.contains("/evictionstrategy")
    {
        (
            ChangeDisposition::RestartRequired,
            "placement policy changes take effect when the VMI is rescheduled or migrated",
            "Confirm a compatible target node exists, then restart or migrate the VMI in a controlled window",
        )
    } else if path.contains("/spec/template/spec/domain/cpu")
        || path.contains("/spec/template/spec/domain/memory")
        || path.contains("/spec/template/spec/domain/resources")
    {
        (
            ChangeDisposition::RestartRequired,
            "CPU/memory hotplug support varies by KubeVirt version, VM configuration and guest OS",
            "Check hotplug capability; otherwise stop the VM, apply resources, restart and validate the guest",
        )
    } else if path.ends_with("/spec/running") || path.ends_with("/spec/runstrategy") {
        (
            ChangeDisposition::RestartRequired,
            "the desired lifecycle state intentionally changes VM availability",
            "Coordinate the lifecycle transition with the workload owner and verify service recovery",
        )
    } else if path.contains("/terminationgraceperiodseconds") {
        (
            ChangeDisposition::Online,
            "termination policy changes affect future shutdown behavior rather than current guest hardware",
            "Apply the lifecycle policy and verify the expected shutdown window",
        )
    } else {
        (
            ChangeDisposition::ManualReview,
            "the field is not yet mapped to a deterministic KubeVirt execution strategy",
            "Review KubeVirt behavior for this field before automatic reconciliation",
        )
    };

    PlannedChange {
        path: finding.path.clone(),
        diff_type: finding.diff_type.clone(),
        severity: finding.severity,
        disposition,
        reason: reason.to_string(),
        desired_value: finding.desired_value.clone(),
        actual_value: finding.actual_value.clone(),
        action: action.to_string(),
    }
}

fn add_checks_and_steps(
    change: &PlannedChange,
    preflight: &mut BTreeSet<String>,
    execution: &mut BTreeSet<String>,
    postflight: &mut BTreeSet<String>,
) {
    execution.insert(change.action.clone());

    match change.disposition {
        ChangeDisposition::Online => {}
        ChangeDisposition::RestartRequired => {
            preflight.insert(
                "Confirm an approved maintenance window or verified hotplug path".to_string(),
            );
            postflight.insert(
                "Confirm the guest returned to its expected node/network state".to_string(),
            );
        }
        ChangeDisposition::RecreateRequired => {
            preflight.insert(
                "Validate recovery by snapshot/backup before destructive reconciliation"
                    .to_string(),
            );
            preflight.insert(
                "Confirm persistent volumes will be retained and reattached as intended"
                    .to_string(),
            );
            postflight.insert(
                "Verify boot, storage attachment and application data after recreation".to_string(),
            );
        }
        ChangeDisposition::ManualReview => {
            preflight.insert(
                "Require explicit operator approval for unmapped/feature-gated behavior"
                    .to_string(),
            );
        }
    }

    if change.path.to_ascii_lowercase().contains("network")
        || change.path.to_ascii_lowercase().contains("interfaces")
    {
        preflight.insert(
            "Validate Multus/NAD, NetworkPolicy and expected MAC/IP dependencies".to_string(),
        );
        postflight.insert("Verify guest ingress/egress and dependent services".to_string());
    }

    if change.path.to_ascii_lowercase().contains("volume")
        || change.path.to_ascii_lowercase().contains("disks")
    {
        preflight
            .insert("Validate PVC/DataVolume readiness and storage-class capabilities".to_string());
        postflight.insert("Verify guest disks, mounts and filesystem integrity".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitops::drift::{DriftEngine, DriftOptions};
    use serde_json::json;

    fn plan_for(desired: serde_json::Value, actual: serde_json::Value) -> ChangePlan {
        let report = DriftEngine::new(DriftOptions::default()).compare(&desired, &actual);
        ChangePlanner::plan(&report)
    }

    fn vm(spec: serde_json::Value) -> serde_json::Value {
        json!({
            "apiVersion": "kubevirt.io/v1",
            "kind": "VirtualMachine",
            "metadata": {"name": "demo", "namespace": "default"},
            "spec": spec
        })
    }

    #[test]
    fn noop_plan_has_no_steps() {
        let value = vm(json!({"running": true}));
        let plan = plan_for(value.clone(), value);
        assert!(plan.is_noop());
        assert!(!plan.requires_downtime);
    }

    #[test]
    fn metadata_is_online() {
        let desired =
            json!({"kind":"VirtualMachine","metadata":{"name":"demo","labels":{"team":"a"}}});
        let actual =
            json!({"kind":"VirtualMachine","metadata":{"name":"demo","labels":{"team":"b"}}});
        let plan = plan_for(desired, actual);
        assert_eq!(plan.overall_disposition, ChangeDisposition::Online);
    }

    #[test]
    fn cpu_change_requires_restart() {
        let desired = vm(json!({"template":{"spec":{"domain":{"cpu":{"cores":4}}}}}));
        let actual = vm(json!({"template":{"spec":{"domain":{"cpu":{"cores":2}}}}}));
        let plan = plan_for(desired, actual);
        assert_eq!(plan.overall_disposition, ChangeDisposition::RestartRequired);
        assert!(plan.requires_downtime);
    }

    #[test]
    fn memory_change_requires_restart() {
        let desired = vm(json!({"template":{"spec":{"domain":{"memory":{"guest":"8Gi"}}}}}));
        let actual = vm(json!({"template":{"spec":{"domain":{"memory":{"guest":"4Gi"}}}}}));
        let plan = plan_for(desired, actual);
        assert_eq!(plan.overall_disposition, ChangeDisposition::RestartRequired);
    }

    #[test]
    fn firmware_change_requires_recreation() {
        let desired =
            vm(json!({"template":{"spec":{"domain":{"firmware":{"bootloader":{"efi":{}}}}}}}));
        let actual =
            vm(json!({"template":{"spec":{"domain":{"firmware":{"bootloader":{"bios":{}}}}}}}));
        let plan = plan_for(desired, actual);
        assert!(plan.requires_recreation);
        assert_eq!(
            plan.overall_disposition,
            ChangeDisposition::RecreateRequired
        );
    }

    #[test]
    fn volume_source_change_requires_recreation() {
        let desired = vm(
            json!({"template":{"spec":{"volumes":[{"name":"root","dataVolume":{"name":"gold-v2"}}]}}}),
        );
        let actual = vm(
            json!({"template":{"spec":{"volumes":[{"name":"root","dataVolume":{"name":"gold-v1"}}]}}}),
        );
        let plan = plan_for(desired, actual);
        assert!(plan.requires_recreation);
    }

    #[test]
    fn added_volume_requires_manual_review() {
        let desired = vm(
            json!({"template":{"spec":{"volumes":[{"name":"data","persistentVolumeClaim":{"claimName":"data"}}]}}}),
        );
        let actual = vm(json!({"template":{"spec":{"volumes":[]}}}));
        let plan = plan_for(desired, actual);
        assert!(plan.manual_review_required);
    }

    #[test]
    fn network_change_requires_restart() {
        let desired = vm(json!({"template":{"spec":{"networks":[{"name":"default","pod":{}}]}}}));
        let actual = vm(
            json!({"template":{"spec":{"networks":[{"name":"default","multus":{"networkName":"prod"}}]}}}),
        );
        let plan = plan_for(desired, actual);
        assert_eq!(plan.overall_disposition, ChangeDisposition::RestartRequired);
    }

    #[test]
    fn placement_change_requires_restart() {
        let desired = vm(json!({"template":{"spec":{"nodeSelector":{"zone":"a"}}}}));
        let actual = vm(json!({"template":{"spec":{"nodeSelector":{"zone":"b"}}}}));
        let plan = plan_for(desired, actual);
        assert_eq!(plan.overall_disposition, ChangeDisposition::RestartRequired);
    }

    #[test]
    fn unknown_field_requires_manual_review() {
        let desired = vm(json!({"customField":"a"}));
        let actual = vm(json!({"customField":"b"}));
        let plan = plan_for(desired, actual);
        assert!(plan.manual_review_required);
        assert_eq!(plan.overall_disposition, ChangeDisposition::ManualReview);
    }

    #[test]
    fn destructive_change_dominates_online_change() {
        let desired = json!({
            "kind":"VirtualMachine",
            "metadata":{"name":"demo","labels":{"team":"a"}},
            "spec":{"template":{"spec":{"domain":{"machine":{"type":"q35"}}}}}
        });
        let actual = json!({
            "kind":"VirtualMachine",
            "metadata":{"name":"demo","labels":{"team":"b"}},
            "spec":{"template":{"spec":{"domain":{"machine":{"type":"pc"}}}}}
        });
        let plan = plan_for(desired, actual);
        assert_eq!(
            plan.overall_disposition,
            ChangeDisposition::RecreateRequired
        );
        assert!(plan.change_count() >= 2);
    }

    #[test]
    fn storage_plan_adds_storage_checks() {
        let desired = vm(
            json!({"template":{"spec":{"volumes":[{"name":"root","dataVolume":{"name":"v2"}}]}}}),
        );
        let actual = vm(
            json!({"template":{"spec":{"volumes":[{"name":"root","dataVolume":{"name":"v1"}}]}}}),
        );
        let plan = plan_for(desired, actual);
        assert!(plan
            .preflight_checks
            .iter()
            .any(|v| v.contains("PVC/DataVolume")));
        assert!(plan
            .postflight_checks
            .iter()
            .any(|v| v.contains("filesystem")));
    }
}
