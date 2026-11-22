// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Namespace-wide PersistentVolumeClaim inventory -- real k8s objects,
//! filling the one genuine gap left after resize (`fabric_resize_disk`) and
//! attach/detach (VM hotplug-disk routes) already covered per-VM disk
//! operations: there was no way to see every volume in the namespace at
//! once, only one VM's disks at a time (`/vms/:name/disks`).

use super::*;
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::api::{Api, ListParams};
use serde_json::json;
use std::collections::BTreeMap;

pub async fn list_storage_volumes_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let kube_client = s.kube_client.clone();
    drop(s);

    let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(kube_client.client(), &namespace);
    let pvcs = match pvc_api.list(&ListParams::default()).await {
        Ok(l) => l.items,
        Err(e) => {
            let (st, j) = err_json(500, "LIST_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    // PVC name -> VM names that reference it, from each VM's own volume list
    // (KubeVirt has no reverse index for this).
    let mut attached_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Ok(vms) = kube_client.list_vms(&namespace).await {
        for vm in &vms {
            let vm_name = vm.metadata.name.clone().unwrap_or_default();
            if let Some(volumes) = &vm.spec.template.spec.volumes {
                for vol in volumes {
                    // A `dataVolume` source realizes as a PVC of the same
                    // name once CDI import completes, same as a direct
                    // `persistentVolumeClaim` reference.
                    let claim_name = vol
                        .persistent_volume_claim
                        .as_ref()
                        .map(|p| p.claim_name.clone())
                        .or_else(|| vol.data_volume.as_ref().map(|dv| dv.name.clone()));
                    if let Some(claim_name) = claim_name {
                        attached_by
                            .entry(claim_name)
                            .or_default()
                            .push(vm_name.clone());
                    }
                }
            }
        }
    }

    let out: Vec<_> = pvcs
        .into_iter()
        .map(|pvc| {
            let name = pvc.metadata.name.clone().unwrap_or_default();
            let spec = pvc.spec.as_ref();
            let status = pvc.status.as_ref();
            let capacity = status
                .and_then(|st| st.capacity.as_ref())
                .and_then(|c| c.get("storage"))
                .map(|q| q.0.clone())
                .or_else(|| {
                    spec.and_then(|sp| sp.resources.as_ref())
                        .and_then(|r| r.requests.as_ref())
                        .and_then(|req| req.get("storage"))
                        .map(|q| q.0.clone())
                });
            json!({
                "name": name,
                "status": status.and_then(|st| st.phase.clone()).unwrap_or_else(|| "Unknown".into()),
                "capacity": capacity,
                "storage_class": spec.and_then(|sp| sp.storage_class_name.clone()),
                "access_modes": spec.and_then(|sp| sp.access_modes.clone()).unwrap_or_default(),
                "attached_vms": attached_by.get(&name).cloned().unwrap_or_default(),
                "created": pvc.metadata.creation_timestamp.as_ref().map(|t| t.0.to_string()),
            })
        })
        .collect();

    Json(out).into_response()
}
