// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Live adapters for the autoscaler reconcile loop (cluster access; not unit-tested).
//!
//! NOT verified against a live cluster. The load source wraps the repo's existing
//! `MonitoringLoadSource` -> `MetricsCollector`, whose usage numbers are currently
//! *simulated* (real allocations, random usage), so samples are flagged `simulated`
//! and the reconciler never applies them.

use super::reconcile::{LoadSample, LoadSource, LoopTarget, ScaleExecutor, TargetKind};
use crate::kube::KubeClient;
use crate::monitoring::metrics::MetricsCollector;
use anyhow::{anyhow, Result};
use kube::api::{Api, DynamicObject, Patch, PatchParams};
use kube::core::{ApiResource, GroupVersionKind};

pub struct MonitoringLoadSource;

impl LoadSource for MonitoringLoadSource {
    async fn load(&self, t: &LoopTarget) -> Result<LoadSample> {
        let vms: Vec<String> = if t.metric_vms.is_empty() {
            vec![t.name.clone()]
        } else {
            t.metric_vms.clone()
        };
        let collected = MetricsCollector::new(t.namespace.clone())
            .collect_multiple(&vms)
            .await?;
        if collected.is_empty() {
            return Err(anyhow!("no metrics"));
        }
        let sum: f64 = collected.iter().map(|(_, m)| m.cpu.usage_percent).sum();
        Ok(LoadSample {
            percent: sum / collected.len() as f64,
            // MetricsCollector fabricates usage today; flip to false only for a real source.
            simulated: true,
            source: "simulated",
        })
    }
}

pub struct KubeExecutor {
    pub kube: KubeClient,
}

fn pool_api(kube: &KubeClient, ns: &str) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk("pool.kubevirt.io", "v1alpha1", "VirtualMachinePool");
    let ar = ApiResource::from_gvk_with_plural(&gvk, "virtualmachinepools");
    Api::namespaced_with(kube.client(), ns, &ar)
}

impl ScaleExecutor for KubeExecutor {
    async fn current(&self, t: &LoopTarget) -> Result<u32> {
        match t.kind {
            TargetKind::VmPool => {
                let pool = pool_api(&self.kube, &t.namespace).get(&t.name).await?;
                pool.data
                    .pointer("/spec/replicas")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .ok_or_else(|| anyhow!("pool has no spec.replicas"))
            }
            TargetKind::VmCpu => {
                let vm = self.kube.get_vm(&t.namespace, &t.name).await?;
                let cpu = vm
                    .spec
                    .template
                    .spec
                    .domain
                    .cpu
                    .as_ref()
                    .ok_or_else(|| anyhow!("VM has no CPU topology"))?;
                Ok(cpu.sockets.unwrap_or(1).max(1)
                    * cpu.cores.unwrap_or(1).max(1)
                    * cpu.threads.unwrap_or(1).max(1))
            }
        }
    }

    async fn scale(&self, t: &LoopTarget, to: u32) -> Result<()> {
        match t.kind {
            TargetKind::VmPool => {
                let patch = serde_json::json!({"spec": {"replicas": to}});
                pool_api(&self.kube, &t.namespace)
                    .patch(&t.name, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                Ok(())
            }
            TargetKind::VmCpu => {
                self.kube.hotplug_cpu(&t.namespace, &t.name, to).await?;
                Ok(())
            }
        }
    }
}
