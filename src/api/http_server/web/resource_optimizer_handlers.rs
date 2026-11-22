// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Right-sizing and idle-VM recommendations, backed by the real
//! `crate::cost::optimization::OptimizationEngine` -- pure functions (like
//! Phase 5's `PlacementAdvisor`) fed real VM specs and real usage numbers
//! (`quick_vm_usage`, shared with `fabric_vm_handlers.rs`). Had zero callers
//! anywhere in the codebase before this.
//!
//! There is no real cloud billing API on this platform (private/on-prem
//! KubeVirt), so `monthly_cost` is an estimate from allocated CPU/memory at
//! a configurable $/core-hour and $/GB-hour rate -- labeled as an estimate
//! everywhere it surfaces, never presented as billing data.

use super::*;
use crate::cost::optimization::OptimizationEngine;
use serde_json::json;

fn cpu_rate() -> f64 {
    std::env::var("ZORVIA_COST_CPU_PER_HOUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.02)
}
fn memory_rate() -> f64 {
    std::env::var("ZORVIA_COST_MEMORY_GB_PER_HOUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.005)
}

fn estimated_monthly_cost(cpu_cores: f64, memory_gib: f64) -> f64 {
    let hours_per_month = 24.0 * 30.0;
    (cpu_cores * cpu_rate() + memory_gib * memory_rate()) * hours_per_month
}

fn vm_cpu_cores(vm: &crate::kube::types::VirtualMachine) -> u32 {
    vm.spec
        .template
        .spec
        .domain
        .cpu
        .as_ref()
        .map(|c| c.cores.unwrap_or(1) * c.sockets.unwrap_or(1) * c.threads.unwrap_or(1))
        .unwrap_or(1)
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

pub async fn resource_optimizer_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vms = match client.list_vms(&namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "OPTIMIZER_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let mut recommendations = Vec::new();
    for vm in &vms {
        let name = vm.metadata.name.clone().unwrap_or_default();
        let cpu_cores = vm_cpu_cores(vm);
        let memory_gib = vm_memory_gib(vm);
        let cost = estimated_monthly_cost(cpu_cores as f64, memory_gib);

        let (cpu_usage, memory_usage, _disk_usage, _source, gm) =
            super::fabric_vm_handlers::quick_vm_usage(&client, &namespace, &name).await;
        let memory_usage_percent = if gm.memory_available > 0 {
            (memory_usage as f64 / gm.memory_available as f64 * 100.0).clamp(0.0, 100.0)
        } else if memory_gib > 0.0 {
            (memory_usage as f64 / (memory_gib * 1024.0 * 1024.0 * 1024.0) * 100.0)
                .clamp(0.0, 100.0)
        } else {
            0.0
        };

        if let Some(rec) = OptimizationEngine::analyze_right_sizing(
            &name,
            cpu_cores,
            memory_gib.round().max(1.0) as u32,
            cpu_usage,
            memory_usage_percent,
            cost,
        ) {
            recommendations.push(rec);
        }

        let days_running = vm
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| {
                let age_secs =
                    (k8s_openapi::jiff::Timestamp::now().as_second() - t.0.as_second()).max(0);
                (age_secs / 86_400) as u32
            })
            .unwrap_or(0);
        if let Some(rec) = OptimizationEngine::detect_idle_vm(&name, cpu_usage, days_running, cost)
        {
            recommendations.push(rec);
        }
    }

    Json(json!({
        "recommendations": recommendations,
        "cost_basis": {
            "cpu_per_core_hour": cpu_rate(),
            "memory_per_gb_hour": memory_rate(),
            "note": "estimated from allocated resources, not real billing data",
        },
    }))
    .into_response()
}
