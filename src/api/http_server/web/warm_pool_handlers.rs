// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Warm pools of pre-provisioned VMs, backed by
//! `crate::warm_pool::WarmPoolManager` (real, disk-persisted) and the real
//! template catalog (`super::template_handlers::create_vm_from_template`).
//! `spawn_warm_pool_loop` actually provisions standby VMs (stopped) every
//! 60s to keep each pool at its configured size; `claim` actually starts
//! one and hands its name back.

use super::*;
use crate::warm_pool::{WarmPool, WarmPoolManager, WarmPoolMember, WarmPoolMemberStatus};
use axum::extract::Json as AxumJson;
use serde_json::json;

fn member_json(m: &WarmPoolMember) -> serde_json::Value {
    json!({
        "vm_name": m.vm_name,
        "status": format!("{:?}", m.status).to_lowercase(),
        "created_at": m.created_at.to_rfc3339(),
        "claimed_at": m.claimed_at.map(|t| t.to_rfc3339()),
        "error": m.error,
    })
}

fn pool_json(p: &WarmPool) -> serde_json::Value {
    json!({
        "name": p.name,
        "template": p.template,
        "size": p.size,
        "ready_count": p.members.iter().filter(|m| m.status == WarmPoolMemberStatus::Ready).count(),
        "members": p.members.iter().map(member_json).collect::<Vec<_>>(),
    })
}

pub async fn list_warm_pools_handler() -> impl IntoResponse {
    let mgr = WarmPoolManager::load();
    Json(mgr.all().iter().map(pool_json).collect::<Vec<_>>()).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateWarmPoolBody {
    pub name: String,
    pub template: String,
    pub size: u32,
}

pub async fn create_warm_pool_handler(
    AxumJson(body): AxumJson<CreateWarmPoolBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "Pool name is required");
        return (st, j).into_response();
    }
    if crate::templates::TEMPLATES.get(&body.template).is_none() {
        let (st, j) = err_json(
            400,
            "INVALID_TEMPLATE",
            &format!("No template named '{}'", body.template),
        );
        return (st, j).into_response();
    }

    let mut mgr = WarmPoolManager::load();
    if mgr.get_pool(&body.name).is_some() {
        let (st, j) = err_json(409, "CONFLICT", "A pool with this name already exists");
        return (st, j).into_response();
    }
    let pool = WarmPool::new(&body.name, &body.template, body.size);
    let response = pool_json(&pool);
    mgr.add_pool(pool);
    if let Err(e) = mgr.save() {
        let (st, j) = err_json(500, "PERSIST_FAILED", &e.to_string());
        return (st, j).into_response();
    }
    (StatusCode::CREATED, Json(response)).into_response()
}

pub async fn delete_warm_pool_handler(Path(name): Path<String>) -> impl IntoResponse {
    let mut mgr = WarmPoolManager::load();
    if mgr.remove_pool(&name) {
        let _ = mgr.save();
        StatusCode::NO_CONTENT.into_response()
    } else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such warm pool");
        (st, j).into_response()
    }
}

/// Starts a Ready standby member for real and hands its name back --
/// nothing is faked here, this is a real VM the caller can use immediately.
pub async fn claim_warm_pool_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mut mgr = WarmPoolManager::load();
    let Some(pool) = mgr.get_pool_mut(&name) else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such warm pool");
        return (st, j).into_response();
    };
    let Some(member) = pool.find_ready().cloned() else {
        let (st, j) = err_json(
            409,
            "NO_STANDBY_AVAILABLE",
            "No ready standby VM in this pool yet -- it may still be provisioning",
        );
        return (st, j).into_response();
    };

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let start_result = client.start_vm(&namespace, &member.vm_name).await;
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Start,
        "vm",
        &member.vm_name,
        start_result.is_ok(),
        start_result.as_ref().err().map(|e| sanitize_error(e)),
    )
    .await;

    if let Err(e) = start_result {
        let (st, j) = err_json(500, "CLAIM_FAILED", &sanitize_error(&e));
        return (st, j).into_response();
    }

    if let Some(m) = pool
        .members
        .iter_mut()
        .find(|m| m.vm_name == member.vm_name)
    {
        m.status = WarmPoolMemberStatus::Claimed;
        m.claimed_at = Some(chrono::Utc::now());
    }
    let _ = mgr.save();

    Json(json!({ "vm_name": member.vm_name, "pool": name })).into_response()
}

/// Spawns the loop that keeps every pool provisioned up to its configured
/// size (checks every 60s, same cadence as the other schedulers/loops).
pub fn spawn_warm_pool_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if crate::api::leader::skip_if_follower("warm-pool") {
                continue;
            }
            let (namespace, client) = {
                let s = state.read().await;
                (s.namespace.clone(), s.client())
            };
            reconcile_pools(&namespace, &client).await;
        }
    });
}

async fn reconcile_pools(namespace: &str, client: &crate::kube::KubeClient) {
    let mut mgr = WarmPoolManager::load();
    let pool_names: Vec<String> = mgr.all().iter().map(|p| p.name.clone()).collect();
    if pool_names.is_empty() {
        return;
    }

    let mut changed = false;
    for pool_name in pool_names {
        // Recompute deficit and next name fresh each iteration since
        // provisioning one member changes both.
        while let Some(pool) = mgr.get_pool(&pool_name) {
            if pool.ready_or_provisioning_count() >= pool.size as usize {
                break;
            }
            let template = pool.template.clone();
            let member_name = pool.next_member_name();

            let result = super::template_handlers::create_vm_from_template(
                client,
                namespace,
                &template,
                &member_name,
            )
            .await;
            let status = if result.is_ok() {
                WarmPoolMemberStatus::Ready
            } else {
                WarmPoolMemberStatus::Failed
            };
            if let Err(e) = &result {
                log::error!(
                    "Warm pool '{pool_name}': failed to provision standby '{member_name}': {e}"
                );
            } else {
                log::info!("Warm pool '{pool_name}': provisioned standby '{member_name}'");
            }

            if let Some(pool) = mgr.get_pool_mut(&pool_name) {
                pool.members.push(WarmPoolMember {
                    vm_name: member_name,
                    status,
                    created_at: chrono::Utc::now(),
                    claimed_at: None,
                    error: result.err().map(|e| e.to_string()),
                });
            }
            changed = true;
        }
    }

    if changed {
        if let Err(e) = mgr.save() {
            log::warn!("Failed to persist warm pool state: {e}");
        }
    }
}
