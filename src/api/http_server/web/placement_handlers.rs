// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Placement recommendations backed by `crate::placement::advisor::PlacementAdvisor`
//! -- a real, tested scoring algorithm with zero callers anywhere in the
//! codebase until now. Recommendation-only by design (matches the module's
//! own doc comment): it never moves a VM itself, it just tells an operator
//! which node a migration should target, using the real live-migration path
//! (`POST /vms/:name/migrate`) that already exists.

use super::*;
use crate::placement::advisor::{NodeSnapshot, PlacementAdvisor, WorkloadRequirements};
use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};
use std::collections::{BTreeMap, BTreeSet};

fn parse_cpu_cores(q: &str) -> f64 {
    let q = q.trim();
    if let Some(m) = q.strip_suffix('m') {
        m.parse::<f64>().unwrap_or(0.0) / 1000.0
    } else {
        q.parse::<f64>().unwrap_or(0.0)
    }
}

fn vm_cpu_cores(vm: &crate::kube::types::VirtualMachine) -> f64 {
    vm.spec
        .template
        .spec
        .domain
        .cpu
        .as_ref()
        .map(|c| (c.cores.unwrap_or(1) * c.sockets.unwrap_or(1) * c.threads.unwrap_or(1)) as f64)
        .unwrap_or(1.0)
}

fn vm_memory_gib(vm: &crate::kube::types::VirtualMachine) -> f64 {
    vm.spec
        .template
        .spec
        .domain
        .resources
        .requests
        .as_ref()
        .and_then(|r| r.get("memory"))
        .map(|q| parse_memory(q) as f64 / (1024.0 * 1024.0 * 1024.0))
        .unwrap_or(0.0)
}

/// Builds the real node/workload snapshot the advisor scores against: real
/// `Node` allocatable capacity + real per-node used CPU/memory computed by
/// summing the VMs actually scheduled there (via each VM's VMI status).
pub(crate) async fn build_snapshot(
    client: &crate::kube::KubeClient,
    namespace: &str,
) -> anyhow::Result<(Vec<NodeSnapshot>, Vec<WorkloadRequirements>)> {
    let raw = client.client();
    let nodes_api: Api<Node> = Api::all(raw);
    let node_list = nodes_api.list(&ListParams::default()).await?;

    let vms = client.list_vms(namespace).await?;

    // node name -> (used_cpu, used_memory_gib, vm_count)
    let mut usage: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();
    let mut workloads = Vec::with_capacity(vms.len());
    for vm in &vms {
        let name = vm.metadata.name.clone().unwrap_or_default();
        let current_node = client.get_vm_node(namespace, &name).await.ok().flatten();
        let cpu = vm_cpu_cores(vm);
        let mem = vm_memory_gib(vm);
        if let Some(node) = &current_node {
            let entry = usage.entry(node.clone()).or_insert((0.0, 0.0, 0));
            entry.0 += cpu;
            entry.1 += mem;
            entry.2 += 1;
        }
        workloads.push(WorkloadRequirements {
            name,
            namespace: namespace.to_string(),
            cpu,
            memory_gib: mem,
            current_node,
            required_labels: BTreeMap::new(),
            avoid_nodes: BTreeSet::new(),
            preferred_zones: BTreeSet::new(),
        });
    }

    let nodes = node_list
        .items
        .into_iter()
        .map(|n| {
            let name = n.metadata.name.clone().unwrap_or_default();
            let allocatable = n.status.as_ref().and_then(|s| s.allocatable.as_ref());
            let allocatable_cpu = allocatable
                .and_then(|a| a.get("cpu"))
                .map(|q| parse_cpu_cores(&q.0))
                .unwrap_or(0.0);
            let allocatable_memory_gib = allocatable
                .and_then(|a| a.get("memory"))
                .map(|q| parse_memory(&q.0) as f64 / (1024.0 * 1024.0 * 1024.0))
                .unwrap_or(0.0);
            let unschedulable = n
                .spec
                .as_ref()
                .and_then(|s| s.unschedulable)
                .unwrap_or(false);
            let labels = n
                .metadata
                .labels
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let taints = n
                .spec
                .as_ref()
                .and_then(|s| s.taints.as_ref())
                .map(|ts| ts.iter().map(|t| t.key.clone()).collect::<BTreeSet<_>>())
                .unwrap_or_default();
            let (used_cpu, used_memory_gib, vm_count) =
                usage.get(&name).copied().unwrap_or((0.0, 0.0, 0));
            NodeSnapshot {
                name,
                allocatable_cpu,
                allocatable_memory_gib,
                used_cpu,
                used_memory_gib,
                vm_count,
                labels,
                taints,
                unschedulable,
            }
        })
        .collect();

    Ok((nodes, workloads))
}

pub async fn placement_recommend_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let (nodes, workloads) = match build_snapshot(&client, &namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "PLACEMENT_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let Some(workload) = workloads.into_iter().find(|w| w.name == name) else {
        let (st, j) = err_json(404, "NOT_FOUND", &format!("VM '{name}' not found"));
        return (st, j).into_response();
    };

    let recommendation = PlacementAdvisor::default().recommend(&workload, &nodes);
    Json(recommendation).into_response()
}

#[derive(Debug, Deserialize)]
pub struct RebalanceQuery {
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub max_moves: Option<usize>,
}

pub async fn placement_rebalance_handler(
    State(state): State<SharedState>,
    Query(q): Query<RebalanceQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let (nodes, workloads) = match build_snapshot(&client, &namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "PLACEMENT_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let moves = PlacementAdvisor::default().rebalance(
        &workloads,
        &nodes,
        q.threshold.unwrap_or(0.15),
        q.max_moves.unwrap_or(5),
    );
    Json(moves).into_response()
}
