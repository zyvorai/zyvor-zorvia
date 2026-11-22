// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Fleet-wide view of exposed VM services -- the real NodePort/ClusterIP
//! Services `crate::kube::expose` creates per VM (VNC/SSH/RDP/custom port
//! forwards), grouped across the whole namespace instead of one VM at a
//! time like `fabric_list_port_forwards`/VMDetails already show.

use super::*;
use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, ListParams};

#[derive(Debug, Serialize)]
pub struct ServiceMapEntry {
    pub vm_name: String,
    pub service_name: String,
    pub guest_port: i32,
    pub host_port: i32,
    pub node_port: Option<i32>,
    pub protocol: String,
    pub expose_host: String,
    pub cluster_ip: Option<String>,
}

pub async fn list_service_map_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let api: Api<Service> = Api::namespaced(client, &namespace);
    let lp = ListParams::default().labels(&format!("{}=true", crate::kube::expose::LABEL_MANAGED));
    match api.list(&lp).await {
        Ok(list) => {
            let expose_host = crate::kube::expose::expose_host();
            let entries: Vec<ServiceMapEntry> = list
                .items
                .into_iter()
                .filter_map(|svc| {
                    let labels = svc.metadata.labels.clone().unwrap_or_default();
                    let vm_name = labels.get(crate::kube::expose::LABEL_VM)?.clone();
                    let service_name = svc.metadata.name.clone().unwrap_or_default();
                    let first_port = svc
                        .spec
                        .as_ref()
                        .and_then(|sp| sp.ports.as_ref())
                        .and_then(|p| p.first());
                    let guest_port = labels
                        .get(crate::kube::expose::LABEL_GUEST_PORT)
                        .and_then(|v| v.parse().ok())
                        .or_else(|| first_port.map(|p| p.port))
                        .unwrap_or(0);
                    let node_port = first_port.and_then(|p| p.node_port);
                    let host_port = labels
                        .get(crate::kube::expose::LABEL_HOST_PORT)
                        .and_then(|v| v.parse().ok())
                        .or(node_port)
                        .unwrap_or(0);
                    let protocol = labels
                        .get(crate::kube::expose::LABEL_PROTOCOL)
                        .cloned()
                        .unwrap_or_else(|| "tcp".into());
                    let cluster_ip = svc.spec.as_ref().and_then(|sp| sp.cluster_ip.clone());
                    Some(ServiceMapEntry {
                        vm_name,
                        service_name,
                        guest_port,
                        host_port,
                        node_port,
                        protocol,
                        expose_host: expose_host.clone(),
                        cluster_ip,
                    })
                })
                .collect();
            Json(entries).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(500, "SERVICE_MAP_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}
