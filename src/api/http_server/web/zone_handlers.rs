// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Zones grouped by the real, standard Kubernetes node label
//! `topology.kubernetes.io/zone` -- the same label Phase 5's Placement
//! Advisor already treats as real (`PlacementAdvisor::zone_label`). No
//! multi-cluster/multi-datacenter federation concept invented here, just a
//! real grouping of real nodes.

use super::*;
use serde_json::json;

const ZONE_LABEL: &str = "topology.kubernetes.io/zone";

pub async fn list_zones_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let (nodes, _) = match super::placement_handlers::build_snapshot(&client, &namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "ZONES_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let mut zones: std::collections::BTreeMap<String, (u64, f64, f64, usize)> =
        std::collections::BTreeMap::new();
    for n in &nodes {
        let zone = n
            .labels
            .get(ZONE_LABEL)
            .cloned()
            .unwrap_or_else(|| "unassigned".to_string());
        let entry = zones.entry(zone).or_insert((0, 0.0, 0.0, 0));
        entry.0 += 1;
        entry.1 += n.allocatable_cpu;
        entry.2 += n.allocatable_memory_gib;
        entry.3 += n.vm_count;
    }

    let out: Vec<_> = zones
        .into_iter()
        .map(|(zone, (node_count, cpu, mem, vm_count))| {
            json!({
                "zone": zone,
                "node_count": node_count,
                "allocatable_cpu": cpu,
                "allocatable_memory_gib": mem,
                "vm_count": vm_count,
            })
        })
        .collect();

    Json(out).into_response()
}
