// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Fabric-compat CPU/memory/disk/NIC hotplug for running VMs.
//!
//! The request field names (`count`, `size_mb`, `path`, `bus`, `bridge`,
//! `model`) match `web/src/api/hotplug.ts` exactly — the frontend contract is
//! unchanged. `path`/`bridge` are reinterpreted server-side as the target
//! PVC/DataVolume name and Multus NetworkAttachmentDefinition name
//! respectively, since that is what KubeVirt's addvolume/addinterface
//! subresources actually take (not a filesystem path or bridge device).

use super::*;
use crate::kube::HotplugOutcome;
use axum::extract::Json as AxumJson;
use serde_json::json;

/// Merge the live-convergence outcome of a CPU/memory hotplug into the VM's
/// JSON representation, so the caller can tell "accepted" from "actually
/// applied" -- see `HotplugOutcome`.
fn with_hotplug_outcome(
    mut vm_json: serde_json::Value,
    outcome: HotplugOutcome,
) -> axum::response::Response {
    let (status, converged, restart_required) = match outcome {
        HotplugOutcome::Converged => (StatusCode::OK, true, false),
        HotplugOutcome::RestartRequired => (StatusCode::ACCEPTED, false, true),
        HotplugOutcome::Pending => (StatusCode::ACCEPTED, false, false),
    };
    if let Some(obj) = vm_json.as_object_mut() {
        obj.insert("hotplug_converged".to_string(), json!(converged));
        obj.insert("restart_required".to_string(), json!(restart_required));
    }
    (status, Json(vm_json)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct HotplugCpuBody {
    pub count: u32,
}

#[derive(Debug, Deserialize)]
pub struct HotplugMemoryBody {
    pub size_mb: u64,
}

#[derive(Debug, Deserialize)]
pub struct HotplugDiskBody {
    pub path: String,
    #[serde(default)]
    pub bus: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HotplugNicBody {
    pub bridge: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub model: Option<String>,
}

fn hotplug_error_response(action: &str, e: impl std::fmt::Display) -> axum::response::Response {
    // classify_hotplug_error is itself the sanitizer here — it inspects the
    // raw error text and returns an already-safe message. Running the
    // generic sanitize_error() first would collapse our own structured
    // errors (which don't start with kube-rs's "ApiError" prefix) to
    // "Internal server error" before classify_hotplug_error ever sees them.
    let raw = e.to_string();
    let (code, kind, msg) = crate::kube::lifecycle::classify_hotplug_error(action, &raw);
    // classify_hotplug_error's pattern matching on raw k8s/subresource error
    // text is inherently heuristic (e.g. a bare 404 with no structured
    // "NotFound" reason could mean the object is missing, or that the
    // addinterface/removeinterface subresource route itself doesn't exist
    // on this KubeVirt version) — log the raw text on every non-2xx outcome
    // so a misclassification is diagnosable server-side instead of only
    // showing the client a possibly-misleading classified message.
    log::debug!("{action} classified as {kind} ({code}) from raw: {raw}");
    let (st, j) = err_json(code, kind, &msg);
    (st, j).into_response()
}

pub async fn fabric_hotplug_cpu(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<HotplugCpuBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.hotplug_cpu(&namespace, &name, body.count).await {
        Ok((vm, outcome)) => {
            let vm_json = fabric_vm_json(&VmInfo::from_vm_with_ip(&vm, None));
            with_hotplug_outcome(vm_json, outcome)
        }
        Err(e) => hotplug_error_response("hotplug CPU", e),
    }
}

pub async fn fabric_hotplug_memory(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<HotplugMemoryBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let current_mib = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm
            .spec
            .template
            .spec
            .domain
            .memory
            .as_ref()
            .and_then(|m| m.guest.as_deref())
            .and_then(crate::storage::parse_size_to_bytes)
            .map(|b| b / (1024 * 1024))
            .unwrap_or(0),
        Err(e) => return hotplug_error_response("hotplug memory", e),
    };
    let target_mib = current_mib.saturating_add(body.size_mb);
    let target = if target_mib >= 1024 && target_mib % 1024 == 0 {
        format!("{}Gi", target_mib / 1024)
    } else {
        format!("{target_mib}Mi")
    };

    match client.hotplug_memory(&namespace, &name, &target).await {
        Ok((vm, outcome)) => {
            let vm_json = fabric_vm_json(&VmInfo::from_vm_with_ip(&vm, None));
            with_hotplug_outcome(vm_json, outcome)
        }
        Err(e) => hotplug_error_response("hotplug memory", e),
    }
}

pub async fn fabric_hotplug_disk(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<HotplugDiskBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let volume_name = format!(
        "hotplug-{}",
        body.path.trim().trim_start_matches('/').replace('/', "-")
    );
    match client
        .add_volume(
            &namespace,
            &name,
            &volume_name,
            body.path.trim(),
            body.bus.as_deref(),
            false,
        )
        .await
    {
        Ok(()) => Json(json!({ "device_id": volume_name, "claim": body.path })).into_response(),
        Err(e) => hotplug_error_response("hotplug disk", e),
    }
}

pub async fn fabric_hotunplug_disk(
    State(state): State<SharedState>,
    Path((name, device_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.remove_volume(&namespace, &name, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => hotplug_error_response("hot-remove disk", e),
    }
}

pub async fn fabric_hotplug_nic(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<HotplugNicBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let iface_name = format!("hotplug-{}", body.bridge.trim());
    match client
        .add_interface(&namespace, &name, &iface_name, body.bridge.trim())
        .await
    {
        Ok(()) => Json(json!({ "device_id": iface_name, "network": body.bridge })).into_response(),
        Err(e) => hotplug_error_response("hotplug NIC", e),
    }
}

pub async fn fabric_hotunplug_nic(
    State(state): State<SharedState>,
    Path((name, device_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.remove_interface(&namespace, &name, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => hotplug_error_response("hot-remove NIC", e),
    }
}
