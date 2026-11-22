// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `GET /api/v1/maintenance/plan?node=NAME`: what a drain of NAME would do to
//! its VMs (see `crate::placement::maintenance`). Read-only; it never cordons,
//! drains or migrates. Cluster-wide, so it needs `cluster.admin`.

use super::*;
use crate::placement::advisor::NodeSnapshot;
use crate::placement::maintenance::{plan_drain, MaintenanceVm};
use k8s_openapi::api::core::v1::{Node, PersistentVolumeClaim};
use kube::api::{Api, DynamicObject, ListParams};
use kube::core::{ApiResource, GroupVersionKind};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
pub struct MaintenanceQuery {
    pub node: String,
}

fn quantity_gib(q: &str) -> f64 {
    parse_memory(q) as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn quantity_cores(q: &str) -> f64 {
    let q = q.trim();
    match q.strip_suffix('m') {
        Some(m) => m.parse::<f64>().unwrap_or(0.0) / 1000.0,
        None => q.parse::<f64>().unwrap_or(0.0),
    }
}

/// vCPU count and memory (GiB) a VMI asks for.
fn vmi_size(vmi: &Value) -> (f64, f64) {
    let dom = vmi.pointer("/spec/domain");
    let n = |p: &str| {
        dom.and_then(|d| d.pointer(p))
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .max(1.0)
    };
    let cpu = n("/cpu/cores") * n("/cpu/sockets") * n("/cpu/threads");
    let mem = dom
        .and_then(|d| d.pointer("/memory/guest"))
        .or_else(|| dom.and_then(|d| d.pointer("/resources/requests/memory")))
        .and_then(|v| v.as_str())
        .map(quantity_gib)
        .unwrap_or(0.0);
    (cpu, mem)
}

/// Claim names a VMI mounts (PVC and DataVolume volumes; a DataVolume's PVC has its name).
fn vmi_claims(vmi: &Value) -> Vec<String> {
    vmi.pointer("/spec/volumes")
        .and_then(|v| v.as_array())
        .map(|vols| {
            vols.iter()
                .filter_map(|v| {
                    v.pointer("/persistentVolumeClaim/claimName")
                        .or_else(|| v.pointer("/dataVolume/name"))
                        .and_then(|n| n.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn vmi_has_pinned_devices(vmi: &Value) -> bool {
    let non_empty = |p: &str| {
        vmi.pointer(p)
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
    };
    let sriov = vmi
        .pointer("/spec/domain/devices/interfaces")
        .and_then(|v| v.as_array())
        .is_some_and(|ifs| ifs.iter().any(|i| i.get("sriov").is_some()));
    non_empty("/spec/domain/devices/gpus") || non_empty("/spec/domain/devices/hostDevices") || sriov
}

/// `(live_migratable, message)` from the VMI's `LiveMigratable` condition.
fn live_migratable(vmi: &Value) -> (Option<bool>, Option<String>) {
    let cond = vmi
        .pointer("/status/conditions")
        .and_then(|c| c.as_array())
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("LiveMigratable"))
        });
    match cond {
        None => (None, None),
        Some(c) => {
            let ok = c.get("status").and_then(|s| s.as_str()) == Some("True");
            let why = ["reason", "message"]
                .iter()
                .filter_map(|k| c.get(*k).and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(": ");
            (Some(ok), (!ok && !why.is_empty()).then_some(why))
        }
    }
}

pub async fn maintenance_plan_handler(
    State(state): State<SharedState>,
    Query(q): Query<MaintenanceQuery>,
) -> impl IntoResponse {
    let target = q.node;
    if !target
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        || target.is_empty()
        || target.len() > 253
    {
        let (st, j) = err_json(400, "INVALID", "invalid node name");
        return (st, j).into_response();
    }
    let client = state.read().await.client();
    let raw = client.client();

    let nodes_api: Api<Node> = Api::all(raw.clone());
    let node_list = match nodes_api.list(&ListParams::default()).await {
        Ok(l) => l.items,
        Err(e) => {
            let (st, j) = err_json(500, "KUBE_ERROR", &sanitize_error(&anyhow::Error::from(e)));
            return (st, j).into_response();
        }
    };
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "kubevirt.io",
        "v1",
        "VirtualMachineInstance",
    ));
    let vmis: Api<DynamicObject> = Api::all_with(raw.clone(), &ar);
    let vmi_list = match vmis.list(&ListParams::default()).await {
        Ok(l) => l.items,
        Err(e) => {
            let (st, j) = err_json(500, "KUBE_ERROR", &sanitize_error(&anyhow::Error::from(e)));
            return (st, j).into_response();
        }
    };

    // Per-node usage from every running VMI, and the VMs on the target node.
    let mut usage: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();
    let mut on_target: Vec<(String, String, Value)> = Vec::new();
    for vmi in &vmi_list {
        let v = serde_json::to_value(vmi).unwrap_or(Value::Null);
        let Some(node) = v.pointer("/status/nodeName").and_then(|n| n.as_str()) else {
            continue;
        };
        let (cpu, mem) = vmi_size(&v);
        let e = usage.entry(node.to_string()).or_insert((0.0, 0.0, 0));
        e.0 += cpu;
        e.1 += mem;
        e.2 += 1;
        if node == target {
            on_target.push((
                vmi.metadata.namespace.clone().unwrap_or_default(),
                vmi.metadata.name.clone().unwrap_or_default(),
                v,
            ));
        }
    }

    let nodes: Vec<NodeSnapshot> = node_list
        .into_iter()
        .map(|n| {
            let name = n.metadata.name.clone().unwrap_or_default();
            let alloc = n.status.as_ref().and_then(|s| s.allocatable.as_ref());
            let (used_cpu, used_memory_gib, vm_count) =
                usage.get(&name).copied().unwrap_or((0.0, 0.0, 0));
            NodeSnapshot {
                allocatable_cpu: alloc
                    .and_then(|a| a.get("cpu"))
                    .map(|q| quantity_cores(&q.0))
                    .unwrap_or(0.0),
                allocatable_memory_gib: alloc
                    .and_then(|a| a.get("memory"))
                    .map(|q| quantity_gib(&q.0))
                    .unwrap_or(0.0),
                used_cpu,
                used_memory_gib,
                vm_count,
                labels: n
                    .metadata
                    .labels
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                taints: BTreeSet::new(),
                unschedulable: n
                    .spec
                    .as_ref()
                    .and_then(|s| s.unschedulable)
                    .unwrap_or(false),
                name,
            }
        })
        .collect();

    let mut vms = Vec::with_capacity(on_target.len());
    for (namespace, name, v) in on_target {
        // A claim is "RWO-only" when it offers no multi-node access mode. If the
        // PVC cannot be read we say nothing rather than guess.
        let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(raw.clone(), &namespace);
        let mut rwo_only_volumes = Vec::new();
        for claim in vmi_claims(&v) {
            if let Ok(pvc) = pvcs.get(&claim).await {
                let modes = pvc.spec.and_then(|s| s.access_modes).unwrap_or_default();
                let multi = modes
                    .iter()
                    .any(|m| m == "ReadWriteMany" || m == "ReadOnlyMany");
                if !modes.is_empty() && !multi {
                    rwo_only_volumes.push(claim);
                }
            }
        }
        let (migratable, reason) = live_migratable(&v);
        let (cpu, memory_gib) = vmi_size(&v);
        vms.push(MaintenanceVm {
            name,
            namespace,
            cpu,
            memory_gib,
            live_migratable: migratable,
            not_migratable_reason: reason,
            eviction_strategy: v
                .pointer("/spec/evictionStrategy")
                .and_then(|s| s.as_str())
                .map(str::to_string),
            rwo_only_volumes,
            pinned_devices: vmi_has_pinned_devices(&v),
        });
    }

    Json(plan_drain(&target, &nodes, &vms)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_size_claims_devices_and_migratability_from_a_vmi() {
        let vmi = json!({
            "spec": {
                "domain": {
                    "cpu": {"cores": 2, "sockets": 2},
                    "memory": {"guest": "4Gi"},
                    "devices": {"gpus": [{"name": "g"}], "interfaces": [{"name": "default"}]}
                },
                "volumes": [
                    {"name": "a", "persistentVolumeClaim": {"claimName": "data"}},
                    {"name": "b", "dataVolume": {"name": "root"}},
                    {"name": "c", "containerDisk": {"image": "x"}}
                ]
            },
            "status": {"conditions": [
                {"type": "Ready", "status": "True"},
                {"type": "LiveMigratable", "status": "False", "reason": "DisksNotLiveMigratable", "message": "RWO"}
            ]}
        });
        assert_eq!(vmi_size(&vmi), (4.0, 4.0));
        assert_eq!(vmi_claims(&vmi), vec!["data", "root"]);
        assert!(vmi_has_pinned_devices(&vmi));
        let (ok, why) = live_migratable(&vmi);
        assert_eq!(ok, Some(false));
        assert_eq!(why.as_deref(), Some("DisksNotLiveMigratable: RWO"));
        assert_eq!(live_migratable(&json!({"status": {}})), (None, None));
        assert!(!vmi_has_pinned_devices(
            &json!({"spec": {"domain": {"devices": {
            "interfaces": [{"name": "d", "masquerade": {}}]}}}})
        ));
        assert!(vmi_has_pinned_devices(
            &json!({"spec": {"domain": {"devices": {
            "interfaces": [{"name": "d", "sriov": {}}]}}}})
        ));
    }
}
