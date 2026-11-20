// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Talking to the GuestKit (Zyvor) guest agent from the control plane.
//!
//! The agent answers QEMU-guest-agent commands on the virtio channel KubeVirt already
//! connects, and any GuestKit JSON-RPC method through the `guestkit-rpc` envelope:
//! `{"execute":"guestkit-rpc","arguments":{"method":..,"params":..,"id":..}}`. Replies are
//! QGA-shaped (`{"return":..}`), so the channel is never left holding a reply the
//! libvirt agent client cannot parse.
//!
//! Zorvia reaches the channel with `virsh qemu-agent-command` inside the VM's
//! `virt-launcher` pod (the `pods/exec` permission it already has for the Pods page). The
//! arguments are passed as an argv vector, never through a shell.
//!
//! Only a fixed set of read-only methods can be called from here; guest exec, file and
//! configuration methods are deliberately not reachable.

use anyhow::{anyhow, bail, Context, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams, ListParams};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Inventory kinds exposed over the API and the agent method behind each.
pub const INVENTORY: &[(&str, &str)] = &[
    ("packages", "guestkit.packages.inventory"),
    ("users", "guestkit.users.inventory"),
    ("certificates", "guestkit.certificates.inventory"),
    ("containers", "guestkit.containers.inventory"),
    ("security", "guestkit.security.posture"),
];

/// Methods this module will send. Everything else is refused before leaving Zorvia.
const READ_ONLY: &[&str] = &[
    "guestkit.ping",
    "guestkit.getVersion",
    "guestkit.getCapabilities",
    "guestkit.getAgentHealth",
    "guestkit.getSnapshotReadiness",
    "guestkit.packages.inventory",
    "guestkit.users.inventory",
    "guestkit.certificates.inventory",
    "guestkit.containers.inventory",
    "guestkit.security.posture",
];

pub fn inventory_method(kind: &str) -> Option<&'static str> {
    INVENTORY.iter().find(|(k, _)| *k == kind).map(|(_, m)| *m)
}

pub fn is_allowed(method: &str) -> bool {
    READ_ONLY.contains(&method)
}

/// libvirt's name for a KubeVirt VM's domain.
pub fn domain_name(namespace: &str, vm: &str) -> String {
    format!("{namespace}_{vm}")
}

/// The `qemu-agent-command` payload for one RPC method.
pub fn request_json(method: &str, params: &Value) -> String {
    json!({
        "execute": "guestkit-rpc",
        "arguments": { "method": method, "params": params, "id": 1 }
    })
    .to_string()
}

/// The `virsh` argument vector (after `virsh`).
pub fn virsh_args(uri: &str, domain: &str, payload: &str, timeout_secs: u64) -> Vec<String> {
    [
        "-c",
        uri,
        "qemu-agent-command",
        domain,
        payload,
        "--timeout",
        &timeout_secs.to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Extract the result from `virsh qemu-agent-command` output.
pub fn parse_reply(out: &str) -> Result<Value> {
    let line = out
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!("no reply from the guest agent: {}", out.trim()))?;
    let v: Value = serde_json::from_str(line).context("guest agent reply is not JSON")?;
    if let Some(err) = v.get("error") {
        let desc = err
            .get("desc")
            .and_then(Value::as_str)
            .unwrap_or("guest agent error");
        bail!("{desc}");
    }
    v.get("return")
        .cloned()
        .ok_or_else(|| anyhow!("guest agent reply has no result"))
}

async fn launcher_pod(pods: &Api<Pod>, vm: &str) -> Result<String> {
    let list = pods
        .list(&ListParams::default().labels(&format!("vm.kubevirt.io/name={vm}")))
        .await?;
    list.items
        .into_iter()
        .filter(|p| {
            p.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .is_some_and(|ph| ph == "Running")
        })
        .filter_map(|p| p.metadata.name)
        .find(|n| n.starts_with("virt-launcher-"))
        .ok_or_else(|| anyhow!("VM '{vm}' has no running virt-launcher pod"))
}

async fn virsh(pods: &Api<Pod>, pod: &str, args: Vec<String>) -> Result<String> {
    let mut cmd = vec!["virsh".to_string()];
    cmd.extend(args);
    let mut attached = pods
        .exec(
            pod,
            cmd,
            &AttachParams::default()
                .container("compute")
                .stdin(false)
                .stdout(true)
                .stderr(true),
        )
        .await?;
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = attached.stdout() {
        o.read_to_string(&mut out).await?;
    }
    if let Some(mut e) = attached.stderr() {
        e.read_to_string(&mut err).await?;
    }
    let _ = attached.join().await;
    Ok(if out.trim().is_empty() { err } else { out })
}

/// Call one read-only agent method on `vm`.
pub async fn call(
    client: &kube::Client,
    namespace: &str,
    vm: &str,
    method: &str,
    params: &Value,
    timeout_secs: u64,
) -> Result<Value> {
    if !is_allowed(method) {
        bail!("guest agent method '{method}' is not available through Zorvia");
    }
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = launcher_pod(&pods, vm).await?;
    let domain = domain_name(namespace, vm);
    let payload = request_json(method, params);
    let overall = Duration::from_secs(timeout_secs + 10);
    let run = async {
        // Current virt-launcher images run libvirt in session mode; older ones in system mode.
        let mut last = String::new();
        for uri in ["qemu:///session", "qemu:///system"] {
            let out = virsh(
                &pods,
                &pod,
                virsh_args(uri, &domain, &payload, timeout_secs),
            )
            .await?;
            if out.contains("unexpected qemu URI path") || out.contains("failed to connect") {
                last = out;
                continue;
            }
            return parse_reply(&out);
        }
        Err(anyhow!(
            "cannot reach libvirt in the launcher pod: {}",
            last.trim()
        ))
    };
    tokio::time::timeout(overall, run)
        .await
        .map_err(|_| anyhow!("guest agent did not answer within {timeout_secs}s"))?
}

/// What the pre-snapshot hooks reported, for recording next to a snapshot or backup. Best
/// effort: a guest without the Zyvor agent, or an agent that does not answer, yields `None`
/// (the snapshot is then filesystem- or crash-consistent, as before).
///
/// `guestkit.getSnapshotReadiness` runs the guest's pre-snapshot hooks (database flush
/// scripts under `/etc/zyvor/hooks`) and reports whether quiescing is possible. It does not
/// freeze the filesystem: KubeVirt freezes and thaws around the snapshot itself through the
/// same agent, so freezing here as well would only lengthen the freeze.
pub async fn pre_snapshot_hooks(
    client: &crate::kube::KubeClient,
    namespace: &str,
    vm: &str,
) -> Option<Value> {
    // Skip the exec round trip for guests whose agent is not connected.
    match client.guest_ready_report(namespace, vm).await {
        Ok(r) if r.agent_connected => {}
        _ => return None,
    }
    let r = call(
        &client.client(),
        namespace,
        vm,
        "guestkit.getSnapshotReadiness",
        &json!({}),
        20,
    )
    .await
    .map_err(|e| log::debug!("guest hooks for {vm}: {e}"))
    .ok()?;
    Some(summarize_readiness(&r))
}

/// Evidence from inside a booted guest for a recovery drill: the agent answers, with its
/// version and its own health verdict. `None` when the Zyvor agent cannot be reached
/// (stock qemu-guest-agent guests, or no agent).
pub async fn probe(client: &crate::kube::KubeClient, namespace: &str, vm: &str) -> Option<Value> {
    let kube = client.client();
    let version = call(&kube, namespace, vm, "guestkit.getVersion", &json!({}), 15)
        .await
        .ok()?;
    let health = call(
        &kube,
        namespace,
        vm,
        "guestkit.getAgentHealth",
        &json!({}),
        15,
    )
    .await
    .ok();
    Some(json!({ "agent": "zyvor", "version": version, "health": health }))
}

pub fn summarize_readiness(r: &Value) -> Value {
    let hooks_ok = r
        .get("hook_results")
        .and_then(Value::as_array)
        .map(|h| {
            h.iter()
                .all(|x| x.get("success").and_then(Value::as_bool) != Some(false))
        })
        .unwrap_or(true);
    let quiesce = r
        .get("quiesce_supported")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "agent": "zyvor",
        "hooks_ok": hooks_ok,
        "hooks": r.get("hook_results").cloned().unwrap_or(json!([])),
        "quiesce_supported": quiesce,
        // What the snapshot can honestly be called.
        "consistency": if quiesce && hooks_ok { "application" } else if quiesce { "filesystem" } else { "crash" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_only_methods_are_allowed() {
        for (_, m) in INVENTORY {
            assert!(is_allowed(m), "{m}");
        }
        assert!(is_allowed("guestkit.getSnapshotReadiness"));
        for bad in [
            "guestkit.exec",
            "guestkit.snapshot.prepare",
            "guestkit.file.write",
            "guest-exec",
            "",
        ] {
            assert!(!is_allowed(bad), "{bad}");
        }
        assert_eq!(
            inventory_method("packages"),
            Some("guestkit.packages.inventory")
        );
        assert_eq!(inventory_method("rm -rf"), None);
    }

    #[test]
    fn request_is_the_guestkit_rpc_envelope() {
        let v: Value = serde_json::from_str(&request_json("guestkit.ping", &json!({}))).unwrap();
        assert_eq!(v["execute"], "guestkit-rpc");
        assert_eq!(v["arguments"]["method"], "guestkit.ping");
        assert_eq!(domain_name("default", "web"), "default_web");
        let a = virsh_args("qemu:///session", "default_web", "{\"x\":1}", 15);
        assert_eq!(a[2], "qemu-agent-command");
        assert_eq!(
            a[4], "{\"x\":1}",
            "payload is one argv entry, not shell-parsed"
        );
        assert_eq!(a[6], "15");
    }

    #[test]
    fn replies_are_parsed_and_errors_surface() {
        let ok = "{\"id\":1,\"return\":{\"version\":\"1.2.4\"}}\n\n";
        assert_eq!(parse_reply(ok).unwrap()["version"], "1.2.4");
        let err =
            "{\"error\":{\"class\":\"GenericError\",\"desc\":\"no active snapshot prepare\"}}";
        assert!(parse_reply(err)
            .unwrap_err()
            .to_string()
            .contains("no active snapshot prepare"));
        assert!(parse_reply("error: guest agent not available").is_err());
        assert!(parse_reply("{\"id\":1}").is_err());
        // libvirt prefixes errors with text; the JSON line is still found.
        assert!(parse_reply("noise\n{\"return\":1}").is_ok());
    }

    #[test]
    fn readiness_is_summarised_honestly() {
        let ok = summarize_readiness(&json!({
            "quiesce_supported": true,
            "hook_results": [{"name":"mysql-flush.sh","success":true}]
        }));
        assert_eq!(ok["consistency"], "application");
        let failed = summarize_readiness(&json!({
            "quiesce_supported": true,
            "hook_results": [{"name":"x","success":false}]
        }));
        assert_eq!(
            (failed["hooks_ok"].clone(), failed["consistency"].clone()),
            (json!(false), json!("filesystem"))
        );
        assert_eq!(
            summarize_readiness(&json!({"quiesce_supported": false}))["consistency"],
            "crash"
        );
    }
}
