// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Disk resize (PVC grow) and real network interface listing — both exist
//! today only as CLI commands (`zorvia disk-expand`, `zorvia network-list`)
//! with no REST route. `network-list`'s CLI implementation only reads VM
//! spec config and always leaves MAC/IP empty; this combines spec config
//! with live VMI status so MAC/IP addresses are actually populated.

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct ResizeDiskBody {
    pub size: String,
}

pub async fn fabric_resize_disk(
    State(state): State<SharedState>,
    Path((name, disk_name)): Path<(String, String)>,
    AxumJson(body): AxumJson<ResizeDiskBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    match client
        .resize_disk(&namespace, &name, &disk_name, &body.size)
        .await
    {
        Ok(pvc) => {
            let new_size = pvc
                .spec
                .as_ref()
                .and_then(|sp| sp.resources.as_ref())
                .and_then(|r| r.requests.as_ref())
                .and_then(|req| req.get("storage"))
                .map(|q| q.0.clone());
            Json(json!({ "disk": disk_name, "size": new_size })).into_response()
        }
        Err(e) => {
            log::error!("resize_disk failed: {e}");
            // Classify on the raw text, not sanitize_error(&e): our own
            // anyhow! messages here (SHRINK_NOT_SUPPORTED, "no volumes",
            // "disk not found", ...) don't start with any of
            // sanitize_error's allowed prefixes, so running it first
            // collapsed every one of these to "Internal server error" and
            // (since the checks below then ran on that already-sanitized
            // text) every failure — including a plain "disk not found" —
            // fell through to the generic 500 RESIZE_FAILED branch.
            let raw = e.to_string();
            let lower = raw.to_ascii_lowercase();
            let (code, kind, msg) = if lower.contains("volume_not_found") {
                (404, "VOLUME_NOT_FOUND", raw.clone())
            } else if lower.contains("unsupported") {
                (501, "UNSUPPORTED", raw.clone())
            } else if lower.contains("shrink_not_supported") {
                (409, "SHRINK_NOT_SUPPORTED", raw.clone())
            } else if lower.contains("notfound") || lower.contains("not found") {
                (404, "NOT_FOUND", raw.clone())
            } else {
                (500, "RESIZE_FAILED", sanitize_error(&e))
            };
            let (st, j) = err_json(code, kind, &msg);
            (st, j).into_response()
        }
    }
}

pub async fn fabric_list_disks(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let spec = &vm.spec.template.spec;
    let disks = spec
        .domain
        .devices
        .as_ref()
        .and_then(|d| d.disks.clone())
        .unwrap_or_default();
    let volumes = spec.volumes.clone().unwrap_or_default();

    let mut result = Vec::new();
    for disk in disks {
        let volume = volumes.iter().find(|v| v.name == disk.name);
        let (source, pvc_name): (&str, Option<String>) = match volume {
            Some(v) if v.persistent_volume_claim.is_some() => (
                "pvc",
                v.persistent_volume_claim
                    .as_ref()
                    .map(|p| p.claim_name.clone()),
            ),
            Some(v) if v.data_volume.is_some() => {
                ("dataVolume", v.data_volume.as_ref().map(|d| d.name.clone()))
            }
            Some(v) if v.container_disk.is_some() => ("containerDisk", None),
            Some(v) if v.empty_disk.is_some() => ("blank", None),
            Some(v) if v.cloud_init_no_cloud.is_some() => ("cloudInit", None),
            _ => ("unknown", None),
        };
        let size = if let Some(pvc_name) = &pvc_name {
            client
                .get_pvc(&namespace, pvc_name)
                .await
                .ok()
                .and_then(|pvc| {
                    pvc.status
                        .as_ref()
                        .and_then(|st| st.capacity.as_ref())
                        .and_then(|c| c.get("storage"))
                        .or_else(|| {
                            pvc.spec
                                .as_ref()
                                .and_then(|sp| sp.resources.as_ref())
                                .and_then(|r| r.requests.as_ref())
                                .and_then(|req| req.get("storage"))
                        })
                        .map(|q| q.0.clone())
                })
        } else {
            None
        };
        let device_type = if disk.cdrom.is_some() {
            "cdrom"
        } else if disk.lun.is_some() {
            "lun"
        } else {
            "disk"
        };
        let bus = disk
            .disk
            .as_ref()
            .and_then(|d| d.bus.clone())
            .or_else(|| disk.cdrom.as_ref().and_then(|d| d.bus.clone()))
            .or_else(|| disk.lun.as_ref().and_then(|d| d.bus.clone()));
        result.push(json!({
            "name": disk.name,
            "boot_order": disk.boot_order,
            "device_type": device_type,
            "bus": bus,
            "source": source,
            "size": size,
            "resizable": pvc_name.is_some(),
        }));
    }

    Json(json!(result)).into_response()
}

pub async fn fabric_list_interfaces(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let vmi_interfaces = client
        .get_vmi(&namespace, &name)
        .await
        .ok()
        .and_then(|vmi| vmi.status)
        .map(|st| st.interfaces)
        .unwrap_or_default();

    let spec = &vm.spec.template.spec;
    let networks = spec.networks.clone().unwrap_or_default();
    let interfaces = spec
        .domain
        .devices
        .as_ref()
        .and_then(|d| d.interfaces.clone())
        .unwrap_or_default();

    let result: Vec<serde_json::Value> = interfaces
        .iter()
        .map(|iface| {
            let network = networks.iter().find(|n| n.name == iface.name);
            let network_type = network
                .map(|n| if n.multus.is_some() { "multus" } else { "pod" })
                .unwrap_or("pod");
            let live = vmi_interfaces
                .iter()
                .find(|v| v.name.as_deref() == Some(iface.name.as_str()));
            json!({
                "name": iface.name,
                "network": network.map(|n| n.name.clone()),
                "model": iface.model,
                "network_type": network_type,
                "mac_address": live
                    .and_then(|l| l.mac.clone())
                    .or_else(|| iface.mac_address.clone()),
                "ip_address": live.and_then(|l| l.ip_address.clone()),
                "ip_addresses": live.map(|l| l.ip_addresses.clone()).unwrap_or_default(),
                "interface_name": live.and_then(|l| l.interface_name.clone()),
            })
        })
        .collect();

    Json(json!(result)).into_response()
}
