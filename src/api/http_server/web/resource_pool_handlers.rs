// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Resource pools (Experimental): `/v1/inventory/pools*`. Membership is a VM label, pool
//! definitions (limits) live in the shared state store; the logic is in
//! `crate::vcenter_ops::pools`. All routes need `cluster.admin` (they list VMs of every
//! namespace) and `ZORVIA_EXPERIMENTAL=1`.

use super::*;
use crate::vcenter_ops::pools::{self, PoolDef, PoolLimits, PoolStore, PoolVm};
use axum::extract::{Json as AxumJson, Query};
use kube::api::{Patch, PatchParams};
use serde_json::json;

fn gate() -> Option<axum::response::Response> {
    if crate::features::allow_non_ga(pools::FEATURE_ID) {
        return None;
    }
    let (st, j) = err_json(
        403,
        "EXPERIMENTAL_DISABLED",
        "resource-pools is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable",
    );
    Some((st, j).into_response())
}

fn path() -> std::path::PathBuf {
    PoolStore::default_path()
}

fn ok(v: serde_json::Value) -> axum::response::Response {
    Json(json!({ "success": true, "data": v })).into_response()
}

fn bad(status: u16, code: &str, msg: &str) -> axum::response::Response {
    let (st, j) = err_json(status, code, msg);
    (st, j).into_response()
}

fn is_not_found(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<kube::Error>(), Some(kube::Error::Api(ae)) if ae.code == 404)
}

/// Every VM of every namespace, reduced to what the pool code needs.
#[allow(clippy::result_large_err)]
async fn load_vms(state: &SharedState) -> Result<Vec<PoolVm>, axum::response::Response> {
    let (client, ns) = {
        let s = state.read().await;
        (s.client(), s.namespace.clone())
    };
    match client.list_all_vms().await {
        Ok(vms) => Ok(vms.iter().map(|v| PoolVm::from_vm(v, &ns)).collect()),
        Err(e) => Err(bad(502, "LIST_FAILED", &sanitize_error(&e))),
    }
}

/// Advisory create-time check used by VM create: warnings for the pools named in `labels`.
/// Never blocks; a failure to evaluate becomes one warning.
pub async fn create_advisory(
    client: &crate::kube::KubeClient,
    labels: Option<&std::collections::HashMap<String, String>>,
    vcpus: u32,
    memory_mib: u64,
) -> Vec<String> {
    if !crate::features::allow_non_ga(pools::FEATURE_ID) {
        return Vec::new();
    }
    let Some(labels) = labels else {
        return Vec::new();
    };
    let sorted: std::collections::BTreeMap<String, String> =
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let wanted = pools::pools_of(&sorted);
    if wanted.is_empty() {
        return Vec::new();
    }
    let defs = PoolStore::load_from(&path()).pools;
    if !wanted.iter().any(|p| defs.iter().any(|d| &d.name == p)) {
        return Vec::new();
    }
    match client.list_all_vms().await {
        Ok(vms) => {
            let vms: Vec<PoolVm> = vms.iter().map(|v| PoolVm::from_vm(v, "default")).collect();
            pools::create_warnings(&wanted, &defs, &vms, u64::from(vcpus), memory_mib)
        }
        Err(_) => vec!["resource pool limits could not be evaluated (VM list failed)".into()],
    }
}

pub async fn list_pools_handler(State(state): State<SharedState>) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let vms = match load_vms(&state).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let defs = PoolStore::load_from(&path()).pools;
    // The list leaves out member rows; use the pool detail or report route for those.
    let rows: Vec<_> = pools::report_all(&defs, &vms)
        .into_iter()
        .map(|mut r| {
            r.members.clear();
            r
        })
        .collect();
    ok(json!(rows))
}

#[derive(Debug, Deserialize)]
pub struct CreatePoolBody {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub max_vms: Option<u64>,
    #[serde(default)]
    pub max_vcpus: Option<u64>,
    #[serde(default)]
    pub max_memory_mib: Option<u64>,
}

pub async fn create_pool_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<CreatePoolBody>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let limits = PoolLimits {
        max_vms: body.max_vms,
        max_vcpus: body.max_vcpus,
        max_memory_mib: body.max_memory_mib,
    };
    let def = match PoolDef::new(&body.name, &body.description, limits, chrono::Utc::now()) {
        Ok(d) => d,
        Err(e) => return bad(400, "INVALID_POOL", &e),
    };
    let res = PoolStore::update(&path(), |s| s.add(def.clone()));
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Create,
        "resource-pool",
        &body.name,
        res.is_ok(),
        res.as_ref().err().cloned(),
    )
    .await;
    match res {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({ "success": true, "data": def })),
        )
            .into_response(),
        Err(e) if e.contains("already") => bad(409, "CONFLICT", &e),
        Err(e) if e.starts_with("at most") => bad(400, "LIMIT_REACHED", &e),
        Err(e) => bad(500, "PERSIST_FAILED", &e),
    }
}

async fn report_response(state: &SharedState, name: &str) -> axum::response::Response {
    if let Err(e) = pools::validate_name(name) {
        return bad(400, "INVALID_POOL", &e);
    }
    let vms = match load_vms(state).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let defs = PoolStore::load_from(&path()).pools;
    let def = defs.iter().find(|d| d.name == name);
    let report = pools::report(name, def, &vms);
    if def.is_none() && report.members.is_empty() {
        return bad(404, "NOT_FOUND", "No such pool");
    }
    ok(json!(report))
}

pub async fn get_pool_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    report_response(&state, &name).await
}

/// Read-only limit evaluation of one pool (same body as the detail route).
pub async fn pool_report_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    report_response(&state, &name).await
}

pub async fn pool_hierarchy_handler(State(state): State<SharedState>) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let vms = match load_vms(&state).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let defs = PoolStore::load_from(&path()).pools;
    ok(json!(pools::hierarchy(&defs, &vms)))
}

#[derive(Debug, Deserialize)]
pub struct AssignBody {
    pub vm: String,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Remove the VM from the pool instead of adding it.
    #[serde(default)]
    pub remove: bool,
}

fn valid_k8s_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !s.starts_with(['-', '.'])
        && !s.ends_with(['-', '.'])
}

pub async fn assign_pool_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<AssignBody>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    if !valid_k8s_name(&body.vm) {
        return bad(400, "INVALID", "vm is not a valid Kubernetes name");
    }
    let namespace = match body.namespace.clone() {
        Some(n) if valid_k8s_name(&n) => n,
        Some(_) => return bad(400, "INVALID", "namespace is not a valid Kubernetes name"),
        None => state.read().await.namespace.clone(),
    };
    let patch = match pools::assign_patch(&name, !body.remove) {
        Ok(p) => p,
        Err(e) => return bad(400, "INVALID_POOL", &e),
    };
    // Joining needs a definition; leaving works for label-only pools too.
    if !body.remove
        && PoolStore::load_from(&path())
            .pools
            .iter()
            .all(|d| d.name != name)
    {
        return bad(404, "NOT_FOUND", "No such pool; create it first");
    }
    let client = state.read().await.client();
    let api: kube::Api<crate::kube::VirtualMachine> =
        kube::Api::namespaced(client.client(), &namespace);
    let res: Result<(), anyhow::Error> = async {
        api.get(&body.vm).await?;
        api.patch(&body.vm, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        Ok(())
    }
    .await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Update,
        "resource-pool",
        &format!(
            "{}:{}{}/{}",
            name,
            if body.remove { "-" } else { "+" },
            namespace,
            body.vm
        ),
        res.is_ok(),
        res.as_ref().err().map(|e| e.to_string()),
    )
    .await;
    match res {
        Ok(()) => ok(json!({
            "pool": name, "vm": body.vm, "namespace": namespace,
            "member": !body.remove
        })),
        Err(e) if is_not_found(&e) => bad(404, "NOT_FOUND", "No such VM"),
        Err(e) => bad(500, "ASSIGN_FAILED", &sanitize_error(&e)),
    }
}

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    /// Also remove the pool label from every member VM.
    #[serde(default)]
    pub unassign: bool,
}

pub async fn delete_pool_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let patch = match pools::assign_patch(&name, false) {
        Ok(p) => p,
        Err(e) => return bad(400, "INVALID_POOL", &e),
    };
    if PoolStore::load_from(&path())
        .pools
        .iter()
        .all(|d| d.name != name)
    {
        return bad(404, "NOT_FOUND", "No such pool");
    }
    let vms = match load_vms(&state).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let members: Vec<&PoolVm> = vms
        .iter()
        .filter(|v| v.pools.iter().any(|p| p == &name))
        .collect();
    if !members.is_empty() && !q.unassign {
        return bad(
            409,
            "POOL_NOT_EMPTY",
            &format!(
                "pool has {} member VM(s); pass unassign=true to remove the label from them",
                members.len()
            ),
        );
    }
    let client = state.read().await.client();
    let mut failed = 0usize;
    for m in &members {
        let api: kube::Api<crate::kube::VirtualMachine> =
            kube::Api::namespaced(client.client(), &m.namespace);
        if api
            .patch(&m.name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .is_err()
        {
            failed += 1;
        }
    }
    // Keep the definition if any label could not be removed, so a retry finds the members.
    let res = if failed > 0 {
        Err(format!("could not remove the label from {failed} VM(s)"))
    } else {
        PoolStore::update(&path(), |s| s.remove(&name))
    };
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Delete,
        "resource-pool",
        &name,
        res.is_ok(),
        res.as_ref().err().cloned(),
    )
    .await;
    match res {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if failed > 0 => bad(502, "UNASSIGN_FAILED", &e),
        Err(e) => bad(500, "PERSIST_FAILED", &e),
    }
}
