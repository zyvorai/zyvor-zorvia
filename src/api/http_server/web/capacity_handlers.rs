// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Cluster capacity, built from the exact same real `NodeSnapshot`/
//! `WorkloadRequirements` data Phase 5's Placement Advisor already computes
//! (`super::placement_handlers::build_snapshot`) -- real `Node` allocatable
//! capacity minus real per-node VM usage, zero new K8s wiring.

use super::*;
use serde_json::json;

pub async fn capacity_overview_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let (nodes, workloads) =
        match super::placement_handlers::build_snapshot(&client, &namespace).await {
            Ok(v) => v,
            Err(e) => {
                let (st, j) = err_json(500, "CAPACITY_FAILED", &sanitize_error(&e));
                return (st, j).into_response();
            }
        };

    let total_cpu: f64 = nodes.iter().map(|n| n.allocatable_cpu).sum();
    let total_memory_gib: f64 = nodes.iter().map(|n| n.allocatable_memory_gib).sum();
    let used_cpu: f64 = nodes.iter().map(|n| n.used_cpu).sum();
    let used_memory_gib: f64 = nodes.iter().map(|n| n.used_memory_gib).sum();

    let per_node: Vec<_> = nodes
        .iter()
        .map(|n| {
            json!({
                "name": n.name,
                "allocatable_cpu": n.allocatable_cpu,
                "allocatable_memory_gib": n.allocatable_memory_gib,
                "used_cpu": n.used_cpu,
                "used_memory_gib": n.used_memory_gib,
                "vm_count": n.vm_count,
                "unschedulable": n.unschedulable,
            })
        })
        .collect();

    Json(json!({
        "node_count": nodes.len(),
        "vm_count": workloads.len(),
        "total_cpu": total_cpu,
        "total_memory_gib": total_memory_gib,
        "used_cpu": used_cpu,
        "used_memory_gib": used_memory_gib,
        "free_cpu": (total_cpu - used_cpu).max(0.0),
        "free_memory_gib": (total_memory_gib - used_memory_gib).max(0.0),
        "nodes": per_node,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CapacityFitQuery {
    #[serde(default = "default_fit_cpu")]
    pub cpu: f64,
    #[serde(default = "default_fit_memory")]
    pub memory_gib: f64,
}
fn default_fit_cpu() -> f64 {
    2.0
}
fn default_fit_memory() -> f64 {
    4.0
}

/// How many more VMs of a given size the cluster could schedule -- a real
/// per-node bin-packing estimate (free capacity / requested size, summed
/// across nodes), not a scheduling guarantee: it doesn't account for taints,
/// affinity, or other pods competing for the same headroom concurrently.
pub async fn capacity_fit_handler(
    State(state): State<SharedState>,
    Query(q): Query<CapacityFitQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let (nodes, _) = match super::placement_handlers::build_snapshot(&client, &namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "CAPACITY_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let cpu = q.cpu.max(0.01);
    let mem = q.memory_gib.max(0.01);
    let fits: u64 = nodes
        .iter()
        .filter(|n| !n.unschedulable)
        .map(|n| {
            let by_cpu = (n.free_cpu() / cpu).floor();
            let by_mem = (n.free_memory_gib() / mem).floor();
            by_cpu.min(by_mem).max(0.0) as u64
        })
        .sum();

    Json(json!({
        "requested_cpu": cpu,
        "requested_memory_gib": mem,
        "estimated_additional_vms": fits,
    }))
    .into_response()
}
