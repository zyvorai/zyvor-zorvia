// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Port-forward / expose helpers for KubeVirt VMs (NodePort Services).

use anyhow::{Context, Result};
use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{DeleteParams, ListParams, PostParams},
    Api,
};
use serde::Serialize;
use std::collections::BTreeMap;

use super::KubeClient;

pub const LABEL_VM: &str = "zorvia.io/vm";
pub const LABEL_MANAGED: &str = "zorvia.io/managed";
pub const LABEL_GUEST_PORT: &str = "zorvia.io/guest-port";
pub const LABEL_HOST_PORT: &str = "zorvia.io/host-port";
pub const LABEL_PROTOCOL: &str = "zorvia.io/protocol";

#[derive(Debug, Clone, Serialize)]
pub struct PortForwardInfo {
    pub host_port: i32,
    pub guest_port: i32,
    pub protocol: String,
    pub service_name: String,
    pub node_port: Option<i32>,
    pub expose_host: String,
}

impl KubeClient {
    fn svc_api(&self, namespace: &str) -> Api<Service> {
        Api::namespaced(self.client.clone(), namespace)
    }

    /// List NodePort/ClusterIP services managed for a VM.
    pub async fn list_port_forwards(
        &self,
        namespace: &str,
        vm_name: &str,
    ) -> Result<Vec<PortForwardInfo>> {
        let svcs = self.svc_api(namespace);
        let lp = ListParams::default().labels(&format!("{}={}", LABEL_VM, vm_name));
        let list = svcs.list(&lp).await?;
        let expose_host = expose_host();
        let mut out = Vec::new();
        for svc in list.items {
            let meta = &svc.metadata;
            let labels = meta.labels.clone().unwrap_or_default();
            if labels.get(LABEL_MANAGED).map(|v| v.as_str()) != Some("true") {
                continue;
            }
            let guest_port = labels
                .get(LABEL_GUEST_PORT)
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    svc.spec
                        .as_ref()
                        .and_then(|s| s.ports.as_ref())
                        .and_then(|p| p.first())
                        .map(|p| p.port)
                })
                .unwrap_or(0);
            let protocol = labels
                .get(LABEL_PROTOCOL)
                .cloned()
                .unwrap_or_else(|| "tcp".into());
            let node_port = svc
                .spec
                .as_ref()
                .and_then(|s| s.ports.as_ref())
                .and_then(|p| p.first())
                .and_then(|p| p.node_port);
            let host_port = labels
                .get(LABEL_HOST_PORT)
                .and_then(|s| s.parse().ok())
                .or(node_port)
                .unwrap_or(0);
            out.push(PortForwardInfo {
                host_port,
                guest_port,
                protocol,
                service_name: meta.name.clone().unwrap_or_default(),
                node_port,
                expose_host: expose_host.clone(),
            });
        }
        Ok(out)
    }

    /// Create (or replace) a NodePort Service exposing a guest port on the VMI.
    pub async fn add_port_forward(
        &self,
        namespace: &str,
        vm_name: &str,
        guest_port: i32,
        protocol: &str,
        preferred_node_port: Option<i32>,
    ) -> Result<PortForwardInfo> {
        let svcs = self.svc_api(namespace);
        let proto = protocol.to_uppercase();
        let name = format!(
            "{}-p{}-{}",
            sanitize_dns(vm_name),
            guest_port,
            proto.to_lowercase()
        );

        // Delete existing with same name if present
        let _ = svcs.delete(&name, &DeleteParams::default()).await;

        let mut labels = BTreeMap::new();
        labels.insert(LABEL_VM.to_string(), vm_name.to_string());
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_GUEST_PORT.to_string(), guest_port.to_string());
        labels.insert(LABEL_PROTOCOL.to_string(), protocol.to_lowercase());
        if let Some(np) = preferred_node_port {
            labels.insert(LABEL_HOST_PORT.to_string(), np.to_string());
        }

        let mut selector = BTreeMap::new();
        selector.insert("kubevirt.io/domain".to_string(), vm_name.to_string());

        let mut port = ServicePort {
            name: Some("fwd".into()),
            port: guest_port,
            target_port: Some(IntOrString::Int(guest_port)),
            protocol: Some(proto.clone()),
            ..Default::default()
        };
        if let Some(np) = preferred_node_port {
            if (30000..=32767).contains(&np) {
                port.node_port = Some(np);
            }
        }

        let svc = Service {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                namespace: Some(namespace.to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                type_: Some("NodePort".into()),
                selector: Some(selector),
                ports: Some(vec![port]),
                ..Default::default()
            }),
            status: None,
        };

        let created = svcs
            .create(&PostParams::default(), &svc)
            .await
            .with_context(|| format!("create Service {}", name))?;

        let node_port = created
            .spec
            .as_ref()
            .and_then(|s| s.ports.as_ref())
            .and_then(|p| p.first())
            .and_then(|p| p.node_port);

        Ok(PortForwardInfo {
            host_port: node_port.unwrap_or(0),
            guest_port,
            protocol: protocol.to_lowercase(),
            service_name: name,
            node_port,
            expose_host: expose_host(),
        })
    }

    /// Delete a managed port-forward Service by host/node port.
    pub async fn remove_port_forward(
        &self,
        namespace: &str,
        vm_name: &str,
        host_port: i32,
    ) -> Result<()> {
        let list = self.list_port_forwards(namespace, vm_name).await?;
        let svcs = self.svc_api(namespace);
        for pf in list {
            if pf.host_port == host_port || pf.node_port == Some(host_port) {
                svcs.delete(&pf.service_name, &DeleteParams::default())
                    .await
                    .with_context(|| format!("delete Service {}", pf.service_name))?;
                return Ok(());
            }
        }
        anyhow::bail!("No port-forward found for host_port {}", host_port)
    }

    /// Delete all managed Services for a VM (used on VM delete).
    pub async fn delete_vm_port_forwards(&self, namespace: &str, vm_name: &str) -> Result<()> {
        let list = self.list_port_forwards(namespace, vm_name).await?;
        let svcs = self.svc_api(namespace);
        for pf in list {
            let _ = svcs
                .delete(&pf.service_name, &DeleteParams::default())
                .await;
        }
        Ok(())
    }
}

pub(crate) fn expose_host() -> String {
    std::env::var("ZORVIA_EXPOSE_HOST")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "127.0.0.1".into())
}

fn sanitize_dns(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if s.len() > 40 {
        s.truncate(40);
    }
    s.trim_matches('-').to_string()
}
