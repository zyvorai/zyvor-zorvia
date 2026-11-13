//! Read-only "adoption" report for an existing KubeVirt cluster.
//!
//! Summarises the VMs Zorvia can see and which Zorvia capabilities apply
//! today, using the maturity registry in [`crate::features`]. Used by both
//! `zorvia adopt` and `GET /api/v1/adopt/report`. Nothing here mutates the
//! cluster.

use crate::features::{feature, Maturity, FEATURES};
use crate::kube::VirtualMachine;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Minimal view of a VM, decoupled from the CRD type so it is easy to test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSummary {
    pub name: String,
    pub namespace: String,
    pub status: String,
}

impl VmSummary {
    pub fn from_vm(vm: &VirtualMachine) -> Self {
        Self {
            name: vm.metadata.name.clone().unwrap_or_default(),
            namespace: vm.metadata.namespace.clone().unwrap_or_default(),
            status: vm
                .status
                .as_ref()
                .and_then(|s| s.printable_status.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureRef {
    pub id: &'static str,
    pub name: &'static str,
    pub maturity: Maturity,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdoptReport {
    pub vm_count: usize,
    pub namespace_count: usize,
    pub by_status: BTreeMap<String, usize>,
    /// GA and Beta features: the production promises Zorvia makes today.
    pub managed_today: Vec<FeatureRef>,
    /// Needs beyond a single cluster that Zorvia does not cover at GA/Beta.
    pub outside_zorvia_today: Vec<FeatureRef>,
    pub note: &'static str,
}

/// Feature ids that matter for leaving another platform but are not GA/Beta.
const OUTSIDE_TODAY: &[&str] = &[
    "fleet-multicluster",
    "transiva-migration",
    "vm-import-h2kvm",
    "cross-cluster-dr",
    "gpu-sriov-numa",
    "golden-image-pipeline",
];

const NOTE: &str = "Read-only report. Not verified against OpenShift Virtualization; \
see docs/adopt-existing-kubevirt-cluster.md. For multi-cluster management, migration \
help and vendor support, ask about Zorvia Enterprise: sales@zyvor.dev";

fn feature_ref(f: &crate::features::Feature) -> FeatureRef {
    FeatureRef {
        id: f.id,
        name: f.name,
        maturity: f.maturity,
    }
}

pub fn build_report(vms: &[VmSummary]) -> AdoptReport {
    let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
    let mut namespaces: BTreeSet<&str> = BTreeSet::new();
    for vm in vms {
        *by_status.entry(vm.status.clone()).or_insert(0) += 1;
        namespaces.insert(vm.namespace.as_str());
    }

    let managed_today = FEATURES
        .iter()
        .filter(|f| matches!(f.maturity, Maturity::Ga | Maturity::Beta))
        .map(feature_ref)
        .collect();

    let outside_zorvia_today = OUTSIDE_TODAY
        .iter()
        .filter_map(|id| feature(id))
        .map(feature_ref)
        .collect();

    AdoptReport {
        vm_count: vms.len(),
        namespace_count: namespaces.len(),
        by_status,
        managed_today,
        outside_zorvia_today,
        note: NOTE,
    }
}

pub fn render_table(report: &AdoptReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "VMs: {} in {} namespace(s)\n",
        report.vm_count, report.namespace_count
    ));
    for (status, n) in &report.by_status {
        out.push_str(&format!("  {:<14} {}\n", status, n));
    }
    out.push_str("\nManaged by Zorvia today (GA/Beta):\n");
    for f in &report.managed_today {
        out.push_str(&format!(
            "  {:<22} {:<6} {}\n",
            f.id,
            f.maturity.as_str(),
            f.name
        ));
    }
    out.push_str("\nNot covered at GA/Beta (Zorvia):\n");
    for f in &report.outside_zorvia_today {
        out.push_str(&format!(
            "  {:<22} {:<12} {}\n",
            f.id,
            f.maturity.as_str(),
            f.name
        ));
    }
    out.push_str(&format!("\n{}\n", report.note));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(name: &str, ns: &str, status: &str) -> VmSummary {
        VmSummary {
            name: name.into(),
            namespace: ns.into(),
            status: status.into(),
        }
    }

    #[test]
    fn counts_vms_namespaces_and_status() {
        let r = build_report(&[
            vm("a", "prod", "Running"),
            vm("b", "prod", "Stopped"),
            vm("c", "dev", "Running"),
        ]);
        assert_eq!(r.vm_count, 3);
        assert_eq!(r.namespace_count, 2);
        assert_eq!(r.by_status.get("Running"), Some(&2));
        assert_eq!(r.by_status.get("Stopped"), Some(&1));
    }

    #[test]
    fn managed_today_is_only_ga_and_beta() {
        let r = build_report(&[]);
        assert!(!r.managed_today.is_empty());
        assert!(r
            .managed_today
            .iter()
            .all(|f| matches!(f.maturity, Maturity::Ga | Maturity::Beta)));
    }

    #[test]
    fn outside_today_is_never_ga_or_beta() {
        let r = build_report(&[]);
        assert!(!r.outside_zorvia_today.is_empty());
        assert!(r
            .outside_zorvia_today
            .iter()
            .all(|f| !matches!(f.maturity, Maturity::Ga | Maturity::Beta)));
        let ids: Vec<_> = r.outside_zorvia_today.iter().map(|f| f.id).collect();
        assert!(ids.contains(&"vm-import-h2kvm"));
        assert!(ids.contains(&"gpu-sriov-numa"));
        assert!(ids.contains(&"fleet-multicluster"));
    }

    #[test]
    fn table_mentions_counts_and_enterprise_contact() {
        let t = render_table(&build_report(&[vm("a", "prod", "Running")]));
        assert!(t.contains("VMs: 1 in 1 namespace(s)"));
        assert!(t.contains("Zorvia Enterprise"));
    }
}
