// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Pre-drain maintenance planner.
//!
//! Answers "what happens to my VMs if I drain this node?" before anyone drains
//! it: which VMs can live-migrate and where, which cannot and why, and what the
//! disruption will be. It is pure logic over a snapshot (the API handler
//! collects the snapshot), recommendation-only, and never moves anything.
//!
//! Destination capacity is estimated from VM CPU/memory requests against node
//! allocatable. It does not model storage topology, NUMA, device or affinity
//! constraints beyond the migratability KubeVirt itself reports, so a plan
//! that says "migrate" is a strong hint, not a guarantee.

use super::advisor::NodeSnapshot;
use serde::{Deserialize, Serialize};

/// One VM running on the node being drained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceVm {
    pub name: String,
    pub namespace: String,
    pub cpu: f64,
    pub memory_gib: f64,
    /// KubeVirt's `LiveMigratable` condition on the VMI (`None` = not reported).
    pub live_migratable: Option<bool>,
    /// The condition's message/reason when it is false.
    pub not_migratable_reason: Option<String>,
    /// `spec.evictionStrategy` of the VMI (`LiveMigrate`, `LiveMigrateIfPossible`, `None`, ...).
    pub eviction_strategy: Option<String>,
    /// Names of PVCs that can only be mounted by one node (ReadWriteOnce-only).
    pub rwo_only_volumes: Vec<String>,
    /// The VM holds host devices (GPU, SR-IOV, hostDevices) that pin it to this node.
    pub pinned_devices: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Live-migrate to `destination` before or during the drain.
    LiveMigrate,
    /// Cannot live-migrate; the drain will stop it (it restarts elsewhere only
    /// if its run strategy allows). Downtime.
    Downtime,
    /// No suitable destination (capacity); drain would be blocked or cause downtime.
    NoDestination,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmVerdict {
    pub name: String,
    pub namespace: String,
    pub action: Action,
    pub destination: Option<String>,
    pub reasons: Vec<String>,
    /// `none` (live migration), `downtime`.
    pub disruption: String,
}

impl VmVerdict {
    #[cfg(test)]
    fn no_dest(&self) -> bool {
        self.action == Action::NoDestination
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenancePlan {
    pub node: String,
    pub node_found: bool,
    pub already_cordoned: bool,
    pub vm_count: usize,
    pub can_drain_without_downtime: bool,
    pub live_migrate: usize,
    pub downtime: usize,
    pub no_destination: usize,
    pub vms: Vec<VmVerdict>,
    pub warnings: Vec<String>,
}

fn eviction_migrates(strategy: Option<&str>) -> bool {
    matches!(
        strategy,
        Some("LiveMigrate") | Some("LiveMigrateIfPossible")
    )
}

/// Why a VM cannot be live-migrated, if it cannot.
fn blockers(vm: &MaintenanceVm) -> Vec<String> {
    let mut r = Vec::new();
    if vm.live_migratable == Some(false) {
        r.push(match &vm.not_migratable_reason {
            Some(why) if !why.is_empty() => {
                format!("KubeVirt reports it is not live-migratable: {why}")
            }
            _ => "KubeVirt reports it is not live-migratable".to_string(),
        });
    }
    if !vm.rwo_only_volumes.is_empty() {
        r.push(format!(
            "ReadWriteOnce-only volume(s) cannot be attached to two nodes: {}",
            vm.rwo_only_volumes.join(", ")
        ));
    }
    if vm.pinned_devices {
        r.push("host devices (GPU/SR-IOV/hostDevices) pin it to this node".to_string());
    }
    r
}

/// Build the plan for draining `node`. `nodes` must include every node, the
/// drained one too (its usage is ignored; the VMs move away from it).
pub fn plan_drain(node: &str, nodes: &[NodeSnapshot], vms: &[MaintenanceVm]) -> MaintenancePlan {
    let target = nodes.iter().find(|n| n.name == node);
    let mut warnings = Vec::new();
    let other_nodes = nodes.iter().filter(|n| n.name != node).count();
    if target.is_none() {
        warnings.push(format!("node '{node}' was not found"));
    }
    let already_cordoned = target.map(|n| n.unschedulable).unwrap_or(false);

    // Remaining free capacity of every possible destination, decremented as VMs
    // are assigned so two VMs are not promised the same room.
    let mut free: Vec<(String, f64, f64, f64)> = nodes
        .iter()
        .filter(|n| n.name != node && !n.unschedulable)
        .map(|n| {
            (
                n.name.clone(),
                n.free_cpu(),
                n.free_memory_gib(),
                n.load_score(),
            )
        })
        .collect();
    if other_nodes == 0 {
        warnings.push(
            "there is no other node: nothing can be migrated, a drain means downtime for every VM"
                .into(),
        );
    } else if free.is_empty() {
        warnings.push("every other node is cordoned (unschedulable)".into());
    }

    // Place the largest VMs first: they are the hardest to fit.
    let mut order: Vec<&MaintenanceVm> = vms.iter().collect();
    order.sort_by(|a, b| {
        b.memory_gib
            .partial_cmp(&a.memory_gib)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut verdicts = Vec::with_capacity(vms.len());
    for vm in order {
        let blocked = blockers(vm);
        if !blocked.is_empty() {
            verdicts.push(VmVerdict {
                name: vm.name.clone(),
                namespace: vm.namespace.clone(),
                action: Action::Downtime,
                destination: None,
                reasons: blocked,
                disruption: "downtime".into(),
            });
            continue;
        }
        let mut reasons = Vec::new();
        if !eviction_migrates(vm.eviction_strategy.as_deref()) {
            reasons.push(
                "evictionStrategy does not live-migrate on drain: migrate it first (POST /api/vms/{name}/migrate) or set LiveMigrate, otherwise the drain stops it"
                    .to_string(),
            );
        }
        // Least-loaded destination with room for the VM.
        let pick = free
            .iter_mut()
            .filter(|(_, c, m, _)| *c >= vm.cpu && *m >= vm.memory_gib)
            .min_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));
        match pick {
            Some(dest) => {
                dest.1 -= vm.cpu;
                dest.2 -= vm.memory_gib;
                verdicts.push(VmVerdict {
                    name: vm.name.clone(),
                    namespace: vm.namespace.clone(),
                    action: Action::LiveMigrate,
                    destination: Some(dest.0.clone()),
                    reasons,
                    disruption: "none".into(),
                });
            }
            None => {
                reasons.push(if other_nodes == 0 {
                    "there is no other node to move it to".to_string()
                } else if free.is_empty() {
                    "every other node is cordoned (unschedulable)".to_string()
                } else {
                    format!(
                        "no other schedulable node has {:.1} vCPU and {:.1} GiB free",
                        vm.cpu, vm.memory_gib
                    )
                });
                verdicts.push(VmVerdict {
                    name: vm.name.clone(),
                    namespace: vm.namespace.clone(),
                    action: Action::NoDestination,
                    destination: None,
                    reasons,
                    disruption: "downtime".into(),
                });
            }
        }
    }
    verdicts.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));

    let count = |a: Action| verdicts.iter().filter(|v| v.action == a).count();
    let live_migrate = count(Action::LiveMigrate);
    let downtime = count(Action::Downtime);
    let no_destination = count(Action::NoDestination);
    let unmanaged = verdicts
        .iter()
        .filter(|v| v.action == Action::LiveMigrate && !v.reasons.is_empty())
        .count();
    if unmanaged > 0 {
        warnings.push(format!(
            "{unmanaged} VM(s) can migrate but will not be migrated automatically by a drain"
        ));
    }
    MaintenancePlan {
        node: node.to_string(),
        node_found: target.is_some(),
        already_cordoned,
        vm_count: verdicts.len(),
        can_drain_without_downtime: target.is_some()
            && downtime == 0
            && no_destination == 0
            && unmanaged == 0,
        live_migrate,
        downtime,
        no_destination,
        vms: verdicts,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn node(name: &str, cpu: f64, mem: f64, used_cpu: f64, used_mem: f64) -> NodeSnapshot {
        NodeSnapshot {
            name: name.into(),
            allocatable_cpu: cpu,
            allocatable_memory_gib: mem,
            used_cpu,
            used_memory_gib: used_mem,
            vm_count: 0,
            labels: BTreeMap::new(),
            taints: BTreeSet::new(),
            unschedulable: false,
        }
    }

    fn vm(name: &str, cpu: f64, mem: f64) -> MaintenanceVm {
        MaintenanceVm {
            name: name.into(),
            namespace: "default".into(),
            cpu,
            memory_gib: mem,
            live_migratable: Some(true),
            not_migratable_reason: None,
            eviction_strategy: Some("LiveMigrate".into()),
            rwo_only_volumes: vec![],
            pinned_devices: false,
        }
    }

    #[test]
    fn migratable_vms_get_a_destination_and_a_clean_drain() {
        let nodes = vec![
            node("a", 8.0, 32.0, 4.0, 8.0),
            node("b", 8.0, 32.0, 1.0, 4.0),
        ];
        let p = plan_drain("a", &nodes, &[vm("web", 2.0, 4.0), vm("db", 2.0, 8.0)]);
        assert!(p.node_found && p.can_drain_without_downtime);
        assert_eq!((p.live_migrate, p.downtime, p.no_destination), (2, 0, 0));
        assert!(p.vms.iter().all(|v| v.destination.as_deref() == Some("b")));
    }

    #[test]
    fn single_node_cluster_means_downtime_for_everyone() {
        let p = plan_drain(
            "a",
            &[node("a", 8.0, 32.0, 2.0, 4.0)],
            &[vm("web", 2.0, 4.0)],
        );
        assert!(!p.can_drain_without_downtime);
        assert_eq!(p.no_destination, 1);
        assert!(p.warnings.iter().any(|w| w.contains("no other node")));
        assert_eq!(
            p.vms[0].reasons,
            vec!["there is no other node to move it to"]
        );
    }

    #[test]
    fn capacity_is_not_promised_twice() {
        // b has room for only one of the two 6 GiB VMs.
        let nodes = vec![
            node("a", 8.0, 32.0, 4.0, 12.0),
            node("b", 8.0, 12.0, 0.0, 0.0),
        ];
        let p = plan_drain("a", &nodes, &[vm("x", 1.0, 6.0), vm("y", 1.0, 6.5)]);
        assert_eq!(p.live_migrate, 1);
        assert_eq!(p.no_destination, 1);
        assert!(!p.can_drain_without_downtime);
        let blocked = p.vms.iter().find(|v| v.no_dest()).unwrap();
        assert!(blocked.reasons[0].starts_with("no other schedulable node has"));
    }

    #[test]
    fn rwo_volumes_devices_and_kubevirt_flag_block_migration() {
        let nodes = vec![
            node("a", 8.0, 32.0, 0.0, 0.0),
            node("b", 8.0, 32.0, 0.0, 0.0),
        ];
        let mut rwo = vm("rwo", 1.0, 1.0);
        rwo.rwo_only_volumes = vec!["data".into()];
        let mut gpu = vm("gpu", 1.0, 1.0);
        gpu.pinned_devices = true;
        let mut flagged = vm("flagged", 1.0, 1.0);
        flagged.live_migratable = Some(false);
        flagged.not_migratable_reason = Some("DisksNotLiveMigratable".into());
        let p = plan_drain("a", &nodes, &[rwo, gpu, flagged]);
        assert_eq!(p.downtime, 3);
        assert!(p
            .vms
            .iter()
            .all(|v| v.action == Action::Downtime && v.disruption == "downtime"));
        assert!(
            p.vms.iter().find(|v| v.name == "flagged").unwrap().reasons[0]
                .contains("DisksNotLiveMigratable")
        );
    }

    #[test]
    fn eviction_strategy_that_does_not_migrate_is_called_out() {
        let nodes = vec![
            node("a", 8.0, 32.0, 0.0, 0.0),
            node("b", 8.0, 32.0, 0.0, 0.0),
        ];
        let mut v = vm("web", 1.0, 1.0);
        v.eviction_strategy = None;
        let p = plan_drain("a", &nodes, &[v]);
        assert_eq!(p.live_migrate, 1);
        assert!(
            !p.can_drain_without_downtime,
            "drain would stop it, so not clean"
        );
        assert!(p
            .warnings
            .iter()
            .any(|w| w.contains("not be migrated automatically")));
    }

    #[test]
    fn cordoned_destinations_are_skipped_and_unknown_node_is_reported() {
        let mut b = node("b", 8.0, 32.0, 0.0, 0.0);
        b.unschedulable = true;
        let nodes = vec![node("a", 8.0, 32.0, 0.0, 0.0), b];
        let p = plan_drain("a", &nodes, &[vm("web", 1.0, 1.0)]);
        assert_eq!(p.no_destination, 1);
        assert!(p.warnings.iter().any(|w| w.contains("cordoned")));
        assert_eq!(
            p.vms[0].reasons,
            vec!["every other node is cordoned (unschedulable)"]
        );
        let missing = plan_drain("zz", &nodes, &[]);
        assert!(!missing.node_found && !missing.can_drain_without_downtime);
    }
}
