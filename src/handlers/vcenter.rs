// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Live-cluster handlers for Zorvia's vCenter-style operations surface.
use crate::kube::{
    KubeClient, VirtualMachine, VirtualMachineInstance, VirtualMachineInstanceMigration,
    VirtualMachineInstanceMigrationSpec,
};
use crate::vcenter_ops::{
    activity::{filter_and_limit, ActivityFilter, ActivityKind, ActivityRecord, ActivitySeverity},
    inventory::{
        extract_prefixed, InventoryBuilder, InventoryFilter, InventoryVm, ATTRIBUTE_PREFIX,
        CLUSTER_LABEL, DATACENTER_LABEL, FOLDER_LABEL, TAG_PREFIX,
    },
    maintenance::{MaintenanceAction, MaintenancePlanner, MaintenancePolicy, MaintenanceWorkload},
    metadata::{merge_patch, MetadataKind},
};
use anyhow::{anyhow, Context, Result};
use k8s_openapi::api::core::v1::{Event, Node};
use kube::{
    api::{ListParams, Patch, PatchParams, PostParams},
    Api, ResourceExt,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

async fn client(kubeconfig: Option<&str>) -> Result<KubeClient> {
    match kubeconfig {
        Some(p) => KubeClient::with_kubeconfig(p)
            .await
            .with_context(|| format!("failed to load kubeconfig '{}'", p)),
        None => KubeClient::new()
            .await
            .context("failed to create Kubernetes client"),
    }
}

pub async fn handle_inventory(
    all_namespaces: bool,
    output: String,
    dc: Option<String>,
    cluster: Option<String>,
    folder: Option<String>,
    namespace: &str,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let c = client(kubeconfig).await?;
    let vms = if all_namespaces {
        c.list_all_vms().await?
    } else {
        c.list_vms(namespace).await?
    };
    let mut rows = Vec::new();
    for vm in vms {
        let name = vm.name_any();
        let ns = vm.namespace().unwrap_or_else(|| namespace.to_string());
        let labels = vm.labels().clone();
        let annotations = vm.annotations().clone();
        let vmi = c.get_vmi(&ns, &name).await.ok();
        let host = vmi
            .as_ref()
            .and_then(|v| v.status.as_ref())
            .and_then(|s| s.node_name.clone());
        let guest_os = vmi
            .as_ref()
            .and_then(|v| v.status.as_ref())
            .and_then(|s| s.guest_os_info.as_ref())
            .and_then(|g| g.name.clone());
        let power_state = vm
            .status
            .as_ref()
            .and_then(|s| s.printable_status.clone())
            .unwrap_or_else(|| {
                if vm.spec.running.unwrap_or(false) {
                    "Starting".into()
                } else {
                    "Stopped".into()
                }
            });
        rows.push(InventoryVm {
            name,
            namespace: ns,
            datacenter: labels
                .get(DATACENTER_LABEL)
                .cloned()
                .unwrap_or_else(|| "default-dc".into()),
            cluster: labels
                .get(CLUSTER_LABEL)
                .cloned()
                .unwrap_or_else(|| "default-cluster".into()),
            folder: labels
                .get(FOLDER_LABEL)
                .cloned()
                .unwrap_or_else(|| "root".into()),
            host,
            power_state,
            guest_os,
            tags: extract_prefixed(&labels, TAG_PREFIX),
            attributes: extract_prefixed(&annotations, ATTRIBUTE_PREFIX),
        });
    }
    let tree = InventoryBuilder::build(
        rows,
        &InventoryFilter {
            datacenter: dc,
            cluster,
            folder,
        },
    );
    match output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&tree)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(&tree)?),
        _ => render_inventory(&tree),
    }
    Ok(())
}
fn render_inventory(tree: &crate::vcenter_ops::inventory::InventoryTree) {
    println!(
        "Zorvia Inventory  VMs {} (running {})  Hosts {}",
        tree.total_vms, tree.running_vms, tree.hosts
    );
    for dc in &tree.datacenters {
        println!("▾ Datacenter {} [{} VMs]", dc.name, dc.vm_count);
        for cl in &dc.clusters {
            println!("  ▾ Cluster {} [{} VMs]", cl.name, cl.vm_count);
            for h in &cl.hosts {
                println!(
                    "    Host {:<28} {:>4} VMs / {:>4} running",
                    h.name, h.vm_count, h.running_vm_count
                );
            }
            for f in &cl.folders {
                println!("    ▾ Folder {}", f.name);
                for vm in &f.vms {
                    println!(
                        "      {:<34} {:<14} {:<18} {}",
                        format!("{}/{}", vm.namespace, vm.name),
                        vm.power_state,
                        vm.host.as_deref().unwrap_or("-"),
                        vm.guest_os.as_deref().unwrap_or("-")
                    );
                }
            }
        }
    }
}

pub async fn handle_vm_metadata(
    vm: String,
    namespace: &str,
    kind: MetadataKind,
    key: String,
    value: Option<String>,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let c = client(kubeconfig).await?;
    let api: Api<VirtualMachine> = Api::namespaced(c.client(), namespace);
    api.get(&vm)
        .await
        .with_context(|| format!("VM {}/{} not found", namespace, vm))?;
    let patch = merge_patch(kind, &key, value.as_deref())?;
    api.patch(&vm, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    println!("updated metadata on {}/{}", namespace, vm);
    Ok(())
}

/// Query parameters for [`handle_activity`], grouped to keep the function's
/// argument count within clippy's `too_many_arguments` limit.
pub struct ActivityQueryArgs {
    pub all_namespaces: bool,
    pub target: Option<String>,
    pub severity: Option<String>,
    pub kind: Option<String>,
    pub limit: usize,
    pub output: String,
}

pub async fn handle_activity(
    args: ActivityQueryArgs,
    namespace: &str,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let ActivityQueryArgs {
        all_namespaces,
        target,
        severity,
        kind,
        limit,
        output,
    } = args;
    let c = client(kubeconfig).await?;
    let events = if all_namespaces {
        Api::<Event>::all(c.client())
            .list(&ListParams::default())
            .await?
    } else {
        Api::<Event>::namespaced(c.client(), namespace)
            .list(&ListParams::default())
            .await?
    };
    let mut rows = Vec::new();
    for e in events {
        let (k, s) = ActivityRecord::classify(e.type_.as_deref(), e.reason.as_deref());
        let ts = e
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.to_string())
            .unwrap_or_default();
        rows.push(ActivityRecord {
            timestamp: ts,
            namespace: e.namespace().unwrap_or_default(),
            kind: k,
            severity: s,
            reason: e.reason.unwrap_or_else(|| "Event".into()),
            resource_kind: e.involved_object.kind.unwrap_or_else(|| "Object".into()),
            resource_name: e.involved_object.name.unwrap_or_default(),
            message: e.message.unwrap_or_default(),
            count: e.count.unwrap_or(1),
        });
    }
    let sev = match severity.as_deref() {
        Some("warning") => Some(ActivitySeverity::Warning),
        Some("error") => Some(ActivitySeverity::Error),
        Some("info") => Some(ActivitySeverity::Info),
        Some(v) => return Err(anyhow!("invalid severity '{}'", v)),
        None => None,
    };
    let k = match kind.as_deref() {
        Some("task") => Some(ActivityKind::Task),
        Some("event") => Some(ActivityKind::Event),
        Some("alarm") => Some(ActivityKind::Alarm),
        Some(v) => return Err(anyhow!("invalid activity kind '{}'", v)),
        None => None,
    };
    let rows = filter_and_limit(
        rows,
        &ActivityFilter {
            target,
            min_severity: sev,
            kind: k,
        },
        limit,
    );
    match output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&rows)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(&rows)?),
        _ => {
            println!(
                "{:<25} {:<7} {:<10} {:<22} {:<28} MESSAGE",
                "TIME", "LEVEL", "KIND", "REASON", "RESOURCE"
            );
            for r in rows {
                println!(
                    "{:<25} {:<7?} {:<10?} {:<22} {:<28} {}",
                    r.timestamp,
                    r.severity,
                    r.kind,
                    r.reason,
                    format!("{}/{}", r.resource_kind, r.resource_name),
                    r.message
                );
            }
        }
    }
    Ok(())
}

async fn live_maintenance_plan(
    c: &KubeClient,
    node: &str,
    max_parallel: usize,
) -> Result<crate::vcenter_ops::maintenance::MaintenancePlan> {
    let vms = c.list_all_vms().await?;
    let mut policies = HashMap::new();
    for vm in vms {
        let ns = vm.namespace().unwrap_or_default();
        let name = vm.name_any();
        let a = vm.annotations();
        let policy =
            MaintenancePolicy::parse(a.get("maintenance.zorvia.io/policy").map(String::as_str));
        let priority = a
            .get("maintenance.zorvia.io/priority")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(50);
        policies.insert((ns, name), (policy, priority));
    }
    let vmis = Api::<VirtualMachineInstance>::all(c.client())
        .list(&ListParams::default())
        .await?;
    let mut workloads = Vec::new();
    for vmi in vmis {
        if vmi.status.as_ref().and_then(|s| s.node_name.as_deref()) != Some(node) {
            continue;
        }
        let ns = vmi.namespace().unwrap_or_default();
        let name = vmi.name_any();
        let (policy, priority) = policies
            .get(&(ns.clone(), name.clone()))
            .copied()
            .unwrap_or((MaintenancePolicy::LiveMigrate, 50));
        workloads.push(MaintenanceWorkload {
            name,
            namespace: ns,
            node: node.into(),
            policy,
            priority,
        });
    }
    Ok(MaintenancePlanner::new(max_parallel).plan(node, workloads))
}

pub async fn handle_maintenance_plan(
    node: String,
    max_parallel: usize,
    output: String,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let c = client(kubeconfig).await?;
    let p = live_maintenance_plan(&c, &node, max_parallel).await?;
    render_maintenance(&p, &output)
}
pub async fn handle_maintenance_enter(
    node: String,
    max_parallel: usize,
    dry_run: bool,
    force: bool,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let c = client(kubeconfig).await?;
    let p = live_maintenance_plan(&c, &node, max_parallel).await?;
    if dry_run {
        return render_maintenance(&p, "table");
    }
    if !p.can_enter() && !force {
        return Err(anyhow!("maintenance blocked: {}", p.blockers.join("; ")));
    }
    let nodes: Api<Node> = Api::all(c.client());
    nodes
        .patch(
            &node,
            &PatchParams::default(),
            &Patch::Merge(&serde_json::json!({"spec":{"unschedulable":true}})),
        )
        .await?;
    for step in &p.steps {
        match step.action {
            MaintenanceAction::Migrate => {
                let api: Api<VirtualMachineInstanceMigration> =
                    Api::namespaced(c.client(), &step.workload.namespace);
                let n = format!(
                    "zorvia-maint-{}-{}",
                    sanitize(&step.workload.name),
                    chrono::Utc::now().timestamp_millis()
                );
                let m = VirtualMachineInstanceMigration::new(
                    &n,
                    VirtualMachineInstanceMigrationSpec {
                        vmi_name: Some(step.workload.name.clone()),
                    },
                );
                api.create(&PostParams::default(), &m).await?;
            }
            MaintenanceAction::Stop => {
                c.stop_vm(&step.workload.namespace, &step.workload.name)
                    .await?;
            }
            MaintenanceAction::Skip | MaintenanceAction::Block => {
                if force {
                    c.stop_vm(&step.workload.namespace, &step.workload.name)
                        .await?;
                } else {
                    return Err(anyhow!(
                        "workload {} prevents complete evacuation",
                        step.workload.name
                    ));
                }
            }
        }
    }
    println!(
        "node {} cordoned; {} workload actions submitted",
        node,
        p.steps.len()
    );
    Ok(())
}
pub async fn handle_maintenance_exit(node: String, kubeconfig: Option<&str>) -> Result<()> {
    let c = client(kubeconfig).await?;
    let nodes: Api<Node> = Api::all(c.client());
    nodes
        .patch(
            &node,
            &PatchParams::default(),
            &Patch::Merge(&serde_json::json!({"spec":{"unschedulable":false}})),
        )
        .await?;
    println!("node {} left maintenance mode", node);
    Ok(())
}
pub async fn handle_maintenance_status(node: String, kubeconfig: Option<&str>) -> Result<()> {
    let c = client(kubeconfig).await?;
    let n = Api::<Node>::all(c.client()).get(&node).await?;
    let p = live_maintenance_plan(&c, &node, 2).await?;
    println!(
        "Node: {}\nMaintenance: {}\nRemaining VMIs: {}\nBlockers: {}",
        node,
        n.spec.and_then(|s| s.unschedulable).unwrap_or(false),
        p.vm_count(),
        p.blockers.len()
    );
    Ok(())
}
fn render_maintenance(
    p: &crate::vcenter_ops::maintenance::MaintenancePlan,
    output: &str,
) -> Result<()> {
    match output {
        "json" => println!("{}", serde_json::to_string_pretty(p)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(p)?),
        _ => {
            println!(
                "Maintenance plan: {}  VMs: {}  batches: {}  blockers: {}  est: {}s",
                p.node,
                p.vm_count(),
                p.migration_batches.len(),
                p.blockers.len(),
                p.estimated_seconds
            );
            for s in &p.steps {
                println!(
                    "  {:<10?} {:<35} {}",
                    s.action,
                    format!("{}/{}", s.workload.namespace, s.workload.name),
                    s.reason
                );
            }
            for b in &p.blockers {
                println!("  BLOCKER: {}", b);
            }
        }
    }
    Ok(())
}
fn sanitize(v: &str) -> String {
    v.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

// Placement live-snapshot helpers are kept here to avoid another network client layer.
pub async fn handle_placement_advisor(
    cpu: f64,
    memory_gib: f64,
    required_label: Vec<String>,
    avoid_node: Vec<String>,
    preferred_zone: Vec<String>,
    output: String,
    kubeconfig: Option<&str>,
) -> Result<()> {
    use crate::placement::advisor::{PlacementAdvisor, WorkloadRequirements};
    let c = client(kubeconfig).await?;
    let (nodes, _) = placement_snapshot(&c).await?;
    let labels = parse_pairs(required_label)?;
    let w = WorkloadRequirements {
        name: "candidate".into(),
        namespace: "default".into(),
        cpu,
        memory_gib,
        current_node: None,
        required_labels: labels,
        avoid_nodes: avoid_node.into_iter().collect(),
        preferred_zones: preferred_zone.into_iter().collect(),
    };
    let r = PlacementAdvisor::default().recommend(&w, &nodes);
    match output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&r)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(&r)?),
        _ => {
            println!(
                "Selected node: {}",
                r.selected_node.as_deref().unwrap_or("NONE")
            );
            for c in r.candidates {
                println!(
                    "  {:<28} eligible={:<5} score={:>6.2}  {}",
                    c.node,
                    c.eligible,
                    c.score,
                    c.reasons.join("; ")
                );
            }
        }
    }
    Ok(())
}
pub async fn handle_placement_rebalance(
    threshold: f64,
    max_migrations: usize,
    output: String,
    kubeconfig: Option<&str>,
) -> Result<()> {
    use crate::placement::advisor::PlacementAdvisor;
    let c = client(kubeconfig).await?;
    let (nodes, workloads) = placement_snapshot(&c).await?;
    let r = PlacementAdvisor::default().rebalance(&workloads, &nodes, threshold, max_migrations);
    match output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&r)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(&r)?),
        _ => {
            println!("Rebalance recommendations: {}", r.len());
            for m in r {
                println!(
                    "  {}/{}: {} -> {}  improvement {:.1}%",
                    m.namespace,
                    m.workload,
                    m.from_node,
                    m.to_node,
                    m.expected_improvement * 100.0
                );
            }
        }
    }
    Ok(())
}
async fn placement_snapshot(
    c: &KubeClient,
) -> Result<(
    Vec<crate::placement::advisor::NodeSnapshot>,
    Vec<crate::placement::advisor::WorkloadRequirements>,
)> {
    use crate::placement::advisor::{NodeSnapshot, WorkloadRequirements};
    let nodes_api = Api::<Node>::all(c.client());
    let nodes = nodes_api.list(&ListParams::default()).await?;
    let vmis = Api::<VirtualMachineInstance>::all(c.client())
        .list(&ListParams::default())
        .await?;
    let mut usage: HashMap<String, (f64, f64, usize)> = HashMap::new();
    let mut workloads = Vec::new();
    for v in &vmis {
        let Some(node) = v.status.as_ref().and_then(|s| s.node_name.clone()) else {
            continue;
        };
        let (cpu, mem) = vmi_resources(v);
        let e = usage.entry(node.clone()).or_default();
        e.0 += cpu;
        e.1 += mem;
        e.2 += 1;
        workloads.push(WorkloadRequirements {
            name: v.name_any(),
            namespace: v.namespace().unwrap_or_default(),
            cpu,
            memory_gib: mem,
            current_node: Some(node),
            required_labels: BTreeMap::new(),
            avoid_nodes: BTreeSet::new(),
            preferred_zones: BTreeSet::new(),
        });
    }
    let mut out = Vec::new();
    for n in nodes {
        let name = n.name_any();
        let (uc, um, count) = usage.get(&name).copied().unwrap_or_default();
        let alloc = n.status.as_ref().and_then(|s| s.allocatable.as_ref());
        let cpu = alloc
            .and_then(|m| m.get("cpu"))
            .map(|q| parse_cpu(&q.0))
            .unwrap_or(0.0);
        let mem = alloc
            .and_then(|m| m.get("memory"))
            .map(|q| parse_memory_gib(&q.0))
            .unwrap_or(0.0);
        let labels = n.labels().clone();
        let taints = n
            .spec
            .as_ref()
            .and_then(|s| s.taints.as_ref())
            .map(|ts| {
                ts.iter()
                    .map(|t| {
                        format!(
                            "{}={}:{}",
                            t.key,
                            t.value.as_deref().unwrap_or(""),
                            t.effect
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let unschedulable = n
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false);
        out.push(NodeSnapshot {
            name,
            allocatable_cpu: cpu,
            allocatable_memory_gib: mem,
            used_cpu: uc,
            used_memory_gib: um,
            vm_count: count,
            labels,
            taints,
            unschedulable,
        });
    }
    Ok((out, workloads))
}
fn vmi_resources(v: &VirtualMachineInstance) -> (f64, f64) {
    let x = serde_json::to_value(&v.spec).unwrap_or_default();
    let cpu = x
        .pointer("/domain/resources/requests/cpu")
        .and_then(|v| v.as_str())
        .map(parse_cpu)
        .or_else(|| {
            let c = x
                .pointer("/domain/cpu/cores")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let s = x
                .pointer("/domain/cpu/sockets")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let t = x
                .pointer("/domain/cpu/threads")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            Some(c * s * t)
        })
        .unwrap_or(1.0);
    let mem = x
        .pointer("/domain/resources/requests/memory")
        .and_then(|v| v.as_str())
        .or_else(|| x.pointer("/domain/memory/guest").and_then(|v| v.as_str()))
        .map(parse_memory_gib)
        .unwrap_or(0.0);
    (cpu, mem)
}
fn parse_cpu(v: &str) -> f64 {
    v.strip_suffix('m')
        .and_then(|x| x.parse::<f64>().ok())
        .map(|x| x / 1000.0)
        .or_else(|| v.parse().ok())
        .unwrap_or(0.0)
}
fn parse_memory_gib(v: &str) -> f64 {
    let v = v.trim();
    for (s, m) in [
        ("Ki", 1.0 / 1048576.0),
        ("Mi", 1.0 / 1024.0),
        ("Gi", 1.0),
        ("Ti", 1024.0),
        ("K", 1000.0 / 1073741824.0),
        ("M", 1e6 / 1073741824.0),
        ("G", 1e9 / 1073741824.0),
    ] {
        if let Some(n) = v.strip_suffix(s) {
            return n.parse::<f64>().unwrap_or(0.0) * m;
        }
    }
    v.parse::<f64>().unwrap_or(0.0) / 1073741824.0
}
fn parse_pairs(values: Vec<String>) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for raw in values {
        let (k, v) = raw
            .split_once('=')
            .ok_or_else(|| anyhow!("expected key=value, got '{}'", raw))?;
        out.insert(k.into(), v.into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantity_parsers() {
        assert_eq!(parse_cpu("2500m"), 2.5);
        assert_eq!(parse_cpu("4"), 4.0);
        assert!((parse_memory_gib("4Gi") - 4.0).abs() < 1e-9);
        assert!((parse_memory_gib("1024Mi") - 1.0).abs() < 1e-9);
    }
    #[test]
    fn pair_parser_rejects_bad_input() {
        assert!(parse_pairs(vec!["bad".into()]).is_err());
    }
}
