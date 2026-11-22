// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Resource pools (Experimental): named sets of VMs with optional quota-like limits.
//!
//! Membership lives on the VM as a label `pool.zorvia.io/<pool>=member`, in the same way as
//! the datacenter/cluster/folder labels, so `kubectl` sees it and no CRD is needed. A VM may
//! be in several pools. Limits cannot live in labels (they carry numbers and a description), so
//! pool definitions are one JSON document in the shared state store (file, or PostgreSQL when
//! `ZORVIA_DATABASE_URL` is set), like warm pools and DR pairs. A label with no definition is
//! a valid, unlimited pool.
//!
//! Everything here is pure: reports and the hierarchy are computed from a VM list, and the
//! limits are never enforced by Kubernetes. They are evaluated read-only and, at VM create,
//! reported as advisory warnings (see docs/RESOURCE_POOLS.md).

use super::inventory::{CLUSTER_LABEL, DATACENTER_LABEL, FOLDER_LABEL};
use crate::kube::VirtualMachine;
use kube::ResourceExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FEATURE_ID: &str = "resource-pools";
pub const POOL_LABEL_PREFIX: &str = "pool.zorvia.io/";
pub const POOL_LABEL_VALUE: &str = "member";
pub const MAX_POOLS: usize = 500;
/// Usage at or above this share of a limit is reported as "near".
pub const NEAR_PERCENT: f64 = 90.0;

const RESERVED_NAMES: [&str; 2] = ["hierarchy", "report"];
const MAX_LIMIT_VMS: u64 = 1_000_000;
const MAX_LIMIT_VCPUS: u64 = 10_000_000;
const MAX_LIMIT_MEMORY_MIB: u64 = 1_000_000_000_000;

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Label key for membership in `pool`.
pub fn label_key(pool: &str) -> String {
    format!("{POOL_LABEL_PREFIX}{pool}")
}

/// A pool name is a DNS label (it becomes a label key, 63 characters at most).
pub fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !ok {
        return Err(format!(
            "pool name '{name}' must be 1-63 characters: lowercase letters, digits and '-', starting and ending with a letter or digit"
        ));
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(format!("pool name '{name}' is reserved"));
    }
    Ok(())
}

/// Pools named by the VM's labels (membership needs the value `member`).
pub fn pools_of(labels: &BTreeMap<String, String>) -> Vec<String> {
    labels
        .iter()
        .filter_map(|(k, v)| {
            let name = k.strip_prefix(POOL_LABEL_PREFIX)?;
            (v == POOL_LABEL_VALUE && !name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolLimits {
    #[serde(default)]
    pub max_vms: Option<u64>,
    #[serde(default)]
    pub max_vcpus: Option<u64>,
    #[serde(default)]
    pub max_memory_mib: Option<u64>,
}

impl PoolLimits {
    pub fn is_empty(&self) -> bool {
        self.max_vms.is_none() && self.max_vcpus.is_none() && self.max_memory_mib.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (what, v, max) in [
            ("max_vms", self.max_vms, MAX_LIMIT_VMS),
            ("max_vcpus", self.max_vcpus, MAX_LIMIT_VCPUS),
            ("max_memory_mib", self.max_memory_mib, MAX_LIMIT_MEMORY_MIB),
        ] {
            if let Some(v) = v {
                if v == 0 || v > max {
                    return Err(format!("{what} must be between 1 and {max}"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub limits: PoolLimits,
    pub created: chrono::DateTime<chrono::Utc>,
}

impl PoolDef {
    pub fn new(
        name: &str,
        description: &str,
        limits: PoolLimits,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, String> {
        validate_name(name)?;
        limits.validate()?;
        if description.chars().count() > 200 || description.chars().any(char::is_control) {
            return Err(
                "description must be at most 200 characters with no control characters".into(),
            );
        }
        Ok(PoolDef {
            name: name.into(),
            description: description.trim().into(),
            limits,
            created: now,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoolStore {
    #[serde(default)]
    pub pools: Vec<PoolDef>,
}

impl PoolStore {
    pub fn default_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("zorvia")
            .join("resource_pools.json")
    }

    pub fn load_from(path: &Path) -> Self {
        if let Some(c) = crate::state_docs::read(path) {
            match serde_json::from_str(&c) {
                Ok(s) => return s,
                Err(e) => log::warn!("Failed to parse resource pools: {e}"),
            }
        }
        Self::default()
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        crate::state_docs::write(path, &serde_json::to_string_pretty(self)?)
    }

    /// Load, mutate and save under a process-wide lock (one replica; see docs/POSTGRES.md for
    /// the last-writer-wins note across replicas).
    pub fn update<T>(
        path: &Path,
        f: impl FnOnce(&mut PoolStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Self::load_from(path);
        let out = f(&mut s)?;
        s.save_to(path)
            .map_err(|e| format!("persist failed: {e}"))?;
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Option<&PoolDef> {
        self.pools.iter().find(|p| p.name == name)
    }

    pub fn add(&mut self, def: PoolDef) -> Result<(), String> {
        if self.get(&def.name).is_some() {
            return Err(format!("pool '{}' already exists", def.name));
        }
        if self.pools.len() >= MAX_POOLS {
            return Err(format!("at most {MAX_POOLS} pools"));
        }
        self.pools.push(def);
        self.pools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let before = self.pools.len();
        self.pools.retain(|p| p.name != name);
        if self.pools.len() == before {
            return Err(format!("no such pool '{name}'"));
        }
        Ok(())
    }
}

/// What the pool code needs to know about one VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolVm {
    pub name: String,
    pub namespace: String,
    pub datacenter: String,
    pub cluster: String,
    pub folder: String,
    pub power_state: String,
    pub vcpus: u64,
    /// `None` when the VM spec has no parseable memory request.
    pub memory_mib: Option<u64>,
    pub pools: Vec<String>,
}

/// Memory in MiB (rounded up) from a Kubernetes quantity: `4Gi`, `512Mi`, `2G`, `1048576`.
pub fn quantity_mib(q: &str) -> Option<u64> {
    let q = q.trim();
    let split = q
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(q.len());
    let (num, suffix) = q.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match suffix {
        "" => 1.0,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "Ki" => 1024.0,
        "Mi" => 1024.0_f64.powi(2),
        "Gi" => 1024.0_f64.powi(3),
        "Ti" => 1024.0_f64.powi(4),
        _ => return None,
    };
    let bytes = n * mult;
    if !bytes.is_finite() || bytes < 0.0 {
        return None;
    }
    Some((bytes / (1024.0 * 1024.0)).ceil() as u64)
}

impl PoolVm {
    pub fn from_vm(vm: &VirtualMachine, default_namespace: &str) -> Self {
        let labels: BTreeMap<String, String> = vm.labels().clone().into_iter().collect();
        let domain = &vm.spec.template.spec.domain;
        let vcpus = domain
            .cpu
            .as_ref()
            .map(|c| {
                u64::from(c.cores.unwrap_or(1))
                    * u64::from(c.sockets.unwrap_or(1))
                    * u64::from(c.threads.unwrap_or(1))
            })
            .unwrap_or(1);
        let memory_mib = domain
            .resources
            .requests
            .as_ref()
            .and_then(|r| r.get("memory"))
            .and_then(|m| quantity_mib(m));
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
        let (datacenter, cluster, folder) = placement(&labels);
        PoolVm {
            name: vm.name_any(),
            namespace: vm
                .namespace()
                .unwrap_or_else(|| default_namespace.to_string()),
            datacenter,
            cluster,
            folder,
            power_state,
            vcpus,
            memory_mib,
            pools: pools_of(&labels),
        }
    }

    fn running(&self) -> bool {
        matches!(
            self.power_state.to_ascii_lowercase().as_str(),
            "running" | "ready"
        )
    }
}

/// Datacenter, cluster and folder of a VM, with the same defaults as `zorvia inventory show`.
pub fn placement(labels: &BTreeMap<String, String>) -> (String, String, String) {
    let get = |k: &str, d: &str| labels.get(k).cloned().unwrap_or_else(|| d.to_string());
    (
        get(DATACENTER_LABEL, "default-dc"),
        get(CLUSTER_LABEL, "default-cluster"),
        get(FOLDER_LABEL, "root"),
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolUsage {
    pub vms: u64,
    pub running_vms: u64,
    pub vcpus: u64,
    pub memory_mib: u64,
    /// Member VMs whose memory request could not be read (counted as 0 MiB).
    pub unknown_memory_vms: u64,
}

pub fn usage_of<'a>(vms: impl IntoIterator<Item = &'a PoolVm>) -> PoolUsage {
    let mut u = PoolUsage::default();
    for vm in vms {
        u.vms += 1;
        u.running_vms += u64::from(vm.running());
        u.vcpus += vm.vcpus;
        match vm.memory_mib {
            Some(m) => u.memory_mib += m,
            None => u.unknown_memory_vms += 1,
        }
    }
    u
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitState {
    Ok,
    Near,
    Exceeded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitCheck {
    pub dimension: String,
    pub limit: u64,
    pub used: u64,
    pub percent: f64,
    pub state: LimitState,
}

fn check(dimension: &str, limit: Option<u64>, used: u64) -> Option<LimitCheck> {
    let limit = limit?;
    let percent = used as f64 * 100.0 / limit as f64;
    let state = if used > limit {
        LimitState::Exceeded
    } else if percent >= NEAR_PERCENT {
        LimitState::Near
    } else {
        LimitState::Ok
    };
    Some(LimitCheck {
        dimension: dimension.into(),
        limit,
        used,
        percent: (percent * 10.0).round() / 10.0,
        state,
    })
}

/// Limit checks for one usage figure (only the dimensions that have a limit).
pub fn checks(limits: &PoolLimits, u: &PoolUsage) -> Vec<LimitCheck> {
    [
        check("vms", limits.max_vms, u.vms),
        check("vcpus", limits.max_vcpus, u.vcpus),
        check("memory_mib", limits.max_memory_mib, u.memory_mib),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolStatus {
    /// No limits set (an undefined label-only pool is always unlimited).
    Unlimited,
    Ok,
    Near,
    Exceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMember {
    pub name: String,
    pub namespace: String,
    pub power_state: String,
    pub vcpus: u64,
    pub memory_mib: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolReport {
    pub name: String,
    /// False when VMs carry the label but no definition exists (unlimited).
    pub defined: bool,
    pub description: String,
    pub limits: PoolLimits,
    pub usage: PoolUsage,
    pub checks: Vec<LimitCheck>,
    pub status: PoolStatus,
    pub members: Vec<PoolMember>,
}

/// Read-only evaluation of one pool against the VMs that carry its label.
pub fn report(name: &str, def: Option<&PoolDef>, vms: &[PoolVm]) -> PoolReport {
    let mut members: Vec<&PoolVm> = vms
        .iter()
        .filter(|v| v.pools.iter().any(|p| p == name))
        .collect();
    members.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    let usage = usage_of(members.iter().copied());
    let limits = def.map(|d| d.limits.clone()).unwrap_or_default();
    let checks = checks(&limits, &usage);
    let status = if checks.iter().any(|c| c.state == LimitState::Exceeded) {
        PoolStatus::Exceeded
    } else if checks.iter().any(|c| c.state == LimitState::Near) {
        PoolStatus::Near
    } else if checks.is_empty() {
        PoolStatus::Unlimited
    } else {
        PoolStatus::Ok
    };
    PoolReport {
        name: name.into(),
        defined: def.is_some(),
        description: def.map(|d| d.description.clone()).unwrap_or_default(),
        limits,
        usage,
        checks,
        status,
        members: members
            .into_iter()
            .map(|v| PoolMember {
                name: v.name.clone(),
                namespace: v.namespace.clone(),
                power_state: v.power_state.clone(),
                vcpus: v.vcpus,
                memory_mib: v.memory_mib,
            })
            .collect(),
    }
}

/// Reports for every defined pool and every label-only pool, by name.
pub fn report_all(defs: &[PoolDef], vms: &[PoolVm]) -> Vec<PoolReport> {
    let mut names: BTreeSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for vm in vms {
        names.extend(vm.pools.iter().map(String::as_str));
    }
    names
        .into_iter()
        .map(|n| report(n, defs.iter().find(|d| d.name == n), vms))
        .collect()
}

/// Advisory create-time warnings: what adding one VM would do to each pool it joins.
/// `vms` is the current inventory (the new VM is not in it yet). Pools without limits are
/// silent. Returns one line per limit that the new VM would push over its limit.
pub fn create_warnings(
    pools: &[String],
    defs: &[PoolDef],
    vms: &[PoolVm],
    new_vcpus: u64,
    new_memory_mib: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    for pool in pools {
        let Some(def) = defs.iter().find(|d| &d.name == pool) else {
            continue;
        };
        let mut u = usage_of(vms.iter().filter(|v| v.pools.contains(pool)));
        u.vms += 1;
        u.vcpus += new_vcpus;
        u.memory_mib += new_memory_mib;
        for c in checks(&def.limits, &u) {
            if c.state == LimitState::Exceeded {
                out.push(format!(
                    "pool '{pool}' would exceed {} ({} of {}) with this VM; limits are advisory and the VM was still created",
                    c.dimension, c.used, c.limit
                ));
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolNode {
    pub name: String,
    pub defined: bool,
    pub vms: u64,
    pub vcpus: u64,
    pub memory_mib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderNode {
    pub name: String,
    pub vm_count: u64,
    /// VMs in this folder that belong to no pool.
    pub unpooled_vms: u64,
    pub pools: Vec<PoolNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub name: String,
    pub folders: Vec<FolderNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatacenterNode {
    pub name: String,
    pub clusters: Vec<ClusterNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolHierarchy {
    pub datacenters: Vec<DatacenterNode>,
    pub total_vms: u64,
    pub pooled_vms: u64,
}

/// Datacenter > cluster > folder > pool. A VM in several pools appears under each of them
/// (so pool VM counts can add up to more than `vm_count`).
pub fn hierarchy(defs: &[PoolDef], vms: &[PoolVm]) -> PoolHierarchy {
    type Key = (String, String, String);
    let mut folders: BTreeMap<Key, Vec<&PoolVm>> = BTreeMap::new();
    for vm in vms {
        folders
            .entry((vm.datacenter.clone(), vm.cluster.clone(), vm.folder.clone()))
            .or_default()
            .push(vm);
    }
    let mut dcs: BTreeMap<String, BTreeMap<String, Vec<FolderNode>>> = BTreeMap::new();
    for ((dc, cl, folder), members) in folders {
        let mut per_pool: BTreeMap<&str, Vec<&PoolVm>> = BTreeMap::new();
        for vm in &members {
            for p in &vm.pools {
                per_pool.entry(p.as_str()).or_default().push(vm);
            }
        }
        let pools = per_pool
            .into_iter()
            .map(|(name, vs)| {
                let u = usage_of(vs.iter().copied());
                PoolNode {
                    name: name.into(),
                    defined: defs.iter().any(|d| d.name == name),
                    vms: u.vms,
                    vcpus: u.vcpus,
                    memory_mib: u.memory_mib,
                }
            })
            .collect();
        dcs.entry(dc)
            .or_default()
            .entry(cl)
            .or_default()
            .push(FolderNode {
                name: folder,
                vm_count: members.len() as u64,
                unpooled_vms: members.iter().filter(|v| v.pools.is_empty()).count() as u64,
                pools,
            });
    }
    PoolHierarchy {
        datacenters: dcs
            .into_iter()
            .map(|(name, cls)| DatacenterNode {
                name,
                clusters: cls
                    .into_iter()
                    .map(|(name, folders)| ClusterNode { name, folders })
                    .collect(),
            })
            .collect(),
        total_vms: vms.len() as u64,
        pooled_vms: vms.iter().filter(|v| !v.pools.is_empty()).count() as u64,
    }
}

/// JSON merge patch that adds (`member = true`) or removes the VM's pool label.
pub fn assign_patch(pool: &str, member: bool) -> Result<serde_json::Value, String> {
    validate_name(pool)?;
    let value = if member {
        serde_json::Value::String(POOL_LABEL_VALUE.into())
    } else {
        serde_json::Value::Null
    };
    Ok(serde_json::json!({ "metadata": { "labels": { label_key(pool): value } } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(
        name: &str,
        folder: &str,
        state: &str,
        cpus: u64,
        mem: Option<u64>,
        pools: &[&str],
    ) -> PoolVm {
        PoolVm {
            name: name.into(),
            namespace: "default".into(),
            datacenter: "dc1".into(),
            cluster: "c1".into(),
            folder: folder.into(),
            power_state: state.into(),
            vcpus: cpus,
            memory_mib: mem,
            pools: pools.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn def(name: &str, limits: PoolLimits) -> PoolDef {
        PoolDef::new(name, "", limits, chrono::Utc::now()).unwrap()
    }

    #[test]
    fn names_are_validated() {
        assert!(validate_name("prod-web").is_ok());
        for bad in [
            "",
            "Prod",
            "-a",
            "a-",
            "a/b",
            "a b",
            "hierarchy",
            &"x".repeat(64),
        ] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn limits_must_be_positive_and_bounded() {
        assert!(PoolLimits {
            max_vms: Some(0),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(PoolLimits {
            max_vcpus: Some(u64::MAX),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(PoolLimits {
            max_vms: Some(5),
            max_vcpus: Some(8),
            max_memory_mib: Some(1024)
        }
        .validate()
        .is_ok());
        assert!(PoolDef::new("a", "bad\ndesc", PoolLimits::default(), chrono::Utc::now()).is_err());
    }

    #[test]
    fn quantities_parse_to_mib() {
        assert_eq!(quantity_mib("4Gi"), Some(4096));
        assert_eq!(quantity_mib("512Mi"), Some(512));
        assert_eq!(quantity_mib("1048576"), Some(1));
        assert_eq!(quantity_mib("2G"), Some(1908));
        assert_eq!(quantity_mib("lots"), None);
        assert_eq!(quantity_mib("4Xi"), None);
    }

    #[test]
    fn membership_needs_the_member_value() {
        let mut l = BTreeMap::new();
        l.insert("pool.zorvia.io/a".into(), "member".into());
        l.insert("pool.zorvia.io/b".into(), "false".into());
        l.insert("tag.zorvia.io/c".into(), "member".into());
        assert_eq!(pools_of(&l), vec!["a".to_string()]);
    }

    #[test]
    fn report_counts_and_flags_exceeded_limits() {
        let vms = vec![
            vm("a", "f", "Running", 4, Some(4096), &["p"]),
            vm("b", "f", "Stopped", 4, Some(4096), &["p"]),
            vm("c", "f", "Running", 2, None, &["p", "q"]),
            vm("d", "f", "Running", 8, Some(8192), &["q"]),
        ];
        let d = def(
            "p",
            PoolLimits {
                max_vms: Some(2),
                max_vcpus: Some(20),
                max_memory_mib: Some(8192),
            },
        );
        let r = report("p", Some(&d), &vms);
        assert_eq!(
            r.usage,
            PoolUsage {
                vms: 3,
                running_vms: 2,
                vcpus: 10,
                memory_mib: 8192,
                unknown_memory_vms: 1
            }
        );
        assert_eq!(r.status, PoolStatus::Exceeded);
        let by: BTreeMap<_, _> = r
            .checks
            .iter()
            .map(|c| (c.dimension.as_str(), c.state))
            .collect();
        assert_eq!(by["vms"], LimitState::Exceeded);
        assert_eq!(by["vcpus"], LimitState::Ok);
        assert_eq!(by["memory_mib"], LimitState::Ok); // exactly at the limit is not exceeded
        assert_eq!(r.members.len(), 3);
    }

    #[test]
    fn near_threshold_and_unlimited() {
        let vms = vec![vm("a", "f", "Running", 9, Some(1), &["p", "u"])];
        let d = def(
            "p",
            PoolLimits {
                max_vcpus: Some(10),
                ..Default::default()
            },
        );
        assert_eq!(report("p", Some(&d), &vms).status, PoolStatus::Near);
        let u = report("u", None, &vms);
        assert!(!u.defined);
        assert_eq!(u.status, PoolStatus::Unlimited);
        let empty = def(
            "e",
            PoolLimits {
                max_vms: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(report("e", Some(&empty), &vms).status, PoolStatus::Ok);
    }

    #[test]
    fn report_all_includes_label_only_pools() {
        let vms = vec![vm("a", "f", "Running", 1, Some(1), &["ghost"])];
        let defs = vec![def("real", PoolLimits::default())];
        let all = report_all(&defs, &vms);
        let names: Vec<_> = all.iter().map(|r| (r.name.as_str(), r.defined)).collect();
        assert_eq!(names, vec![("ghost", false), ("real", true)]);
    }

    #[test]
    fn create_warnings_are_advisory_and_only_for_limited_pools() {
        let vms = vec![vm("a", "f", "Running", 4, Some(4096), &["p"])];
        let defs = vec![
            def(
                "p",
                PoolLimits {
                    max_vms: Some(1),
                    max_vcpus: Some(8),
                    max_memory_mib: None,
                },
            ),
            def("free", PoolLimits::default()),
        ];
        let w = create_warnings(
            &["p".into(), "free".into(), "undefined".into()],
            &defs,
            &vms,
            2,
            1024,
        );
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("vms") && w[0].contains("advisory"));
        assert!(create_warnings(&["p".into()], &defs, &[], 2, 1024).is_empty());
    }

    #[test]
    fn hierarchy_groups_datacenter_cluster_folder_pool() {
        let mut other = vm("z", "g", "Running", 1, Some(1), &[]);
        other.datacenter = "dc2".into();
        let vms = vec![
            vm("a", "prod", "Running", 2, Some(2048), &["web", "gold"]),
            vm("b", "prod", "Running", 2, Some(2048), &["web"]),
            vm("c", "prod", "Stopped", 1, Some(512), &[]),
            other,
        ];
        let defs = vec![def("web", PoolLimits::default())];
        let h = hierarchy(&defs, &vms);
        assert_eq!((h.total_vms, h.pooled_vms), (4, 2));
        assert_eq!(h.datacenters.len(), 2);
        let f = &h.datacenters[0].clusters[0].folders[0];
        assert_eq!(
            (f.name.as_str(), f.vm_count, f.unpooled_vms),
            ("prod", 3, 1)
        );
        let names: Vec<_> = f
            .pools
            .iter()
            .map(|p| (p.name.as_str(), p.defined, p.vms, p.vcpus))
            .collect();
        assert_eq!(names, vec![("gold", false, 1, 2), ("web", true, 2, 4)]);
    }

    #[test]
    fn assign_patch_adds_and_removes_the_label() {
        let add = assign_patch("web", true).unwrap();
        assert_eq!(
            add.pointer("/metadata/labels/pool.zorvia.io~1web")
                .and_then(|v| v.as_str()),
            Some("member")
        );
        let rm = assign_patch("web", false).unwrap();
        assert!(rm
            .pointer("/metadata/labels/pool.zorvia.io~1web")
            .unwrap()
            .is_null());
        assert!(assign_patch("Bad/Name", true).is_err());
    }

    #[test]
    fn store_round_trips_and_rejects_duplicates() {
        let dir = std::env::temp_dir().join(format!("zorvia-pools-{}", std::process::id()));
        let path = dir.join("resource_pools.json");
        let _ = std::fs::remove_dir_all(&dir);
        PoolStore::update(&path, |s| {
            s.add(def(
                "a",
                PoolLimits {
                    max_vms: Some(1),
                    ..Default::default()
                },
            ))
        })
        .unwrap();
        assert!(PoolStore::update(&path, |s| s.add(def("a", PoolLimits::default()))).is_err());
        assert_eq!(PoolStore::load_from(&path).pools.len(), 1);
        PoolStore::update(&path, |s| s.remove("a")).unwrap();
        assert!(PoolStore::update(&path, |s| s.remove("a")).is_err());
        assert!(PoolStore::load_from(&path).pools.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_a_fake_kubevirt_vm() {
        let vm: VirtualMachine = serde_json::from_value(serde_json::json!({
            "apiVersion": "kubevirt.io/v1", "kind": "VirtualMachine",
            "metadata": {"name": "web1", "namespace": "ns1", "labels": {
                "pool.zorvia.io/web": "member",
                "inventory.zorvia.io/datacenter": "dc9",
                "inventory.zorvia.io/folder": "prod"}},
            "spec": {"running": true, "template": {"spec": {"domain": {
                "cpu": {"cores": 2, "sockets": 2},
                "resources": {"requests": {"memory": "4Gi"}},
                "devices": {}}}}},
            "status": {"printableStatus": "Running"}
        }))
        .unwrap();
        let v = PoolVm::from_vm(&vm, "fallback");
        assert_eq!((v.vcpus, v.memory_mib), (4, Some(4096)));
        assert_eq!(
            (v.datacenter.as_str(), v.cluster.as_str(), v.folder.as_str()),
            ("dc9", "default-cluster", "prod")
        );
        assert_eq!(v.pools, vec!["web".to_string()]);
        assert_eq!(v.namespace, "ns1");
        assert!(v.running());
    }
}
