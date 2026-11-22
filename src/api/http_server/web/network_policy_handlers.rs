// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real Kubernetes `NetworkPolicy` CRUD, scoped to VMs by matching KubeVirt's
//! own `kubevirt.io/domain` pod label. This replaces the deleted
//! `network-security/PoliciesTab.tsx`, which assumed a Cilium-identity /
//! host-agent "adopt" architecture with zero real backing -- rebuilt here
//! against the plain, native k8s object instead (same relationship as
//! Quotas -> `ResourceQuota`).

use super::*;
use axum::extract::Json as AxumJson;
use k8s_openapi::api::networking::v1::{
    IPBlock, NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicyPeer,
    NetworkPolicyPort, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use serde_json::json;
use std::collections::BTreeMap;

/// The pod label KubeVirt puts on every VMI's virt-launcher pod -- letting a
/// NetworkPolicy target "this VM" instead of an arbitrary label selector.
const VM_POD_LABEL: &str = "kubevirt.io/domain";

fn np_error(action: &str, e: impl std::fmt::Display) -> axum::response::Response {
    let raw = sanitize_error(&e);
    let lower = raw.to_ascii_lowercase();
    let (code, kind) = if lower.contains("notfound") || lower.contains("not found") {
        (404, "NOT_FOUND")
    } else if lower.contains("already exists") || lower.contains("conflict") {
        (409, "CONFLICT")
    } else {
        (500, "NETWORK_POLICY_FAILED")
    };
    let (st, j) = err_json(code, kind, &format!("Failed to {action}: {raw}"));
    (st, j).into_response()
}

fn port_to_string(p: &IntOrString) -> String {
    match p {
        IntOrString::Int(n) => n.to_string(),
        IntOrString::String(s) => s.clone(),
    }
}

fn rules_out<'a, T>(
    rules: &'a [T],
    peers: impl Fn(&'a T) -> Option<&'a Vec<NetworkPolicyPeer>>,
    ports: impl Fn(&'a T) -> Option<&'a Vec<NetworkPolicyPort>>,
) -> Vec<serde_json::Value> {
    rules
        .iter()
        .map(|r| {
            let cidr = peers(r)
                .and_then(|p| p.first())
                .and_then(|p| p.ip_block.as_ref())
                .map(|b| b.cidr.clone());
            let port_rule = ports(r).and_then(|p| p.first());
            json!({
                "cidr": cidr,
                "protocol": port_rule.and_then(|p| p.protocol.clone()),
                "port": port_rule.and_then(|p| p.port.as_ref()).map(port_to_string),
            })
        })
        .collect()
}

fn policy_json(p: &NetworkPolicy) -> serde_json::Value {
    let spec = p.spec.clone().unwrap_or_default();
    let vm_name = spec
        .pod_selector
        .as_ref()
        .and_then(|s| s.match_labels.as_ref())
        .and_then(|m| m.get(VM_POD_LABEL))
        .cloned();
    let ingress = rules_out(
        spec.ingress.as_deref().unwrap_or(&[]),
        |r: &NetworkPolicyIngressRule| r.from.as_ref(),
        |r: &NetworkPolicyIngressRule| r.ports.as_ref(),
    );
    let egress = rules_out(
        spec.egress.as_deref().unwrap_or(&[]),
        |r: &NetworkPolicyEgressRule| r.to.as_ref(),
        |r: &NetworkPolicyEgressRule| r.ports.as_ref(),
    );
    json!({
        "name": p.metadata.name,
        "vm_name": vm_name,
        "ingress": ingress,
        "egress": egress,
        "created": p.metadata.creation_timestamp.as_ref().map(|t| t.0.to_string()),
    })
}

pub async fn list_network_policies_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
    match api.list(&ListParams::default()).await {
        Ok(list) => Json(list.items.iter().map(policy_json).collect::<Vec<_>>()).into_response(),
        Err(e) => np_error("list network policies", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RuleIn {
    #[serde(default)]
    pub cidr: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNetworkPolicyBody {
    pub name: String,
    /// Scopes the policy to one VM's pod via KubeVirt's `kubevirt.io/domain`
    /// label; omit to select every pod in the namespace.
    #[serde(default)]
    pub vm_name: Option<String>,
    #[serde(default)]
    pub ingress: Vec<RuleIn>,
    #[serde(default)]
    pub egress: Vec<RuleIn>,
}

fn peer_from(cidr: &Option<String>) -> Option<Vec<NetworkPolicyPeer>> {
    cidr.as_ref().map(|c| {
        vec![NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: c.clone(),
                except: None,
            }),
            namespace_selector: None,
            pod_selector: None,
        }]
    })
}

fn ports_from(protocol: &Option<String>, port: &Option<u16>) -> Option<Vec<NetworkPolicyPort>> {
    port.map(|p| {
        vec![NetworkPolicyPort {
            end_port: None,
            port: Some(IntOrString::Int(p as i32)),
            protocol: Some(protocol.clone().unwrap_or_else(|| "TCP".to_string())),
        }]
    })
}

pub async fn create_network_policy_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<CreateNetworkPolicyBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "Policy name is required");
        return (st, j).into_response();
    }

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let mut pod_selector = LabelSelector::default();
    if let Some(vm) = &body.vm_name {
        let mut labels = BTreeMap::new();
        labels.insert(VM_POD_LABEL.to_string(), vm.clone());
        pod_selector.match_labels = Some(labels);
    }

    let ingress: Vec<NetworkPolicyIngressRule> = body
        .ingress
        .iter()
        .map(|r| NetworkPolicyIngressRule {
            from: peer_from(&r.cidr),
            ports: ports_from(&r.protocol, &r.port),
        })
        .collect();
    let egress: Vec<NetworkPolicyEgressRule> = body
        .egress
        .iter()
        .map(|r| NetworkPolicyEgressRule {
            to: peer_from(&r.cidr),
            ports: ports_from(&r.protocol, &r.port),
        })
        .collect();

    let mut policy_types = Vec::new();
    if !ingress.is_empty() {
        policy_types.push("Ingress".to_string());
    }
    if !egress.is_empty() {
        policy_types.push("Egress".to_string());
    }
    if policy_types.is_empty() {
        // No rules at all means "deny all" for whichever types are declared;
        // default to Ingress so the policy is at least meaningful.
        policy_types.push("Ingress".to_string());
    }

    let policy = NetworkPolicy {
        metadata: ObjectMeta {
            name: Some(body.name.clone()),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        spec: Some(NetworkPolicySpec {
            pod_selector: Some(pod_selector),
            ingress: if ingress.is_empty() {
                None
            } else {
                Some(ingress)
            },
            egress: if egress.is_empty() {
                None
            } else {
                Some(egress)
            },
            policy_types: Some(policy_types),
        }),
    };

    let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
    let result = api.create(&PostParams::default(), &policy).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Create,
        "network-policy",
        &body.name,
        result.is_ok(),
        result.as_ref().err().map(|e| sanitize_error(e)),
    )
    .await;
    match result {
        Ok(created) => (StatusCode::CREATED, Json(policy_json(&created))).into_response(),
        Err(e) => np_error("create network policy", e),
    }
}

pub async fn delete_network_policy_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.kube_client.client();
    drop(s);

    let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
    let result = api.delete(&name, &DeleteParams::default()).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Delete,
        "network-policy",
        &name,
        result.is_ok(),
        result.as_ref().err().map(|e| sanitize_error(e)),
    )
    .await;
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => np_error("delete network policy", e),
    }
}
