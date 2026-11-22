// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real HA/eviction-policy control for a VM -- KubeVirt's actual
//! `spec.template.spec.evictionStrategy` field on the `VirtualMachine`
//! object, not a simulated "Fault Tolerance" concept. vSphere-style
//! lockstep fault tolerance has no equivalent on KubeVirt; what KubeVirt
//! really offers is a per-VM policy for what happens when its node is
//! drained/evicted (`LiveMigrate`, `None`, or leaving it unset).

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

pub async fn get_ha_policy_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    match client.get_vm(&namespace, &name).await {
        Ok(vm) => Json(json!({
            "vm_name": name,
            "eviction_strategy": vm.spec.template.spec.eviction_strategy,
        }))
        .into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetHaPolicyBody {
    /// KubeVirt's real values: "LiveMigrate", "LiveMigrateIfPossible",
    /// "External", "None". Pass `null` to clear it (cluster default applies).
    pub eviction_strategy: Option<String>,
}

pub async fn set_ha_policy_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<SetHaPolicyBody>,
) -> impl IntoResponse {
    if let Some(v) = &body.eviction_strategy {
        const VALID: &[&str] = &["LiveMigrate", "LiveMigrateIfPossible", "External", "None"];
        if !VALID.contains(&v.as_str()) {
            let (st, j) = err_json(
                400,
                "INVALID_STRATEGY",
                &format!("eviction_strategy must be one of {VALID:?}"),
            );
            return (st, j).into_response();
        }
    }

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let patch = json!({
        "spec": {
            "template": {
                "spec": {
                    "evictionStrategy": body.eviction_strategy,
                }
            }
        }
    });
    let vms: kube::Api<crate::kube::types::VirtualMachine> =
        kube::Api::namespaced(client.client(), &namespace);
    match vms
        .patch(
            &name,
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&patch),
        )
        .await
    {
        Ok(vm) => Json(json!({
            "vm_name": name,
            "eviction_strategy": vm.spec.template.spec.eviction_strategy,
        }))
        .into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "HA_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}
