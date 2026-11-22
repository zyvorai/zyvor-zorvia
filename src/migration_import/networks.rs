// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Explicit target NIC mappings for imported VMs. Zorvia applies these only
//! while the VM is stopped; it does not discover source port groups or create
//! network infrastructure. Multus attachments must already exist locally.

use super::dns_label;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn attachment_api(
    client: &kube::Client,
    namespace: &str,
) -> kube::Api<kube::core::DynamicObject> {
    let mut ar = kube::core::ApiResource::from_gvk(&kube::core::GroupVersionKind::gvk(
        "k8s.cni.cncf.io",
        "v1",
        "NetworkAttachmentDefinition",
    ));
    // This CRD's plural is not the default inferred from its Kind.
    ar.plural = "network-attachment-definitions".into();
    kube::Api::namespaced_with(client.clone(), namespace, &ar)
}

/// Validate that a named attachment has an inline CNI configuration. External
/// node-local CNI files cannot be verified by this API and are rejected here.
pub fn validate_attachment(attachment: &Value) -> Result<()> {
    let config = attachment["spec"]["config"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("attachment must have inline spec.config"))?;
    let cni: Value = serde_json::from_str(config)?;
    let plugin = |p: &Value| p["type"].as_str().is_some_and(|s| !s.is_empty());
    if !plugin(&cni)
        && !cni["plugins"]
            .as_array()
            .is_some_and(|p| !p.is_empty() && p.iter().all(plugin))
    {
        bail!("attachment CNI config must have a plugin type or a nonempty plugins list");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImportNetwork {
    pub name: String,
    /// Optional source port-group label for the operation's audit record.
    #[serde(default)]
    pub source_network: Option<String>,
    /// Same-namespace Multus NetworkAttachmentDefinition. None means the pod
    /// network with masquerade binding; a Multus NIC uses bridge binding.
    #[serde(default)]
    pub attachment: Option<String>,
    #[serde(default)]
    pub mac_address: Option<String>,
}

fn valid_mac(mac: &str) -> bool {
    let parts: Vec<_> = mac.split(':').collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2) {
        return false;
    }
    let bytes: Option<Vec<u8>> = parts
        .iter()
        .map(|p| u8::from_str_radix(p, 16).ok())
        .collect();
    bytes.is_some_and(|b| b[0] & 1 == 0 && b.iter().any(|n| *n != 0))
}

pub fn problems(networks: &[ImportNetwork]) -> Vec<String> {
    let mut errors = Vec::new();
    if networks.len() > 16 {
        errors.push("networks holds at most 16 NICs".into());
    }
    let mut names = HashSet::new();
    let mut macs = HashSet::new();
    let mut pod_count = 0;
    for (i, network) in networks.iter().enumerate() {
        let at = format!("networks[{i}]");
        if !dns_label(&network.name) || !names.insert(network.name.as_str()) {
            errors.push(format!("{at}.name must be a unique DNS label"));
        }
        match network.attachment.as_deref() {
            Some(name) if !dns_label(name) => errors.push(format!(
                "{at}.attachment must name a NetworkAttachmentDefinition in the target namespace"
            )),
            None => pod_count += 1,
            _ => {}
        }
        if let Some(mac) = &network.mac_address {
            if !valid_mac(mac) || !macs.insert(mac.to_ascii_lowercase()) {
                errors.push(format!(
                    "{at}.mac_address must be a unique nonzero unicast MAC"
                ));
            }
        }
        if let Some(source) = &network.source_network {
            if source.is_empty() || source.len() > 256 || source.chars().any(char::is_control) {
                errors.push(format!(
                    "{at}.source_network must be 1-256 characters without controls"
                ));
            }
        }
    }
    if pod_count > 1 {
        errors.push("networks may contain at most one pod network".into());
    }
    errors
}

/// Build the complete network/interface replacement with an optimistic
/// resourceVersion guard. No change is permitted on an already-running VM.
pub fn network_patch(vm: &Value, networks: &[ImportNetwork]) -> Result<Value> {
    let errors = problems(networks);
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    if networks.is_empty() {
        bail!("an empty mapping must keep the importer's default networking");
    }
    let spec = &vm["spec"];
    if spec["running"].as_bool() == Some(true)
        || spec["runStrategy"].as_str().is_some_and(|s| s != "Halted")
        || vm["status"]["created"].as_bool() == Some(true)
    {
        bail!("refusing to replace NICs on an active VM; stop it before mapping networks");
    }
    let version = vm["metadata"]["resourceVersion"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("VM has no resourceVersion"))?;
    let mut interfaces = Vec::new();
    let mut targets = Vec::new();
    for network in networks {
        let mut interface = json!({"name": network.name, "model": "virtio"});
        let mut target = json!({"name": network.name});
        if let Some(attachment) = &network.attachment {
            interface["bridge"] = json!({});
            target["multus"] = json!({"networkName": attachment});
        } else {
            interface["masquerade"] = json!({});
            target["pod"] = json!({});
        }
        if let Some(mac) = &network.mac_address {
            interface["macAddress"] = json!(mac);
        }
        interfaces.push(interface);
        targets.push(target);
    }
    Ok(json!({
        "metadata": {"resourceVersion": version},
        "spec": {"template": {"spec": {
            "domain": {"devices": {"interfaces": interfaces}},
            "networks": targets,
        }}},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(name: &str, attachment: Option<&str>) -> ImportNetwork {
        ImportNetwork {
            name: name.into(),
            source_network: None,
            attachment: attachment.map(String::from),
            mac_address: None,
        }
    }

    #[test]
    fn maps_multus_and_pod_networks_with_matching_interfaces() {
        let mut mapped = nic("prod", Some("prod-vlan"));
        mapped.mac_address = Some("52:54:00:12:34:56".into());
        let vm = json!({"metadata":{"resourceVersion":"123"},"spec":{"running":false}});
        let p = network_patch(&vm, &[nic("default", None), mapped]).unwrap();
        assert_eq!(p["metadata"]["resourceVersion"], "123");
        let spec = &p["spec"]["template"]["spec"];
        assert_eq!(spec["networks"][1]["multus"]["networkName"], "prod-vlan");
        assert!(spec["domain"]["devices"]["interfaces"][0]["masquerade"].is_object());
        assert_eq!(
            spec["domain"]["devices"]["interfaces"][1]["macAddress"],
            "52:54:00:12:34:56"
        );
        assert!(p["spec"].get("running").is_none());
    }

    #[test]
    fn refuses_running_or_unversioned_vms() {
        for state in [json!({"running":true}), json!({"runStrategy":"Always"})] {
            let vm = json!({"metadata":{"resourceVersion":"1"},"spec":state});
            assert!(network_patch(&vm, &[nic("prod", Some("vlan"))]).is_err());
        }
        assert!(network_patch(&json!({}), &[nic("prod", Some("vlan"))]).is_err());
        let vm = json!({"metadata":{"resourceVersion":"1"},"status":{"created":true}});
        assert!(network_patch(&vm, &[nic("prod", Some("vlan"))]).is_err());
    }

    #[test]
    fn rejects_ambiguous_or_cross_namespace_nics() {
        assert!(!problems(&[nic("eth0", None), nic("eth1", None)]).is_empty());
        assert!(!problems(&[nic("eth0", Some("other/vlan"))]).is_empty());
        assert!(!problems(&[nic("eth0", Some("vlan")), nic("eth0", Some("vlan"))]).is_empty());
        assert!(!problems(&vec![nic("eth0", Some("vlan")); 17]).is_empty());
    }

    #[test]
    fn rejects_multicast_zero_and_duplicate_macs() {
        for mac in [
            "ff:ff:ff:ff:ff:ff",
            "00:00:00:00:00:00",
            "52:54:00:00:00",
            "gg:00:00:00:00:01",
        ] {
            let mut network = nic("eth0", Some("vlan"));
            network.mac_address = Some(mac.into());
            assert!(!problems(&[network]).is_empty(), "{mac}");
        }
        let mut a = nic("eth0", Some("vlan"));
        a.mac_address = Some("52:54:00:AA:00:01".into());
        let mut b = a.clone();
        b.name = "eth1".into();
        b.mac_address = Some("52:54:00:aa:00:01".into());
        assert!(!problems(&[a, b]).is_empty());
    }

    #[test]
    fn validates_inline_attachment_config() {
        for config in [
            r#"{"type":"bridge"}"#,
            r#"{"plugins":[{"type":"bridge"},{"type":"tuning"}]}"#,
        ] {
            assert!(validate_attachment(&json!({"spec":{"config":config}})).is_ok());
        }
        for config in [
            "",
            "invalid-json",
            "{}",
            r#"{"plugins":[]}"#,
            r#"{"plugins":[{}]}"#,
        ] {
            assert!(validate_attachment(&json!({"spec":{"config":config}})).is_err());
        }
    }
}
