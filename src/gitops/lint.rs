// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Pre-flight lint for KubeVirt `VirtualMachine` manifests.
//!
//! Drift Guard compares two states; this answers a different question before
//! anything is applied: "is this VM definition going to bite me on day 2?".
//! Every rule is a pure check over one manifest (no cluster access), has a
//! stable ID, a severity, a JSON-pointer location and a remedy, and says
//! whether it blocks live migration, so the report can state plainly why a VM
//! would be stopped by a node drain instead of moved.
//!
//! The rules encode pitfalls that operators of KubeVirt report repeatedly:
//! bridge binding on the pod network and explicit masquerade ports 49152/49153
//! break live migration, host devices and `host-passthrough` pin a VM to a
//! node, ephemeral disks lose data on restart, and secrets pasted into inline
//! cloud-init end up in the VM object. See docs/VM_LINT.md.
//!
//! Hostile input is expected: the manifest may be any JSON value. Only
//! `get`/`as_*` accessors are used, finding counts are capped, and a finding
//! never repeats manifest content (a flagged secret is located, not echoed).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Upper bound on findings in one report (a manifest with thousands of bad
/// disks must not produce an unbounded response).
pub const MAX_FINDINGS: usize = 500;

/// Severity of a lint finding. Ordering is used by `--fail-on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LintSeverity {
    Info,
    Low,
    Medium,
    High,
}

impl LintSeverity {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

impl std::fmt::Display for LintSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintFinding {
    pub rule: String,
    pub severity: LintSeverity,
    /// RFC 6901 pointer into the manifest.
    pub pointer: String,
    pub message: String,
    pub remedy: String,
    /// True when this finding stops the VM from live-migrating.
    pub blocks_live_migration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintReport {
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub findings: Vec<LintFinding>,
    pub highest_severity: Option<LintSeverity>,
    /// `false` when any finding blocks live migration (a drain would stop the VM).
    pub live_migration_possible: bool,
    /// Rule IDs that block live migration.
    pub live_migration_blockers: Vec<String>,
    /// True when `MAX_FINDINGS` cut the list short.
    pub truncated: bool,
}

impl LintReport {
    pub fn meets_threshold(&self, threshold: LintSeverity) -> bool {
        self.highest_severity.is_some_and(|s| s >= threshold)
    }
}

/// One entry of the rule catalogue (for docs and `GET`-style listings).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RuleInfo {
    pub id: &'static str,
    pub severity: LintSeverity,
    pub blocks_live_migration: bool,
    pub title: &'static str,
}

pub const RULES: &[RuleInfo] = &[
    r(
        "ZL001",
        LintSeverity::High,
        true,
        "bridge binding on the pod network",
    ),
    r(
        "ZL002",
        LintSeverity::High,
        true,
        "masquerade ports 49152/49153 set explicitly",
    ),
    r(
        "ZL003",
        LintSeverity::Medium,
        false,
        "ephemeral disk loses data on restart",
    ),
    r(
        "ZL004",
        LintSeverity::Medium,
        true,
        "host device, GPU, SR-IOV or hostDisk pins the VM to a node",
    ),
    r(
        "ZL005",
        LintSeverity::Medium,
        false,
        "no memory size declared",
    ),
    r(
        "ZL006",
        LintSeverity::Low,
        false,
        "no evictionStrategy: a node drain stops the VM",
    ),
    r(
        "ZL007",
        LintSeverity::Medium,
        true,
        "cpu model host-passthrough limits migration to identical CPUs",
    ),
    r(
        "ZL008",
        LintSeverity::High,
        false,
        "credential or private key in inline cloud-init",
    ),
    r(
        "ZL009",
        LintSeverity::Low,
        false,
        "spec.running is deprecated, use runStrategy",
    ),
    r(
        "ZL010",
        LintSeverity::Low,
        false,
        "legacy disk bus or NIC model (ide/sata/e1000)",
    ),
    r(
        "ZL011",
        LintSeverity::High,
        false,
        "disk without a volume, volume without a disk, or interface without a network",
    ),
    r(
        "ZL012",
        LintSeverity::Medium,
        false,
        "opaque base64 cloud-init cannot be inspected",
    ),
    r(
        "ZL013",
        LintSeverity::Medium,
        true,
        "dedicated CPU placement pins the VM to a node",
    ),
];

const fn r(
    id: &'static str,
    severity: LintSeverity,
    blocks_live_migration: bool,
    title: &'static str,
) -> RuleInfo {
    RuleInfo {
        id,
        severity,
        blocks_live_migration,
        title,
    }
}

fn info(id: &str) -> &'static RuleInfo {
    RULES
        .iter()
        .find(|x| x.id == id)
        .expect("rule id is in the catalogue")
}

struct Sink {
    findings: Vec<LintFinding>,
    truncated: bool,
}

impl Sink {
    fn add(&mut self, id: &str, pointer: String, message: String, remedy: &str) {
        if self.findings.len() >= MAX_FINDINGS {
            self.truncated = true;
            return;
        }
        let i = info(id);
        self.findings.push(LintFinding {
            rule: id.to_string(),
            severity: i.severity,
            pointer,
            message,
            remedy: remedy.to_string(),
            blocks_live_migration: i.blocks_live_migration,
        });
    }
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Lint one manifest. `manifest` may be a `VirtualMachine` (the template spec
/// is linted) or a bare `VirtualMachineInstance`; anything else yields no
/// findings except for a structure note.
pub fn lint_vm(manifest: &Value) -> LintReport {
    let mut sink = Sink {
        findings: Vec::new(),
        truncated: false,
    };
    let name = manifest
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let namespace = manifest
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .map(str::to_string);

    let (base, vmspec) = if manifest.pointer("/spec/template/spec").is_some() {
        (
            "/spec/template/spec",
            manifest.pointer("/spec/template/spec"),
        )
    } else {
        ("/spec", manifest.get("spec"))
    };

    if let Some(spec) = vmspec.filter(|s| s.is_object()) {
        lint_spec(manifest, spec, base, &mut sink);
    }

    let mut blockers: BTreeSet<String> = BTreeSet::new();
    for f in &sink.findings {
        if f.blocks_live_migration {
            blockers.insert(f.rule.clone());
        }
    }
    let highest = sink.findings.iter().map(|f| f.severity).max();
    LintReport {
        name,
        namespace,
        findings: sink.findings,
        highest_severity: highest,
        live_migration_possible: blockers.is_empty(),
        live_migration_blockers: blockers.into_iter().collect(),
        truncated: sink.truncated,
    }
}

fn lint_spec(manifest: &Value, spec: &Value, base: &str, sink: &mut Sink) {
    let dom = spec.get("domain").unwrap_or(&Value::Null);
    let devices = dom.get("devices").unwrap_or(&Value::Null);
    let networks = arr(spec, "networks");
    let volumes = arr(spec, "volumes");
    let ifaces = arr(devices, "interfaces");
    let disks = arr(devices, "disks");

    // ZL001 / ZL002 / ZL010 (NIC model) / ZL011 (interface without network)
    for (i, nic) in ifaces.iter().enumerate() {
        let nname = text(nic, "name").unwrap_or("");
        let ptr = format!("{base}/domain/devices/interfaces/{i}");
        let net = networks.iter().find(|n| text(n, "name") == Some(nname));
        let on_pod = net.is_some_and(|n| n.get("pod").is_some());
        if nic.get("bridge").is_some() && on_pod {
            sink.add(
                "ZL001",
                format!("{ptr}/bridge"),
                format!(
                    "interface {:?} uses bridge binding on the pod network; KubeVirt cannot live-migrate it",
                    nname
                ),
                "use masquerade binding on the pod network, or a Multus secondary network",
            );
        }
        if let Some(ports) = nic.get("ports").and_then(Value::as_array) {
            let hits = ports
                .iter()
                .any(|p| matches!(p.get("port").and_then(Value::as_u64), Some(49152 | 49153)));
            if nic.get("masquerade").is_some() && hits {
                sink.add(
                    "ZL002",
                    format!("{ptr}/ports"),
                    "ports 49152 and 49153 are reserved for the migration proxy in virt-launcher; listing them breaks live migration".to_string(),
                    "remove those ports from the masquerade interface, or expose other ports",
                );
            }
        }
        if nic.get("sriov").is_some() {
            sink.add(
                "ZL004",
                format!("{ptr}/sriov"),
                format!("interface {nname:?} is SR-IOV, which pins the VM to its node"),
                "use masquerade or a bridge CNI network if the VM must move during maintenance",
            );
        }
        if let Some(model) = text(nic, "model") {
            if matches!(model, "e1000" | "e1000e" | "rtl8139" | "ne2k_pci" | "pcnet") {
                sink.add(
                    "ZL010",
                    format!("{ptr}/model"),
                    format!("NIC model {model:?} is emulated and slow"),
                    "use virtio unless the guest has no virtio driver",
                );
            }
        }
        if net.is_none() {
            sink.add(
                "ZL011",
                ptr,
                format!("interface {nname:?} has no matching entry in spec.networks"),
                "add a network with the same name, or remove the interface",
            );
        }
    }

    // ZL003 ephemeral volumes, ZL012 opaque cloud-init, ZL008 secrets
    for (i, vol) in volumes.iter().enumerate() {
        let ptr = format!("{base}/volumes/{i}");
        let vname = text(vol, "name").unwrap_or("");
        if vol.get("emptyDisk").is_some() || vol.get("containerDisk").is_some() {
            let kind = if vol.get("emptyDisk").is_some() {
                "emptyDisk"
            } else {
                "containerDisk"
            };
            // A containerDisk is a normal read-only boot image; only flag it when it is the sole
            // writable store. Keep the rule to emptyDisk plus containerDisk-without-a-PVC.
            let has_persistent = volumes.iter().any(|v| {
                v.get("persistentVolumeClaim").is_some()
                    || v.get("dataVolume").is_some()
                    || v.get("hostDisk").is_some()
            });
            if kind == "emptyDisk" || !has_persistent {
                sink.add(
                    "ZL003",
                    format!("{ptr}/{kind}"),
                    format!(
                        "volume {vname:?} ({kind}) is discarded when the VM restarts{}",
                        if kind == "containerDisk" {
                            " and the VM has no persistent volume at all"
                        } else {
                            ""
                        }
                    ),
                    "back data with a PersistentVolumeClaim or DataVolume",
                );
            }
        }
        if vol.get("hostDisk").is_some() {
            sink.add(
                "ZL004",
                format!("{ptr}/hostDisk"),
                format!("volume {vname:?} is a hostDisk, which lives on one node"),
                "use a PVC on shared storage",
            );
        }
        for key in ["cloudInitNoCloud", "cloudInitConfigDrive"] {
            let Some(ci) = vol.get(key) else { continue };
            for field in ["userData", "networkData"] {
                if let Some(t) = text(ci, field) {
                    if contains_credential(t) {
                        sink.add(
                            "ZL008",
                            format!("{ptr}/{key}/{field}"),
                            "inline cloud-init appears to contain a password or private key (value not shown); it is readable by anyone who can get the VM object".to_string(),
                            "move it to a Secret and reference it with userDataSecretRef / networkDataSecretRef, or use SSH public keys",
                        );
                    }
                }
            }
            for field in ["userDataBase64", "networkDataBase64"] {
                if ci.get(field).is_some() {
                    sink.add(
                        "ZL012",
                        format!("{ptr}/{key}/{field}"),
                        "base64 cloud-init is opaque to review and to this linter".to_string(),
                        "use a Secret reference or plain userData so changes are reviewable",
                    );
                }
            }
        }
    }

    // ZL011: disks vs volumes
    let vnames: BTreeSet<&str> = volumes.iter().filter_map(|v| text(v, "name")).collect();
    let dnames: BTreeSet<&str> = disks.iter().filter_map(|d| text(d, "name")).collect();
    for (i, d) in disks.iter().enumerate() {
        let ptr = format!("{base}/domain/devices/disks/{i}");
        let dn = text(d, "name").unwrap_or("");
        if !vnames.contains(dn) {
            sink.add(
                "ZL011",
                ptr.clone(),
                format!("disk {dn:?} has no matching volume"),
                "add a volume with the same name, or remove the disk",
            );
        }
        let bus = d
            .pointer("/disk/bus")
            .or_else(|| d.pointer("/cdrom/bus"))
            .and_then(Value::as_str);
        if matches!(bus, Some("ide" | "sata")) {
            sink.add(
                "ZL010",
                format!("{ptr}/disk/bus"),
                format!(
                    "disk {dn:?} uses the {} bus, which is much slower than virtio",
                    bus.unwrap_or("")
                ),
                "use bus: virtio (or scsi) where the guest supports it",
            );
        }
    }
    for (i, v) in volumes.iter().enumerate() {
        let vn = text(v, "name").unwrap_or("");
        if !dnames.contains(vn) {
            sink.add(
                "ZL011",
                format!("{base}/volumes/{i}"),
                format!("volume {vn:?} is not attached to any disk"),
                "add a disk with the same name, or remove the volume",
            );
        }
    }

    // ZL004 host devices / GPUs
    for key in ["gpus", "hostDevices"] {
        if !arr(devices, key).is_empty() {
            sink.add(
                "ZL004",
                format!("{base}/domain/devices/{key}"),
                format!("{key} pin the VM to a node with that hardware"),
                "accept downtime on drain, or avoid passthrough for this VM",
            );
        }
    }

    // ZL005 memory
    let has_mem = dom.pointer("/memory/guest").is_some()
        || dom.pointer("/resources/requests/memory").is_some()
        || dom.pointer("/resources/limits/memory").is_some()
        || dom.pointer("/memory/hugepages").is_some();
    if !has_mem {
        sink.add(
            "ZL005",
            format!("{base}/domain"),
            "no memory declared (memory.guest or resources.requests.memory)".to_string(),
            "set memory.guest explicitly so the VM and its scheduling are predictable",
        );
    }

    // ZL006 eviction strategy (spec level, VM template sets it on the VMI spec)
    if spec.get("evictionStrategy").is_none() {
        sink.add(
            "ZL006",
            base.to_string(),
            "evictionStrategy is not set; unless the cluster default is LiveMigrate a node drain stops this VM".to_string(),
            "set evictionStrategy: LiveMigrate (or LiveMigrateIfPossible)",
        );
    }

    // ZL007 host-passthrough
    if dom.pointer("/cpu/model").and_then(Value::as_str) == Some("host-passthrough") {
        sink.add(
            "ZL007",
            format!("{base}/domain/cpu/model"),
            "host-passthrough exposes the exact host CPU; migration only works between identical CPUs".to_string(),
            "use host-model, or a named model that every node supports",
        );
    }

    // ZL013 dedicated CPUs
    if dom
        .pointer("/cpu/dedicatedCpuPlacement")
        .and_then(Value::as_bool)
        == Some(true)
    {
        sink.add(
            "ZL013",
            format!("{base}/domain/cpu/dedicatedCpuPlacement"),
            "dedicated CPU placement pins vCPUs to host cores".to_string(),
            "only use it for latency-critical VMs and plan their maintenance separately",
        );
    }

    // ZL009 deprecated spec.running (VM level only)
    if manifest.pointer("/spec/running").is_some()
        && manifest.pointer("/spec/runStrategy").is_none()
    {
        sink.add(
            "ZL009",
            "/spec/running".to_string(),
            "spec.running is deprecated in KubeVirt".to_string(),
            "use spec.runStrategy: Always / Halted / Manual / RerunOnFailure",
        );
    }
}

/// True when inline cloud-init text looks like it carries a credential.
/// Deliberately conservative: key names with a non-boolean value, an
/// embedded `chpasswd` block or a PEM private key header.
fn contains_credential(t: &str) -> bool {
    // Bound the work on a hostile multi-megabyte value.
    let t = if t.len() > 256 * 1024 {
        &t[..floor_boundary(t, 256 * 1024)]
    } else {
        t
    };
    let lower = t.to_ascii_lowercase();
    if lower.contains("-----begin") && lower.contains("private key-----") {
        return true;
    }
    if lower.contains("| chpasswd") || (lower.contains("chpasswd:") && lower.contains("list:")) {
        return true;
    }
    for line in lower.lines() {
        let l = line.trim_start_matches([' ', '\t', '-']);
        for key in [
            "password",
            "passwd",
            "plain_text_passwd",
            "secret",
            "token",
            "api_key",
            "apikey",
            "api-key",
        ] {
            if let Some(rest) = l.strip_prefix(key) {
                if let Some(val) = rest.trim_start().strip_prefix(':') {
                    let v = val.trim().trim_matches(|c| c == '"' || c == '\'');
                    if !v.is_empty()
                        && !matches!(v, "true" | "false" | "null" | "~")
                        && !v.starts_with("{{")
                        && !v.starts_with("${")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vm(spec: Value) -> Value {
        json!({"kind":"VirtualMachine","metadata":{"name":"t","namespace":"ns"},"spec":{"template":{"spec":spec}}})
    }

    fn ids(r: &LintReport) -> Vec<&str> {
        r.findings.iter().map(|f| f.rule.as_str()).collect()
    }

    fn clean() -> Value {
        json!({
            "evictionStrategy":"LiveMigrate",
            "domain":{"memory":{"guest":"2Gi"},"devices":{
                "disks":[{"name":"root","disk":{"bus":"virtio"}}],
                "interfaces":[{"name":"default","masquerade":{}}]}},
            "networks":[{"name":"default","pod":{}}],
            "volumes":[{"name":"root","persistentVolumeClaim":{"claimName":"r"}}]
        })
    }

    #[test]
    fn clean_vm_has_no_findings() {
        let r = lint_vm(&vm(clean()));
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert!(r.live_migration_possible);
        assert_eq!(r.highest_severity, None);
        assert_eq!(r.name.as_deref(), Some("t"));
    }

    #[test]
    fn bridge_on_pod_network_blocks_migration() {
        let mut s = clean();
        s["domain"]["devices"]["interfaces"] = json!([{"name":"default","bridge":{}}]);
        let r = lint_vm(&vm(s));
        assert_eq!(ids(&r), vec!["ZL001"]);
        assert!(!r.live_migration_possible);
        assert_eq!(r.live_migration_blockers, vec!["ZL001"]);
        assert!(r.meets_threshold(LintSeverity::High));
    }

    #[test]
    fn bridge_on_multus_network_is_fine() {
        let mut s = clean();
        s["domain"]["devices"]["interfaces"] = json!([{"name":"vlan","bridge":{}}]);
        s["networks"] = json!([{"name":"vlan","multus":{"networkName":"br0"}}]);
        assert!(lint_vm(&vm(s)).findings.is_empty());
    }

    #[test]
    fn migration_ports_flagged() {
        let mut s = clean();
        s["domain"]["devices"]["interfaces"] =
            json!([{"name":"default","masquerade":{},"ports":[{"port":49152},{"port":80}]}]);
        assert_eq!(ids(&lint_vm(&vm(s))), vec!["ZL002"]);
    }

    #[test]
    fn ephemeral_and_hostdisk() {
        let mut s = clean();
        s["domain"]["devices"]["disks"] = json!([{"name":"root"},{"name":"scratch"},{"name":"h"}]);
        s["volumes"] = json!([
            {"name":"root","persistentVolumeClaim":{"claimName":"r"}},
            {"name":"scratch","emptyDisk":{"capacity":"1Gi"}},
            {"name":"h","hostDisk":{"path":"/x","type":"Disk"}}]);
        let r = lint_vm(&vm(s));
        assert_eq!(ids(&r), vec!["ZL003", "ZL004"]);
    }

    #[test]
    fn containerdisk_alone_flagged_but_with_pvc_ok() {
        let mut s = clean();
        s["volumes"] = json!([{"name":"root","containerDisk":{"image":"x"}}]);
        assert_eq!(ids(&lint_vm(&vm(s.clone()))), vec!["ZL003"]);
        s["domain"]["devices"]["disks"] =
            json!([{"name":"root","disk":{"bus":"virtio"}},{"name":"d","disk":{"bus":"virtio"}}]);
        s["volumes"] = json!([
            {"name":"root","containerDisk":{"image":"x"}},
            {"name":"d","persistentVolumeClaim":{"claimName":"d"}}]);
        assert!(lint_vm(&vm(s)).findings.is_empty());
    }

    #[test]
    fn devices_memory_eviction_cpu() {
        let s = json!({
            "domain":{"cpu":{"model":"host-passthrough","dedicatedCpuPlacement":true},
              "devices":{"gpus":[{"name":"g","deviceName":"nvidia.com/T4"}],
                "interfaces":[{"name":"s","sriov":{}}]}},
            "networks":[{"name":"s","multus":{"networkName":"sr"}}]
        });
        let r = lint_vm(&vm(s));
        let got: BTreeSet<&str> = ids(&r).into_iter().collect();
        for want in ["ZL004", "ZL005", "ZL006", "ZL007", "ZL013"] {
            assert!(got.contains(want), "{want} missing in {got:?}");
        }
        assert_eq!(r.live_migration_blockers, vec!["ZL004", "ZL007", "ZL013"]);
    }

    #[test]
    fn legacy_bus_and_nic_model() {
        let mut s = clean();
        s["domain"]["devices"]["disks"] = json!([{"name":"root","disk":{"bus":"sata"}}]);
        s["domain"]["devices"]["interfaces"] =
            json!([{"name":"default","masquerade":{},"model":"e1000"}]);
        assert_eq!(ids(&lint_vm(&vm(s))), vec!["ZL010", "ZL010"]);
    }

    #[test]
    fn dangling_references() {
        let mut s = clean();
        s["domain"]["devices"]["disks"] = json!([{"name":"gone"}]);
        s["domain"]["devices"]["interfaces"] = json!([{"name":"nonet","masquerade":{}}]);
        let r = lint_vm(&vm(s));
        let n = ids(&r).iter().filter(|i| **i == "ZL011").count();
        assert_eq!(n, 3, "{:?}", r.findings); // iface w/o net, disk w/o volume, volume w/o disk
    }

    #[test]
    fn running_is_deprecated() {
        let mut m = vm(clean());
        m["spec"]["running"] = json!(true);
        assert_eq!(ids(&lint_vm(&m)), vec!["ZL009"]);
        m["spec"]["runStrategy"] = json!("Always");
        assert!(lint_vm(&m).findings.is_empty());
    }

    fn with_cloud_init(user_data: &str) -> Value {
        let mut s = clean();
        s["domain"]["devices"]["disks"] = json!([
            {"name":"root","disk":{"bus":"virtio"}},{"name":"ci","disk":{"bus":"virtio"}}]);
        s["volumes"] = json!([
            {"name":"root","persistentVolumeClaim":{"claimName":"r"}},
            {"name":"ci","cloudInitNoCloud":{"userData":user_data}}]);
        vm(s)
    }

    #[test]
    fn credentials_flagged_and_never_echoed() {
        let secret = "Sup3rS3cretPassw0rd!";
        for ud in [
            format!("#cloud-config\npassword: {secret}\n"),
            format!("#cloud-config\nchpasswd:\n  list: |\n    root:{secret}\n"),
            format!("#cloud-config\nwrite_files:\n- content: |\n    -----BEGIN OPENSSH PRIVATE KEY-----\n    {secret}\n"),
            format!("#cloud-config\n  - TOKEN: \"{secret}\"\n"),
        ] {
            let r = lint_vm(&with_cloud_init(&ud));
            assert_eq!(ids(&r), vec!["ZL008"], "{ud}");
            let out = serde_json::to_string(&r).unwrap();
            assert!(!out.contains(secret), "secret leaked into report");
            assert!(r.findings[0].pointer.ends_with("/cloudInitNoCloud/userData"));
        }
    }

    #[test]
    fn benign_cloud_init_is_quiet() {
        let r = lint_vm(&with_cloud_init(
            "#cloud-config\nssh_authorized_keys:\n- ssh-ed25519 AAAA\nlock_passwd: true\npassword: false\npackages: [curl]\n",
        ));
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    #[test]
    fn base64_cloud_init_is_opaque() {
        let mut m = with_cloud_init("x");
        m["spec"]["template"]["spec"]["volumes"][1]["cloudInitNoCloud"] =
            json!({"userDataBase64":"cGFzc3dvcmQ6IGh1bnRlcjI="});
        let r = lint_vm(&m);
        assert_eq!(ids(&r), vec!["ZL012"]);
        assert!(!serde_json::to_string(&r).unwrap().contains("cGFzc3dvcmQ6"));
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        for m in [
            Value::Null,
            json!(42),
            json!("str"),
            json!([1, 2]),
            json!({"spec": 7}),
            json!({"spec":{"template":{"spec":"x"}}}),
            json!({"spec":{"template":{"spec":{"domain":5,"networks":"n","volumes":[1,null,{"name":3}],
                "interfaces":{}}}}}),
            json!({"spec":{"domain":{"devices":{"interfaces":[null,{"name":null,"ports":"x"}],"disks":[[],{"disk":[]}]}}}}),
        ] {
            let _ = lint_vm(&m);
        }
    }

    #[test]
    fn findings_are_capped() {
        let n = MAX_FINDINGS + 50;
        let disks: Vec<Value> = (0..n).map(|i| json!({"name": format!("d/{i}~")})).collect();
        let r = lint_vm(&json!({"spec":{"domain":{"devices":{"disks":disks}}}}));
        assert_eq!(r.findings.len(), MAX_FINDINGS);
        assert!(r.truncated);
    }

    #[test]
    fn huge_cloud_init_is_bounded_and_multibyte_safe() {
        let ud = "é".repeat(400_000);
        let _ = lint_vm(&with_cloud_init(&ud));
    }

    #[test]
    fn catalogue_matches_emitted_metadata() {
        for rule in RULES {
            assert_eq!(info(rule.id).id, rule.id);
        }
        let ids: BTreeSet<_> = RULES.iter().map(|x| x.id).collect();
        assert_eq!(ids.len(), RULES.len(), "duplicate rule id");
    }

    #[test]
    fn bare_vmi_is_linted() {
        let r = lint_vm(
            &json!({"kind":"VirtualMachineInstance","metadata":{"name":"i"},
            "spec":{"domain":{"devices":{"interfaces":[{"name":"d","bridge":{}}]}},"networks":[{"name":"d","pod":{}}]}}),
        );
        assert!(ids(&r).contains(&"ZL001"));
    }

    #[test]
    fn severity_parse_and_order() {
        assert!(LintSeverity::High > LintSeverity::Medium);
        assert_eq!(LintSeverity::parse("MED"), Some(LintSeverity::Medium));
        assert_eq!(LintSeverity::parse("critical"), None);
    }
}
