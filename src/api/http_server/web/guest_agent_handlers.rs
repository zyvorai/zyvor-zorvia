// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Read-only views into a running guest through the GuestKit (Zyvor) guest agent:
//! agent version/health/snapshot readiness and the guest's own package, user,
//! certificate, container and security inventories. See docs/GUEST_AGENT.md.
//!
//! Needs the Zyvor agent (`guest_agent: "zyvor"` at creation) to be connected; the
//! stock qemu-guest-agent does not speak these methods. Only a fixed read-only method
//! set is reachable (`crate::guest_rpc`).

use super::*;

const AGENT_TIMEOUT_SECS: u64 = 20;

/// Same name rule the other VM sub-resource handlers apply before using a VM name in a
/// Kubernetes label selector.
fn valid_vm_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !s.starts_with(['-', '.'])
        && !s.ends_with(['-', '.'])
}

async fn connected_client(
    state: &SharedState,
    name: &str,
) -> Result<(kube::Client, String), Box<axum::response::Response>> {
    if !valid_vm_name(name) {
        let (st, j) = err_json(400, "INVALID_NAME", "Invalid VM name");
        return Err(Box::new((st, j).into_response()));
    }
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.guest_ready_report(&namespace, name).await {
        Ok(r) if r.agent_connected => Ok((client.client(), namespace)),
        Ok(_) => {
            let (st, j) = err_json(
                409,
                "GUEST_AGENT_NOT_CONNECTED",
                "The guest agent is not connected (create the VM with guest_agent: \"zyvor\", or wait for it to start)",
            );
            Err(Box::new((st, j).into_response()))
        }
        Err(e) => {
            let (st, j) = err_json(404, "VM_NOT_FOUND", &sanitize_error(&e));
            Err(Box::new((st, j).into_response()))
        }
    }
}

/// `GET /vms/{name}/guest/agent`: agent version, health and snapshot readiness.
pub async fn guest_agent_info(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (kube, ns) = match connected_client(&state, &name).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let call = |m: &'static str| {
        let kube = kube.clone();
        let (ns, name) = (ns.clone(), name.clone());
        async move {
            crate::guest_rpc::call(
                &kube,
                &ns,
                &name,
                m,
                &serde_json::json!({}),
                AGENT_TIMEOUT_SECS,
            )
            .await
        }
    };
    let version = call("guestkit.getVersion").await;
    let version = match version {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(502, "GUEST_AGENT_ERROR", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let health = call("guestkit.getAgentHealth").await.ok();
    let readiness = call("guestkit.getSnapshotReadiness")
        .await
        .ok()
        .map(|r| crate::guest_rpc::summarize_readiness(&r));
    Json(serde_json::json!({
        "vm_name": name,
        "agent": "zyvor",
        "version": version,
        "health": health,
        "snapshot_readiness": readiness,
    }))
    .into_response()
}

/// `GET /vms/{name}/guest/inventory/{kind}` for packages, users, certificates, containers
/// and security.
pub async fn guest_inventory(
    State(state): State<SharedState>,
    Path((name, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(method) = crate::guest_rpc::inventory_method(&kind) else {
        let (st, j) = err_json(
            404,
            "UNKNOWN_INVENTORY",
            "Use one of: packages, users, certificates, containers, security",
        );
        return (st, j).into_response();
    };
    let (kube, ns) = match connected_client(&state, &name).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match crate::guest_rpc::call(
        &kube,
        &ns,
        &name,
        method,
        &serde_json::json!({}),
        AGENT_TIMEOUT_SECS,
    )
    .await
    {
        Ok(v) => Json(serde_json::json!({ "vm_name": name, "kind": kind, "inventory": v }))
            .into_response(),
        Err(e) => {
            let (st, j) = err_json(502, "GUEST_AGENT_ERROR", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}
