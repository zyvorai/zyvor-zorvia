// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Passthrough devices (GPUs, SR-IOV virtual functions, other host devices): what
//! the cluster offers, and whether a VM that asks for some can actually start.
//!
//! KubeVirt only schedules a device that (1) a device plugin advertises as an
//! extended resource on at least one node with capacity left, and (2) is listed in
//! the KubeVirt CR's `permittedHostDevices`. Without a check, a VM that misses either
//! is created and then sits Pending or is refused with an obscure condition. This
//! module is pure (the API handlers gather the inputs) and unit-tested.

use crate::config::HostDeviceConfig;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Pseudo-devices KubeVirt itself advertises on every node; not passthrough hardware.
pub fn is_builtin(resource: &str) -> bool {
    resource.starts_with("devices.kubevirt.io/")
}

/// `4`, `1k` (=1000, KubeVirt's builtin counts), anything else is not a count.
fn parse_count(q: &str) -> Option<u64> {
    let q = q.trim();
    match q.strip_suffix('k') {
        Some(n) => n.parse::<u64>().ok().map(|n| n * 1000),
        None => q.parse().ok(),
    }
}

/// The device-plugin resources in a node's allocatable map: extended resources
/// (`vendor.com/name`) with a usable count, minus kubernetes.io/* and hugepages.
pub fn extended_resources(allocatable: &BTreeMap<String, String>) -> BTreeMap<String, u64> {
    allocatable
        .iter()
        .filter(|(k, _)| {
            k.contains('/') && !k.starts_with("kubernetes.io/") && !k.starts_with("hugepages-")
        })
        .filter_map(|(k, v)| parse_count(v).map(|n| (k.clone(), n)))
        .collect()
}

/// A Kubernetes quantity in bytes: a plain integer or one with a binary suffix
/// (`Ki`, `Mi`, `Gi`, `Ti`), as hugepage capacity is reported.
pub fn parse_bytes(q: &str) -> Option<u64> {
    let q = q.trim();
    for (suffix, mult) in [
        ("Ti", 1u64 << 40),
        ("Gi", 1 << 30),
        ("Mi", 1 << 20),
        ("Ki", 1 << 10),
    ] {
        if let Some(n) = q.strip_suffix(suffix) {
            return n.parse::<u64>().ok().and_then(|n| n.checked_mul(mult));
        }
    }
    q.parse().ok()
}

/// Number of hugepages of each size a node can still give, from its allocatable map
/// (`hugepages-2Mi: 1Gi` -> `2Mi: 512`).
pub fn hugepages(allocatable: &BTreeMap<String, String>) -> BTreeMap<String, u64> {
    allocatable
        .iter()
        .filter_map(|(k, v)| {
            let size = k.strip_prefix("hugepages-")?;
            let page = parse_bytes(size)?;
            let total = parse_bytes(v)?;
            (page > 0).then(|| (size.to_string(), total / page))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeDevices {
    pub node: String,
    /// The node runs the static CPU manager policy (KubeVirt's `kubevirt.io/cpumanager` label),
    /// which dedicated CPU placement needs.
    pub cpu_manager: bool,
    /// Free hugepages by page size (`2Mi`, `1Gi`).
    pub hugepages: BTreeMap<String, u64>,
    /// Passthrough candidates: extended resources that are not KubeVirt builtins.
    pub devices: BTreeMap<String, u64>,
    /// KubeVirt's own pseudo-devices (kvm, tun, ...), listed for completeness.
    pub builtin: BTreeMap<String, u64>,
}

impl NodeDevices {
    pub fn from_allocatable(node: &str, allocatable: &BTreeMap<String, String>) -> Self {
        let (builtin, devices) = extended_resources(allocatable)
            .into_iter()
            .partition(|(k, _)| is_builtin(k));
        Self {
            node: node.to_string(),
            cpu_manager: false,
            hugepages: hugepages(allocatable),
            devices,
            builtin,
        }
    }

    /// Like [`from_allocatable`](Self::from_allocatable), with the node's labels.
    pub fn from_node(
        node: &str,
        allocatable: &BTreeMap<String, String>,
        labels: &BTreeMap<String, String>,
    ) -> Self {
        let mut n = Self::from_allocatable(node, allocatable);
        n.cpu_manager = labels.get("kubevirt.io/cpumanager").map(String::as_str) == Some("true");
        n
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermittedDevice {
    pub resource_name: String,
    /// `pci`, `mediated` or `usb`.
    pub kind: &'static str,
    /// The selector KubeVirt matches (PCI `vendor:product`, mdev type name, or USB selectors).
    pub selector: String,
    /// A device plugin outside KubeVirt manages the resource.
    pub external: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KubevirtDevices {
    pub feature_gates: Vec<String>,
    pub permitted: Vec<PermittedDevice>,
}

impl KubevirtDevices {
    /// From the KubeVirt CR's `spec.configuration`.
    pub fn from_configuration(cfg: &Value) -> Self {
        let gates = cfg
            .pointer("/developerConfiguration/featureGates")
            .and_then(|g| g.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut permitted = Vec::new();
        let list = |key: &str| {
            cfg.pointer(&format!("/permittedHostDevices/{key}"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        };
        let text = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        for d in list("pciHostDevices") {
            permitted.push(PermittedDevice {
                resource_name: text(&d, "resourceName"),
                kind: "pci",
                selector: text(&d, "pciVendorSelector"),
                external: d
                    .get("externalResourceProvider")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
            });
        }
        for d in list("mediatedDevices") {
            permitted.push(PermittedDevice {
                resource_name: text(&d, "resourceName"),
                kind: "mediated",
                selector: text(&d, "mdevNameSelector"),
                external: d
                    .get("externalResourceProvider")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
            });
        }
        for d in list("usb") {
            let sel = d
                .get("selectors")
                .and_then(|s| s.as_array())
                .map(|a| {
                    a.iter()
                        .map(|s| format!("{}:{}", text(s, "vendor"), text(s, "product")))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            permitted.push(PermittedDevice {
                resource_name: text(&d, "resourceName"),
                kind: "usb",
                selector: sel,
                external: d
                    .get("externalResourceProvider")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
            });
        }
        permitted.retain(|p| !p.resource_name.is_empty());
        Self {
            feature_gates: gates,
            permitted,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeviceInventory {
    pub nodes: Vec<NodeDevices>,
    /// `None` when the KubeVirt CR could not be read (RBAC, or not found).
    pub kubevirt: Option<KubevirtDevices>,
    /// The NetworkAttachmentDefinition CRD (Multus) exists, which SR-IOV needs.
    pub multus_installed: bool,
    /// SR-IOV networks (attachments that name a virtual-function pool) and how many VFs are free.
    pub sriov_pools: Vec<SriovPool>,
    /// Likely device kind (`gpu` or `host-device`) for each advertised resource, a hint for forms.
    pub kind_hints: BTreeMap<String, String>,
}

/// A `NetworkAttachmentDefinition` that names a virtual-function pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SriovPool {
    pub namespace: String,
    pub network: String,
    /// The `k8s.v1.cni.cncf.io/resourceName` annotation.
    pub resource_name: String,
    /// Free virtual functions across all nodes.
    pub free: u64,
    /// Nodes that have any.
    pub nodes: usize,
}

impl DeviceInventory {
    /// Resolve attachments `(namespace, name, resource_name)` against what the nodes advertise.
    /// Fill [`kind_hints`](Self::kind_hints) from the advertised resources.
    pub fn with_kind_hints(mut self) -> Self {
        self.kind_hints = self
            .nodes
            .iter()
            .flat_map(|n| n.devices.keys())
            .map(|r| (r.clone(), kind_hint(r).to_string()))
            .collect();
        self
    }

    pub fn with_sriov_pools(mut self, nads: &[(String, String, String)]) -> Self {
        self.sriov_pools = nads
            .iter()
            .map(|(ns, name, res)| {
                let (free, nodes) = self.capacity(res);
                SriovPool {
                    namespace: ns.clone(),
                    network: name.clone(),
                    resource_name: res.clone(),
                    free,
                    nodes,
                }
            })
            .collect();
        self
    }
}

impl DeviceInventory {
    /// Total of `resource` across schedulable nodes, and how many nodes have any.
    fn capacity(&self, resource: &str) -> (u64, usize) {
        let counts: Vec<u64> = self
            .nodes
            .iter()
            .filter_map(|n| n.devices.get(resource).copied())
            .collect();
        (
            counts.iter().sum(),
            counts.iter().filter(|c| **c > 0).count(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// `error` blocks the VM; `warning` is advice.
    pub severity: &'static str,
    pub subject: String,
    pub message: String,
}

fn err(subject: &str, message: String) -> Issue {
    Issue {
        severity: "error",
        subject: subject.to_string(),
        message,
    }
}

fn warn(subject: &str, message: String) -> Issue {
    Issue {
        severity: "warning",
        subject: subject.to_string(),
        message,
    }
}

/// An SR-IOV network a VM wants, resolved by the caller from the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SriovNetwork {
    pub name: String,
    pub exists: bool,
    /// The NAD's `k8s.v1.cni.cncf.io/resourceName` annotation (the VF pool).
    pub resource_name: Option<String>,
}

pub fn preflight(
    devices: &[HostDeviceConfig],
    sriov: &[SriovNetwork],
    inv: &DeviceInventory,
) -> Vec<Issue> {
    let mut out = Vec::new();
    for d in devices {
        let subject = d.name.as_str();
        if let Some(kv) = &inv.kubevirt {
            if !kv
                .permitted
                .iter()
                .any(|p| p.resource_name == d.device_name)
            {
                out.push(err(
                    subject,
                    format!(
                        "'{}' is not in the KubeVirt CR's spec.configuration.permittedHostDevices, so KubeVirt will refuse to attach it",
                        d.device_name
                    ),
                ));
            }
        }
        match inv.capacity(&d.device_name) {
            (_, 0) if inv.nodes.iter().any(|n| n.devices.contains_key(&d.device_name)) => {
                out.push(err(
                    subject,
                    format!("every '{}' is already in use (allocatable is 0 on every node)", d.device_name),
                ))
            }
            (_, 0) => out.push(err(
                subject,
                format!(
                    "no node advertises '{}': is the device plugin running and the hardware present?",
                    d.device_name
                ),
            )),
            _ => {}
        }
    }
    for n in sriov {
        if !inv.multus_installed {
            out.push(err(
                &n.name,
                "the NetworkAttachmentDefinition CRD is not installed (Multus), which SR-IOV interfaces need".into(),
            ));
            continue;
        }
        if !n.exists {
            out.push(err(
                &n.name,
                format!(
                    "NetworkAttachmentDefinition '{}' does not exist in the VM's namespace",
                    n.name
                ),
            ));
            continue;
        }
        match &n.resource_name {
            None => out.push(warn(
                &n.name,
                "the network has no k8s.v1.cni.cncf.io/resourceName annotation, so the virtual-function pool cannot be checked".into(),
            )),
            Some(r) => {
                if inv.capacity(r).1 == 0 {
                    out.push(err(
                        &n.name,
                        format!("no node has free '{r}' virtual functions (device plugin missing, or all in use)"),
                    ));
                }
            }
        }
    }
    if !devices.is_empty() || !sriov.is_empty() {
        out.push(Issue {
            severity: "warning",
            subject: "live-migration".into(),
            message: "a VM with a passthrough device or SR-IOV interface is pinned to its node: it cannot be live-migrated, and a node drain will stop it".into(),
        });
    }
    out
}

/// CPU and memory placement a VM asks for, which GPU workloads usually need together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlacementRequest {
    pub dedicated_cpus: bool,
    pub numa_passthrough: bool,
    /// Hugepage size (`2Mi`, `1Gi`).
    pub hugepages: Option<String>,
}

impl PlacementRequest {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Rules KubeVirt's admission webhook enforces for NUMA passthrough (checked against
/// KubeVirt 1.9: it refuses `guestMappingPassthrough` without `dedicatedCpuPlacement`, and
/// without requested hugepages), so the API can say so before sending the VM.
pub fn placement_rules(req: &PlacementRequest) -> Result<(), String> {
    if req.numa_passthrough && !req.dedicated_cpus {
        return Err(
            "NUMA passthrough needs dedicated CPU placement (cpu_dedicated_placement: true)".into(),
        );
    }
    if req.numa_passthrough && req.hugepages.is_none() {
        return Err("NUMA passthrough needs hugepages (memory_hugepages_page_size, e.g. \"2Mi\" or \"1Gi\")".into());
    }
    if let Some(h) = &req.hugepages {
        if parse_bytes(h).is_none_or(|b| b == 0) {
            return Err(format!(
                "hugepage size '{h}' is not a quantity like 2Mi or 1Gi"
            ));
        }
    }
    Ok(())
}

/// Can some node give this placement? Dedicated CPUs need the static CPU manager policy;
/// hugepages need free pages of that size. Skipped when no nodes could be read.
pub fn placement_preflight(req: &PlacementRequest, inv: &DeviceInventory) -> Vec<Issue> {
    let mut out = Vec::new();
    if let Err(m) = placement_rules(req) {
        out.push(err("placement", m));
        return out;
    }
    if inv.nodes.is_empty() {
        return out;
    }
    if req.dedicated_cpus && !inv.nodes.iter().any(|n| n.cpu_manager) {
        out.push(err(
            "dedicated-cpus",
            "no node runs the static CPU manager policy (label kubevirt.io/cpumanager=true), so the VM would stay Pending".into(),
        ));
    }
    if let Some(h) = &req.hugepages {
        if !inv
            .nodes
            .iter()
            .any(|n| n.hugepages.get(h).copied().unwrap_or(0) > 0)
        {
            out.push(err(
                "hugepages",
                format!("no node has free {h} hugepages (reserve them on the node, e.g. vm.nr_hugepages)"),
            ));
        }
    }
    if req.numa_passthrough {
        out.push(warn(
            "numa",
            "guest NUMA mapping follows the host node's topology: the VM cannot be live-migrated to a node with a different layout".into(),
        ));
    }
    out
}

// ── Permitting devices in the KubeVirt CR ─────────────────────────────────────────────

/// A device to add to `spec.configuration.permittedHostDevices`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PermitRequest {
    /// A PCI device by `vendor:product` id (`10DE:2236`).
    Pci {
        resource_name: String,
        pci_vendor_selector: String,
        #[serde(default)]
        external_resource_provider: bool,
    },
    /// A mediated (vGPU / mdev) device by type name.
    Mediated {
        resource_name: String,
        mdev_name_selector: String,
        #[serde(default)]
        external_resource_provider: bool,
    },
    /// A USB device by vendor/product id.
    Usb {
        resource_name: String,
        vendor: String,
        product: String,
        #[serde(default)]
        external_resource_provider: bool,
    },
}

fn hex4(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl PermitRequest {
    pub fn resource_name(&self) -> &str {
        match self {
            Self::Pci { resource_name, .. }
            | Self::Mediated { resource_name, .. }
            | Self::Usb { resource_name, .. } => resource_name,
        }
    }

    fn key(&self) -> &'static str {
        match self {
            Self::Pci { .. } => "pciHostDevices",
            Self::Mediated { .. } => "mediatedDevices",
            Self::Usb { .. } => "usb",
        }
    }

    /// The entry as KubeVirt stores it, with selectors normalised (PCI ids upper case,
    /// USB ids lower case).
    pub fn validated_entry(&self) -> Result<Value, String> {
        let name = self.resource_name();
        if !HostDeviceConfig::valid_device_name(name) || is_builtin(name) {
            return Err(format!(
                "resource name '{name}' must look like vendor.com/resource (and not be a KubeVirt builtin)"
            ));
        }
        use serde_json::json;
        match self {
            Self::Pci {
                pci_vendor_selector: sel,
                external_resource_provider: ext,
                ..
            } => {
                let ok = sel.split_once(':').is_some_and(|(v, p)| hex4(v) && hex4(p));
                if !ok {
                    return Err(format!(
                        "PCI selector '{sel}' must be vendor:product in hex, like 10DE:2236"
                    ));
                }
                let mut e =
                    json!({ "pciVendorSelector": sel.to_ascii_uppercase(), "resourceName": name });
                if *ext {
                    e["externalResourceProvider"] = json!(true);
                }
                Ok(e)
            }
            Self::Mediated {
                mdev_name_selector: sel,
                external_resource_provider: ext,
                ..
            } => {
                let ok = !sel.trim().is_empty()
                    && sel.len() <= 128
                    && sel
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b" ._-/".contains(&c));
                if !ok {
                    return Err("mediated device type name must be 1 to 128 characters of letters, digits, space and ._-/".into());
                }
                let mut e = json!({ "mdevNameSelector": sel, "resourceName": name });
                if *ext {
                    e["externalResourceProvider"] = json!(true);
                }
                Ok(e)
            }
            Self::Usb {
                vendor,
                product,
                external_resource_provider: ext,
                ..
            } => {
                if !hex4(vendor) || !hex4(product) {
                    return Err("USB vendor and product ids must be 4 hex digits".into());
                }
                let mut e = json!({
                    "resourceName": name,
                    "selectors": [{ "vendor": vendor.to_ascii_lowercase(), "product": product.to_ascii_lowercase() }],
                });
                if *ext {
                    e["externalResourceProvider"] = json!(true);
                }
                Ok(e)
            }
        }
    }
}

const PERMITTED_KEYS: [&str; 3] = ["pciHostDevices", "mediatedDevices", "usb"];

fn permitted_list(cfg: &Value, key: &str) -> Vec<Value> {
    cfg.pointer(&format!("/permittedHostDevices/{key}"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn merge_patch(key: &str, list: Vec<Value>, resource_version: Option<&str>) -> Value {
    // An emptied list is removed (null in a merge patch) rather than left as `[]`.
    let list = if list.is_empty() {
        Value::Null
    } else {
        Value::Array(list)
    };
    let mut patch = serde_json::json!({
        "spec": { "configuration": { "permittedHostDevices": { key: list } } }
    });
    // A merge patch replaces a whole list, so name the version we read: a concurrent change
    // makes the API server answer 409 instead of silently losing it.
    if let Some(rv) = resource_version {
        patch["metadata"] = serde_json::json!({ "resourceVersion": rv });
    }
    patch
}

#[derive(Debug, PartialEq, Eq)]
pub enum PermitError {
    Invalid(String),
    Conflict(String),
    NotFound(String),
}

impl std::fmt::Display for PermitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) | Self::Conflict(m) | Self::NotFound(m) => f.write_str(m),
        }
    }
}

/// The merge patch that permits `req`, given the CR's current `spec.configuration`.
/// `Ok(None)`: exactly this entry is already there.
pub fn permit_patch(
    cfg: &Value,
    req: &PermitRequest,
    resource_version: Option<&str>,
) -> Result<Option<Value>, PermitError> {
    let entry = req.validated_entry().map_err(PermitError::Invalid)?;
    let name = req.resource_name();
    for key in PERMITTED_KEYS {
        for existing in permitted_list(cfg, key) {
            if existing.get("resourceName").and_then(|v| v.as_str()) == Some(name) {
                // Compare ignoring a missing `externalResourceProvider: false`.
                let norm = |v: &Value| {
                    let mut v = v.clone();
                    if v.get("externalResourceProvider") == Some(&Value::Bool(false)) {
                        v.as_object_mut()
                            .map(|o| o.remove("externalResourceProvider"));
                    }
                    v
                };
                return if key == req.key() && norm(&existing) == norm(&entry) {
                    Ok(None)
                } else {
                    Err(PermitError::Conflict(format!(
                        "'{name}' is already permitted with a different selector or kind; remove it first"
                    )))
                };
            }
        }
    }
    let mut list = permitted_list(cfg, req.key());
    list.push(entry);
    Ok(Some(merge_patch(req.key(), list, resource_version)))
}

/// The merge patch that stops permitting `resource_name`.
pub fn unpermit_patch(
    cfg: &Value,
    resource_name: &str,
    resource_version: Option<&str>,
) -> Result<Value, PermitError> {
    for key in PERMITTED_KEYS {
        let list = permitted_list(cfg, key);
        let kept: Vec<Value> = list
            .iter()
            .filter(|e| e.get("resourceName").and_then(|v| v.as_str()) != Some(resource_name))
            .cloned()
            .collect();
        if kept.len() != list.len() {
            return Ok(merge_patch(key, kept, resource_version));
        }
    }
    Err(PermitError::NotFound(format!(
        "'{resource_name}' is not in permittedHostDevices"
    )))
}

/// A hint for the Create VM page: what a resource name is most likely to be.
pub fn kind_hint(resource: &str) -> &'static str {
    let r = resource.to_ascii_lowercase();
    let gpu_words = [
        "gpu", "grid", "a100", "h100", "a10", "t4", "l4", "l40", "v100", "mi250", "mi300", "vgpu",
        "mig-",
    ];
    if r.starts_with("nvidia.com/")
        || r.starts_with("amd.com/")
        || gpu_words.iter().any(|w| r.contains(w))
    {
        "gpu"
    } else {
        "host-device"
    }
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == "error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HostDeviceKind;
    use serde_json::json;

    fn alloc(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn dev(name: &str, resource: &str) -> HostDeviceConfig {
        HostDeviceConfig {
            name: name.into(),
            device_name: resource.into(),
            kind: HostDeviceKind::Gpu,
        }
    }

    fn inventory(gpus: u64, permitted: bool) -> DeviceInventory {
        let a = alloc(&[
            ("cpu", "12"),
            ("memory", "32Gi"),
            ("hugepages-2Mi", "0"),
            ("nvidia.com/GA102GL_A10", &gpus.to_string()),
            ("intel.com/sriov_vfs", "0"),
            ("devices.kubevirt.io/kvm", "1k"),
            ("kubernetes.io/foo", "3"),
        ]);
        DeviceInventory {
            nodes: vec![NodeDevices::from_allocatable("n1", &a)],
            kubevirt: Some(KubevirtDevices {
                feature_gates: vec![],
                permitted: if permitted {
                    vec![PermittedDevice {
                        resource_name: "nvidia.com/GA102GL_A10".into(),
                        kind: "pci",
                        selector: "10DE:2236".into(),
                        external: false,
                    }]
                } else {
                    vec![]
                },
            }),
            multus_installed: true,
            ..Default::default()
        }
    }

    #[test]
    fn allocatable_is_split_into_passthrough_candidates_and_builtins() {
        let n = NodeDevices::from_allocatable(
            "n1",
            &alloc(&[
                ("cpu", "4"),
                ("nvidia.com/GA102GL_A10", "2"),
                ("devices.kubevirt.io/kvm", "1k"),
                ("hugepages-1Gi", "0"),
                ("kubernetes.io/x", "1"),
                ("vendor.com/odd", "1.5"),
            ]),
        );
        assert_eq!(n.devices.get("nvidia.com/GA102GL_A10"), Some(&2));
        assert_eq!(
            n.devices.len(),
            1,
            "no cpu, hugepages, kubernetes.io or non-integer: {:?}",
            n.devices
        );
        assert_eq!(n.builtin.get("devices.kubevirt.io/kvm"), Some(&1000));
    }

    #[test]
    fn kubevirt_configuration_yields_permitted_devices_of_every_kind() {
        let cfg = json!({
            "developerConfiguration": {"featureGates": ["GPU", "Snapshot"]},
            "permittedHostDevices": {
                "pciHostDevices": [{"resourceName": "nvidia.com/A10", "pciVendorSelector": "10DE:2236"}],
                "mediatedDevices": [{"resourceName": "nvidia.com/vgpu", "mdevNameSelector": "NVIDIA A10-4Q", "externalResourceProvider": true}],
                "usb": [{"resourceName": "kubevirt.io/key", "selectors": [{"vendor": "046d", "product": "c52b"}]}]
            }
        });
        let k = KubevirtDevices::from_configuration(&cfg);
        assert_eq!(k.feature_gates, vec!["GPU", "Snapshot"]);
        let kinds: Vec<_> = k
            .permitted
            .iter()
            .map(|p| (p.kind, p.resource_name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("pci", "nvidia.com/A10"),
                ("mediated", "nvidia.com/vgpu"),
                ("usb", "kubevirt.io/key")
            ]
        );
        assert!(k.permitted[1].external);
        assert_eq!(k.permitted[2].selector, "046d:c52b");
        assert!(KubevirtDevices::from_configuration(&json!({}))
            .permitted
            .is_empty());
    }

    #[test]
    fn a_permitted_available_device_passes_with_only_the_migration_warning() {
        let issues = preflight(
            &[dev("gpu0", "nvidia.com/GA102GL_A10")],
            &[],
            &inventory(2, true),
        );
        assert!(!has_errors(&issues), "{issues:?}");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].subject, "live-migration");
    }

    #[test]
    fn missing_permission_missing_hardware_and_exhausted_devices_are_errors() {
        // Advertised and free, but KubeVirt does not permit it.
        let i = preflight(
            &[dev("gpu0", "nvidia.com/GA102GL_A10")],
            &[],
            &inventory(2, false),
        );
        assert!(i
            .iter()
            .any(|x| x.severity == "error" && x.message.contains("permittedHostDevices")));
        // Permitted, but every unit is taken.
        let i = preflight(
            &[dev("gpu0", "nvidia.com/GA102GL_A10")],
            &[],
            &inventory(0, true),
        );
        assert!(i.iter().any(|x| x.message.contains("already in use")));
        // Nothing advertises that resource at all.
        let i = preflight(&[dev("x", "amd.com/mi300")], &[], &inventory(2, true));
        assert!(i.iter().any(|x| x.message.contains("no node advertises")));
        // KubeVirt config unreadable: do not invent a permission error.
        let mut inv = inventory(2, true);
        inv.kubevirt = None;
        assert!(!has_errors(&preflight(
            &[dev("gpu0", "nvidia.com/GA102GL_A10")],
            &[],
            &inv
        )));
    }

    #[test]
    fn sriov_needs_multus_an_existing_network_and_free_virtual_functions() {
        let net = |exists, res: Option<&str>| SriovNetwork {
            name: "vf-net".into(),
            exists,
            resource_name: res.map(str::to_string),
        };
        let inv = inventory(1, true);
        // Free VFs: fine (warning only).
        let mut ok = inv.clone();
        ok.nodes[0].devices.insert("intel.com/sriov_vfs".into(), 8);
        assert!(!has_errors(&preflight(
            &[],
            &[net(true, Some("intel.com/sriov_vfs"))],
            &ok
        )));
        // Pool exists but is empty.
        assert!(
            preflight(&[], &[net(true, Some("intel.com/sriov_vfs"))], &inv)
                .iter()
                .any(|i| i.message.contains("no node has free"))
        );
        // Network missing, annotation missing, Multus missing.
        assert!(preflight(&[], &[net(false, None)], &inv)
            .iter()
            .any(|i| i.message.contains("does not exist")));
        let w = preflight(&[], &[net(true, None)], &inv);
        assert!(!has_errors(&w) && w.iter().any(|i| i.message.contains("resourceName")));
        let mut no_multus = inv.clone();
        no_multus.multus_installed = false;
        assert!(preflight(&[], &[net(true, None)], &no_multus)
            .iter()
            .any(|i| i.message.contains("Multus")));
    }

    #[test]
    fn no_devices_means_no_issues_at_all() {
        assert!(preflight(&[], &[], &inventory(1, true)).is_empty());
    }

    // ── placement ──

    fn node(cpu_manager: bool, hp: &[(&str, u64)]) -> NodeDevices {
        NodeDevices {
            node: "n".into(),
            cpu_manager,
            hugepages: hp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            devices: BTreeMap::new(),
            builtin: BTreeMap::new(),
        }
    }

    #[test]
    fn quantities_and_hugepage_capacity_are_parsed() {
        assert_eq!(parse_bytes("2Mi"), Some(2 << 20));
        assert_eq!(parse_bytes("1Gi"), Some(1 << 30));
        assert_eq!(parse_bytes("4096"), Some(4096));
        assert_eq!(parse_bytes("x"), None);
        let a = alloc(&[
            ("hugepages-2Mi", "1Gi"),
            ("hugepages-1Gi", "0"),
            ("cpu", "4"),
        ]);
        let h = hugepages(&a);
        assert_eq!(h.get("2Mi"), Some(&512));
        assert_eq!(h.get("1Gi"), Some(&0));
        assert!(!h.contains_key("cpu"));
        let n = NodeDevices::from_node("n1", &a, &alloc(&[("kubevirt.io/cpumanager", "true")]));
        assert!(n.cpu_manager);
        assert!(
            !NodeDevices::from_node("n2", &a, &alloc(&[("kubevirt.io/cpumanager", "false")]))
                .cpu_manager
        );
    }

    #[test]
    fn numa_passthrough_follows_the_rules_kubevirts_webhook_enforces() {
        let numa = |d: bool, h: Option<&str>| PlacementRequest {
            dedicated_cpus: d,
            numa_passthrough: true,
            hugepages: h.map(String::from),
        };
        assert!(placement_rules(&numa(false, Some("2Mi")))
            .unwrap_err()
            .contains("dedicated"));
        assert!(placement_rules(&numa(true, None))
            .unwrap_err()
            .contains("hugepages"));
        assert!(placement_rules(&numa(true, Some("2Mi"))).is_ok());
        assert!(placement_rules(&numa(true, Some("lots"))).is_err());
        assert!(placement_rules(&PlacementRequest::default()).is_ok());
        assert!(PlacementRequest::default().is_empty());
    }

    #[test]
    fn dedicated_cpus_and_hugepages_need_a_node_that_can_give_them() {
        let req = PlacementRequest {
            dedicated_cpus: true,
            numa_passthrough: true,
            hugepages: Some("2Mi".into()),
        };
        let none = DeviceInventory {
            nodes: vec![node(false, &[("2Mi", 0)])],
            ..Default::default()
        };
        let issues = placement_preflight(&req, &none);
        let subjects: Vec<_> = issues
            .iter()
            .map(|i| (i.severity, i.subject.as_str()))
            .collect();
        assert!(subjects.contains(&("error", "dedicated-cpus")));
        assert!(subjects.contains(&("error", "hugepages")));
        let ok = DeviceInventory {
            nodes: vec![node(true, &[("2Mi", 512)])],
            ..Default::default()
        };
        let issues = placement_preflight(&req, &ok);
        assert!(!has_errors(&issues));
        assert_eq!(
            issues.len(),
            1,
            "only the NUMA migration warning: {issues:?}"
        );
        // A broken request is reported once, as a placement error, before looking at nodes.
        let bad = PlacementRequest {
            numa_passthrough: true,
            ..Default::default()
        };
        assert_eq!(placement_preflight(&bad, &ok)[0].subject, "placement");
        // No nodes readable: nothing to say.
        assert!(placement_preflight(
            &PlacementRequest {
                dedicated_cpus: true,
                ..Default::default()
            },
            &DeviceInventory::default()
        )
        .is_empty());
    }

    // ── permitting ──

    fn pci(name: &str, sel: &str) -> PermitRequest {
        PermitRequest::Pci {
            resource_name: name.into(),
            pci_vendor_selector: sel.into(),
            external_resource_provider: false,
        }
    }

    #[test]
    fn permit_requests_are_validated_and_normalised() {
        let e = pci("nvidia.com/GA102GL_A10", "10de:2236")
            .validated_entry()
            .unwrap();
        assert_eq!(e["pciVendorSelector"], "10DE:2236");
        assert!(e.get("externalResourceProvider").is_none());
        for bad in ["10DE", "10DE:22", "10DE:223G", "GG:2236"] {
            assert!(pci("nvidia.com/x", bad).validated_entry().is_err(), "{bad}");
        }
        for bad in ["noslash", "a/b/c", "devices.kubevirt.io/kvm", "a b/c"] {
            assert!(pci(bad, "10DE:2236").validated_entry().is_err(), "{bad}");
        }
        let m = PermitRequest::Mediated {
            resource_name: "nvidia.com/GRID_T4-2Q".into(),
            mdev_name_selector: "GRID T4-2Q".into(),
            external_resource_provider: true,
        };
        let e = m.validated_entry().unwrap();
        assert_eq!(
            (
                e["mdevNameSelector"].as_str(),
                e["externalResourceProvider"].as_bool()
            ),
            (Some("GRID T4-2Q"), Some(true))
        );
        assert!(PermitRequest::Mediated {
            resource_name: "nvidia.com/x".into(),
            mdev_name_selector: "bad;name".into(),
            external_resource_provider: false,
        }
        .validated_entry()
        .is_err());
        let u = PermitRequest::Usb {
            resource_name: "kubevirt.io/storage".into(),
            vendor: "46F4".into(),
            product: "0001".into(),
            external_resource_provider: false,
        };
        assert_eq!(
            u.validated_entry().unwrap()["selectors"][0]["vendor"],
            "46f4"
        );
        assert!(PermitRequest::Usb {
            resource_name: "kubevirt.io/storage".into(),
            vendor: "46".into(),
            product: "0001".into(),
            external_resource_provider: false,
        }
        .validated_entry()
        .is_err());
        // The JSON shape of the API.
        let r: PermitRequest = serde_json::from_value(json!({
            "kind": "pci", "resource_name": "nvidia.com/A10", "pci_vendor_selector": "10DE:2236"
        }))
        .unwrap();
        assert_eq!(r.resource_name(), "nvidia.com/A10");
    }

    #[test]
    fn permitting_builds_a_full_list_patch_with_the_version_we_read() {
        let cfg = json!({ "permittedHostDevices": { "pciHostDevices": [
            { "pciVendorSelector": "8086:1572", "resourceName": "intel.com/x710" } ] } });
        let patch = permit_patch(&cfg, &pci("nvidia.com/A10", "10de:2236"), Some("42"))
            .unwrap()
            .unwrap();
        let list = patch
            .pointer("/spec/configuration/permittedHostDevices/pciHostDevices")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            list.len(),
            2,
            "the existing entry is kept: a merge patch replaces the whole list"
        );
        assert_eq!(list[1]["resourceName"], "nvidia.com/A10");
        assert_eq!(patch["metadata"]["resourceVersion"], "42");
        assert!(
            permit_patch(&cfg, &pci("nvidia.com/A10", "10de:2236"), None)
                .unwrap()
                .unwrap()
                .get("metadata")
                .is_none()
        );
        // Same entry again: nothing to do. Same name, other selector or kind: conflict.
        assert_eq!(
            permit_patch(&cfg, &pci("intel.com/x710", "8086:1572"), None),
            Ok(None)
        );
        assert!(matches!(
            permit_patch(&cfg, &pci("intel.com/x710", "8086:1573"), None),
            Err(PermitError::Conflict(_))
        ));
        let mdev = PermitRequest::Mediated {
            resource_name: "intel.com/x710".into(),
            mdev_name_selector: "x".into(),
            external_resource_provider: false,
        };
        assert!(matches!(
            permit_patch(&cfg, &mdev, None),
            Err(PermitError::Conflict(_))
        ));
        assert!(matches!(
            permit_patch(&cfg, &pci("bad", "10DE:2236"), None),
            Err(PermitError::Invalid(_))
        ));
        // Empty configuration works too.
        let p = permit_patch(&json!({}), &pci("nvidia.com/A10", "10DE:2236"), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            p.pointer("/spec/configuration/permittedHostDevices/pciHostDevices/0/resourceName")
                .unwrap(),
            "nvidia.com/A10"
        );
    }

    #[test]
    fn unpermitting_removes_only_that_entry_and_reports_unknown_ones() {
        let cfg = json!({ "permittedHostDevices": {
            "pciHostDevices": [
                { "pciVendorSelector": "8086:1572", "resourceName": "intel.com/x710" },
                { "pciVendorSelector": "10DE:2236", "resourceName": "nvidia.com/A10" } ],
            "mediatedDevices": [ { "mdevNameSelector": "T4", "resourceName": "nvidia.com/T4" } ] } });
        let p = unpermit_patch(&cfg, "nvidia.com/A10", Some("7")).unwrap();
        let list = p
            .pointer("/spec/configuration/permittedHostDevices/pciHostDevices")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["resourceName"], "intel.com/x710");
        let p = unpermit_patch(&cfg, "nvidia.com/T4", None).unwrap();
        assert_eq!(
            p.pointer("/spec/configuration/permittedHostDevices/mediatedDevices")
                .unwrap(),
            &Value::Null,
            "the last entry removes the key instead of leaving an empty list"
        );
        assert!(matches!(
            unpermit_patch(&cfg, "nope.com/x", None),
            Err(PermitError::NotFound(_))
        ));
    }

    #[test]
    fn sriov_pools_show_free_virtual_functions_and_kinds_are_hinted() {
        let a = alloc(&[
            ("intel.com/sriov_netdevice", "8"),
            ("nvidia.com/GA102GL_A10", "1"),
        ]);
        let inv = DeviceInventory {
            nodes: vec![NodeDevices::from_allocatable("n1", &a)],
            ..Default::default()
        }
        .with_sriov_pools(&[
            (
                "default".into(),
                "vf-net".into(),
                "intel.com/sriov_netdevice".into(),
            ),
            ("default".into(), "empty".into(), "intel.com/none".into()),
        ])
        .with_kind_hints();
        assert_eq!((inv.sriov_pools[0].free, inv.sriov_pools[0].nodes), (8, 1));
        assert_eq!((inv.sriov_pools[1].free, inv.sriov_pools[1].nodes), (0, 0));
        assert_eq!(inv.kind_hints["nvidia.com/GA102GL_A10"], "gpu");
        assert_eq!(inv.kind_hints["intel.com/sriov_netdevice"], "host-device");
        assert_eq!(kind_hint("amd.com/gpu"), "gpu");
    }
}
