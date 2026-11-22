// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DATACENTER_LABEL: &str = "inventory.zorvia.io/datacenter";
pub const CLUSTER_LABEL: &str = "inventory.zorvia.io/cluster";
pub const FOLDER_LABEL: &str = "inventory.zorvia.io/folder";
pub const TAG_PREFIX: &str = "tag.zorvia.io/";
pub const ATTRIBUTE_PREFIX: &str = "attr.zorvia.io/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryVm {
    pub name: String,
    pub namespace: String,
    pub datacenter: String,
    pub cluster: String,
    pub folder: String,
    pub host: Option<String>,
    pub power_state: String,
    pub guest_os: Option<String>,
    pub tags: BTreeMap<String, String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryHost {
    pub name: String,
    pub vm_count: usize,
    pub running_vm_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryFolder {
    pub name: String,
    pub vms: Vec<InventoryVm>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryCluster {
    pub name: String,
    pub hosts: Vec<InventoryHost>,
    pub folders: Vec<InventoryFolder>,
    pub vm_count: usize,
    pub running_vm_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryDatacenter {
    pub name: String,
    pub clusters: Vec<InventoryCluster>,
    pub vm_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryTree {
    pub datacenters: Vec<InventoryDatacenter>,
    pub total_vms: usize,
    pub running_vms: usize,
    pub hosts: usize,
}

#[derive(Debug, Clone, Default)]
pub struct InventoryFilter {
    pub datacenter: Option<String>,
    pub cluster: Option<String>,
    pub folder: Option<String>,
}

impl InventoryFilter {
    pub fn matches(&self, vm: &InventoryVm) -> bool {
        self.datacenter.as_ref().is_none_or(|v| v == &vm.datacenter)
            && self.cluster.as_ref().is_none_or(|v| v == &vm.cluster)
            && self.folder.as_ref().is_none_or(|v| v == &vm.folder)
    }
}

pub fn extract_prefixed(
    values: &BTreeMap<String, String>,
    prefix: &str,
) -> BTreeMap<String, String> {
    values
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix(prefix)
                .map(|name| (name.to_string(), v.clone()))
        })
        .collect()
}

pub struct InventoryBuilder;

impl InventoryBuilder {
    pub fn build(
        vms: impl IntoIterator<Item = InventoryVm>,
        filter: &InventoryFilter,
    ) -> InventoryTree {
        let mut grouped: BTreeMap<String, BTreeMap<String, Vec<InventoryVm>>> = BTreeMap::new();
        for vm in vms.into_iter().filter(|vm| filter.matches(vm)) {
            grouped
                .entry(vm.datacenter.clone())
                .or_default()
                .entry(vm.cluster.clone())
                .or_default()
                .push(vm);
        }

        let mut datacenters = Vec::new();
        let mut total_vms = 0usize;
        let mut running_vms = 0usize;
        let mut all_hosts = BTreeSet::new();

        for (dc_name, clusters) in grouped {
            let mut dc_count = 0usize;
            let mut cluster_rows = Vec::new();
            for (cluster_name, mut cluster_vms) in clusters {
                cluster_vms.sort_by(|a, b| {
                    (&a.folder, &a.namespace, &a.name).cmp(&(&b.folder, &b.namespace, &b.name))
                });
                let vm_count = cluster_vms.len();
                let running = cluster_vms
                    .iter()
                    .filter(|v| is_running(&v.power_state))
                    .count();
                dc_count += vm_count;
                total_vms += vm_count;
                running_vms += running;

                let mut host_map: BTreeMap<String, (usize, usize)> = BTreeMap::new();
                let mut folder_map: BTreeMap<String, Vec<InventoryVm>> = BTreeMap::new();
                for vm in cluster_vms {
                    let host = vm.host.clone().unwrap_or_else(|| "unassigned".into());
                    let row = host_map.entry(host.clone()).or_default();
                    row.0 += 1;
                    if is_running(&vm.power_state) {
                        row.1 += 1;
                    }
                    if host != "unassigned" {
                        all_hosts.insert(host);
                    }
                    folder_map.entry(vm.folder.clone()).or_default().push(vm);
                }
                let hosts = host_map
                    .into_iter()
                    .map(|(name, (vm_count, running_vm_count))| InventoryHost {
                        name,
                        vm_count,
                        running_vm_count,
                    })
                    .collect();
                let folders = folder_map
                    .into_iter()
                    .map(|(name, vms)| InventoryFolder { name, vms })
                    .collect();
                cluster_rows.push(InventoryCluster {
                    name: cluster_name,
                    hosts,
                    folders,
                    vm_count,
                    running_vm_count: running,
                });
            }
            datacenters.push(InventoryDatacenter {
                name: dc_name,
                clusters: cluster_rows,
                vm_count: dc_count,
            });
        }
        InventoryTree {
            datacenters,
            total_vms,
            running_vms,
            hosts: all_hosts.len(),
        }
    }
}

fn is_running(state: &str) -> bool {
    matches!(state.to_ascii_lowercase().as_str(), "running" | "ready")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vm(
        name: &str,
        dc: &str,
        cluster: &str,
        folder: &str,
        host: Option<&str>,
        state: &str,
    ) -> InventoryVm {
        InventoryVm {
            name: name.into(),
            namespace: "default".into(),
            datacenter: dc.into(),
            cluster: cluster.into(),
            folder: folder.into(),
            host: host.map(str::to_string),
            power_state: state.into(),
            guest_os: None,
            tags: BTreeMap::new(),
            attributes: BTreeMap::new(),
        }
    }
    #[test]
    fn builds_hierarchy_and_counts() {
        let tree = InventoryBuilder::build(
            vec![
                vm("a", "dc1", "c1", "prod", Some("n1"), "Running"),
                vm("b", "dc1", "c1", "dev", Some("n2"), "Stopped"),
                vm("c", "dc2", "c2", "prod", None, "Stopped"),
            ],
            &InventoryFilter::default(),
        );
        assert_eq!(tree.total_vms, 3);
        assert_eq!(tree.running_vms, 1);
        assert_eq!(tree.hosts, 2);
        assert_eq!(tree.datacenters.len(), 2);
    }
    #[test]
    fn filters_without_destroying_hierarchy() {
        let f = InventoryFilter {
            datacenter: Some("dc1".into()),
            cluster: None,
            folder: Some("prod".into()),
        };
        let tree = InventoryBuilder::build(
            vec![
                vm("a", "dc1", "c1", "prod", Some("n1"), "Running"),
                vm("b", "dc1", "c1", "dev", Some("n2"), "Running"),
                vm("c", "dc2", "c2", "prod", Some("n3"), "Running"),
            ],
            &f,
        );
        assert_eq!(tree.total_vms, 1);
        assert_eq!(tree.datacenters[0].name, "dc1");
    }
    #[test]
    fn extracts_tags() {
        let mut m = BTreeMap::new();
        m.insert("tag.zorvia.io/tier".into(), "gold".into());
        m.insert("app".into(), "api".into());
        let t = extract_prefixed(&m, TAG_PREFIX);
        assert_eq!(t.get("tier").map(String::as_str), Some("gold"));
        assert_eq!(t.len(), 1);
    }
    #[test]
    fn unassigned_is_not_counted_as_host() {
        let tree = InventoryBuilder::build(
            vec![vm("a", "dc", "c", "f", None, "Stopped")],
            &InventoryFilter::default(),
        );
        assert_eq!(tree.hosts, 0);
    }
}
