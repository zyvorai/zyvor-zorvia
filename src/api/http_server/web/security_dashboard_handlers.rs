// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Security dashboard, assembled entirely from real data this project has
//! already shipped -- no new subsystem invented for it:
//! - "listening ports" reuses the same real NodePort/ClusterIP Services
//!   `crate::kube::expose` creates per VM (`service_map_handlers.rs`).
//! - "failed logins" and "alerts" read the real `crate::audit_trail::AuditTrail`
//!   (now fed by real `auth_login` attempts and the VM lifecycle handlers).
//! - "risk_score" is an explicit, documented heuristic over those two real
//!   signals -- not a sourced/certified metric, and labeled as such below
//!   so nobody mistakes it for one.

use super::*;
use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, ListParams};
use serde_json::json;

pub async fn security_dashboard_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let kube_client = s.kube_client.clone();
    let audit = s.audit.clone();
    drop(s);

    let svc_api: Api<Service> = Api::namespaced(kube_client.client(), &namespace);
    let lp = ListParams::default().labels(&format!("{}=true", crate::kube::expose::LABEL_MANAGED));
    let listening_ports: Vec<serde_json::Value> = match svc_api.list(&lp).await {
        Ok(list) => list
            .items
            .iter()
            .map(|svc| {
                let labels = svc.metadata.labels.clone().unwrap_or_default();
                let vm_name = labels.get(crate::kube::expose::LABEL_VM).cloned();
                let host_port = labels
                    .get(crate::kube::expose::LABEL_HOST_PORT)
                    .and_then(|v| v.parse::<i32>().ok())
                    .or_else(|| {
                        svc.spec
                            .as_ref()
                            .and_then(|sp| sp.ports.as_ref())
                            .and_then(|p| p.first())
                            .and_then(|p| p.node_port)
                    });
                let protocol = labels
                    .get(crate::kube::expose::LABEL_PROTOCOL)
                    .cloned()
                    .unwrap_or_else(|| "tcp".into());
                json!({
                    "port": host_port,
                    "protocol": protocol,
                    "service_name": svc.metadata.name,
                    "vm_name": vm_name,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let trail = crate::audit_trail::read_fresh(&audit).await;
    let failed_logins: Vec<serde_json::Value> = trail
        .entries
        .iter()
        .rev()
        .filter(|e| e.action == crate::audit_trail::AuditAction::Login && !e.success)
        .take(50)
        .map(|e| {
            json!({
                "timestamp": e.timestamp.to_rfc3339(),
                "user": e.user,
            })
        })
        .collect();

    let alerts: Vec<serde_json::Value> = trail
        .entries
        .iter()
        .rev()
        .filter(|e| !e.success)
        .take(50)
        .map(|e| {
            let severity = match e.severity {
                crate::audit_trail::AuditSeverity::Critical => "critical",
                crate::audit_trail::AuditSeverity::High => "warning",
                _ => "info",
            };
            json!({
                "severity": severity,
                "message": format!("Failed to {:?} {} '{}'", e.action, e.resource_type, e.resource_name),
                "source": e.resource_type,
                "timestamp": e.timestamp.to_rfc3339(),
            })
        })
        .collect();

    // Heuristic, not a sourced metric: starts at 100, docked for real
    // recent failure signals. Never claims precision beyond that.
    let critical_count = trail
        .entries
        .iter()
        .rev()
        .take(100)
        .filter(|e| !e.success && e.severity == crate::audit_trail::AuditSeverity::High)
        .count();
    let risk_score =
        (100i64 - (failed_logins.len() as i64 * 5) - (critical_count as i64 * 15)).clamp(0, 100);
    drop(trail);

    Json(json!({
        "risk_score": risk_score,
        "alerts": alerts,
        "failed_logins": failed_logins,
        "listening_ports": listening_ports,
    }))
    .into_response()
}
