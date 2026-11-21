// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! DR replication pairs (Experimental): `/v1/enterprise/dr/*`. The logic lives in
//! `crate::dr::pairs`; this file gathers cluster facts, audits and serves the API.
//! Requires `ZORVIA_EXPERIMENTAL=1` and `ZORVIA_FEATURE_DR_REPLICATION=1`.

use super::*;
use crate::dr::pairs::{self, DrPair, Facts, PairStore, SourceState};
use axum::extract::Json as AxumJson;
use serde_json::json;

fn gate() -> Option<axum::response::Response> {
    if pairs::enabled() {
        None
    } else {
        let (st, j) = err_json(
            403,
            "feature_disabled",
            "DR replication is disabled. Set ZORVIA_EXPERIMENTAL=1 and ZORVIA_FEATURE_DR_REPLICATION=1",
        );
        Some((st, j).into_response())
    }
}

fn path() -> std::path::PathBuf {
    PairStore::default_path()
}

fn ok(v: serde_json::Value) -> axum::response::Response {
    Json(json!({ "success": true, "data": v })).into_response()
}

fn bad(status: u16, code: &str, msg: &str) -> axum::response::Response {
    let (st, j) = err_json(status, code, msg);
    (st, j).into_response()
}

#[allow(clippy::result_large_err)]
fn load_pair(name: &str) -> Result<DrPair, axum::response::Response> {
    PairStore::load_from(&path())
        .get(name)
        .cloned()
        .ok_or_else(|| bad(404, "NOT_FOUND", "No such DR pair"))
}

/// Ask the cluster about the source VM and the DR target.
async fn gather_facts(state: &SharedState, pair: &DrPair) -> Facts {
    let client = state.read().await.client();
    let source = match client.get_vm(&pair.namespace, &pair.vm).await {
        Ok(vm) => {
            let printable = vm.status.as_ref().and_then(|s| s.printable_status.clone());
            // Anything but an explicit Stopped counts as running (conservative).
            if printable.as_deref() == Some("Stopped") {
                SourceState::Stopped
            } else {
                SourceState::Running
            }
        }
        Err(e) if is_not_found(&e) => SourceState::Absent,
        Err(e) => SourceState::Unknown(sanitize_error(&e)),
    };
    let dr_vm_exists = match client.get_vm(&pair.dr_namespace, &pair.dr_vm).await {
        Ok(_) => Some(true),
        Err(e) if is_not_found(&e) => Some(false),
        Err(_) => None,
    };
    Facts {
        source,
        dr_vm_exists,
    }
}

fn is_not_found(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<kube::Error>(), Some(kube::Error::Api(ae)) if ae.code == 404)
}

fn backend() -> pairs::OpsBackend {
    pairs::OpsBackend { target: Ok(()) }
}

pub async fn list_dr_pairs_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let now = chrono::Utc::now();
    let s = PairStore::load_from(&path());
    ok(json!(s
        .pairs
        .iter()
        .map(|p| pairs::status(p, now))
        .collect::<Vec<_>>()))
}

#[derive(Debug, Deserialize)]
pub struct CreateDrPairBody {
    pub name: String,
    pub vm: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default = "default_dr_ns")]
    pub dr_namespace: String,
    pub rpo_seconds: u64,
    #[serde(default = "default_rto")]
    pub rto_seconds: u64,
}

fn default_dr_ns() -> String {
    "zorvia-dr".into()
}
fn default_rto() -> u64 {
    3600
}

pub async fn create_dr_pair_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<CreateDrPairBody>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let namespace = match body.namespace {
        Some(n) => n,
        None => state.read().await.namespace.clone(),
    };
    let now = chrono::Utc::now();
    let pair = match DrPair::new(
        &body.name,
        &body.vm,
        &namespace,
        &body.dr_namespace,
        body.rpo_seconds,
        body.rto_seconds,
        now,
    ) {
        Ok(p) => p,
        Err(e) => return bad(400, "INVALID_PAIR", &e),
    };
    let res = PairStore::update(&path(), |s| s.add(pair.clone()));
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Create,
        "dr-pair",
        &body.name,
        res.is_ok(),
        res.as_ref().err().cloned(),
    )
    .await;
    match res {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({ "success": true, "data": pairs::status(&pair, now) })),
        )
            .into_response(),
        Err(e) if e.contains("already") => bad(409, "CONFLICT", &e),
        Err(e) => bad(500, "PERSIST_FAILED", &e),
    }
}

pub async fn get_dr_pair_handler(Path(name): Path<String>) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    match load_pair(&name) {
        Ok(p) => ok(json!({ "pair": p, "status": pairs::status(&p, chrono::Utc::now()) })),
        Err(r) => r,
    }
}

/// RPO/RTO status for one pair; also refreshes the measured RTO of a pending failover.
pub async fn dr_pair_status_handler(Path(name): Path<String>) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let now = chrono::Utc::now();
    let res = PairStore::update(&path(), |s| {
        let p = s.get_mut(&name).ok_or("not found")?;
        pairs::refresh_failover(p, &backend(), now);
        Ok(pairs::status(p, now))
    });
    match res {
        Ok(st) => ok(json!(st)),
        Err(e) if e == "not found" => bad(404, "NOT_FOUND", "No such DR pair"),
        Err(e) => bad(500, "PERSIST_FAILED", &e),
    }
}

pub async fn dr_alerts_handler() -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    ok(json!(pairs::recent_alerts()))
}

pub async fn delete_dr_pair_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let res = PairStore::update(&path(), |s| {
        if s.remove(&name) {
            Ok(())
        } else {
            Err("not found".to_string())
        }
    });
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::Delete,
        "dr-pair",
        &name,
        res.is_ok(),
        res.as_ref().err().cloned(),
    )
    .await;
    match res {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e == "not found" => bad(404, "NOT_FOUND", "No such DR pair"),
        Err(e) => bad(500, "PERSIST_FAILED", &e),
    }
}

pub async fn dr_failover_plan_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let pair = match load_pair(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let facts = gather_facts(&state, &pair).await;
    ok(json!(pairs::plan_failover(
        &pair,
        &facts,
        chrono::Utc::now()
    )))
}

pub async fn dr_failover_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    AxumJson(req): AxumJson<pairs::FailoverRequest>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let pair = match load_pair(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let facts = gather_facts(&state, &pair).await;
    let now = chrono::Utc::now();
    let be = backend();
    let res = PairStore::update(&path(), |s| {
        let p = s.get_mut(&name).ok_or("not found".to_string())?;
        match pairs::execute_failover(p, &facts, &req, &be, now) {
            Ok(o) => Ok(Ok(o)),
            // Refusals are results, not persistence errors: nothing to save.
            Err(e) => Ok(Err(e)),
        }
    });
    let outcome = match res {
        Ok(o) => o,
        Err(e) if e == "not found" => return bad(404, "NOT_FOUND", "No such DR pair"),
        Err(e) => return bad(500, "PERSIST_FAILED", &e),
    };
    if !req.dry_run {
        record_audit(
            &state,
            &headers,
            crate::audit_trail::AuditAction::Restore,
            "dr-pair",
            &name,
            outcome.is_ok(),
            outcome.as_ref().err().map(|e| e.to_string()),
        )
        .await;
    }
    match outcome {
        Ok(pairs::FailoverOutcome::DryRun(plan)) => ok(json!({ "dry_run": true, "plan": plan })),
        Ok(pairs::FailoverOutcome::Started { plan, restore_op }) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "success": true,
                "data": { "dry_run": false, "restore_operation_id": restore_op, "plan": plan }
            })),
        )
            .into_response(),
        Err(pairs::FailoverError::Refused(b)) => bad(409, "FAILOVER_REFUSED", &b.join("; ")),
        Err(e @ pairs::FailoverError::ConfirmationRequired(_)) => {
            bad(400, "CONFIRMATION_REQUIRED", &e.to_string())
        }
        Err(pairs::FailoverError::Backend(e)) => bad(500, "RESTORE_FAILED", &e),
    }
}

pub async fn dr_failback_plan_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let pair = match load_pair(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let facts = gather_facts(&state, &pair).await;
    ok(json!(pairs::failback_plan(&pair, &facts)))
}

#[derive(Debug, Deserialize)]
pub struct ResumeBody {
    #[serde(default)]
    pub confirm: Option<String>,
}

pub async fn dr_resume_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<ResumeBody>,
) -> impl IntoResponse {
    if let Some(r) = gate() {
        return r;
    }
    let pair = match load_pair(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let facts = gather_facts(&state, &pair).await;
    let now = chrono::Utc::now();
    let res = PairStore::update(&path(), |s| {
        let p = s.get_mut(&name).ok_or("not found".to_string())?;
        Ok(pairs::resume(p, &facts, body.confirm.as_deref(), now))
    });
    let outcome = match res {
        Ok(o) => o,
        Err(e) if e == "not found" => return bad(404, "NOT_FOUND", "No such DR pair"),
        Err(e) => return bad(500, "PERSIST_FAILED", &e),
    };
    record_audit(
        &state,
        &headers,
        crate::audit_trail::AuditAction::ConfigChange,
        "dr-pair",
        &name,
        outcome.is_ok(),
        outcome.as_ref().err().map(|e| e.to_string()),
    )
    .await;
    match outcome {
        Ok(()) => ok(json!({ "resumed": true })),
        Err(pairs::FailoverError::Refused(b)) => bad(409, "RESUME_REFUSED", &b.join("; ")),
        Err(e) => bad(400, "CONFIRMATION_REQUIRED", &e.to_string()),
    }
}

/// Scheduler loop: every 60s (leader only, feature enabled) advance each pair.
pub fn spawn_dr_replication_loop(_state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if !pairs::enabled() || crate::api::leader::skip_if_follower("dr-replication") {
                continue;
            }
            run_pairs_once().await;
        }
    });
}

async fn run_pairs_once() {
    if PairStore::load_from(&path()).pairs.is_empty() {
        return;
    }
    let target = match crate::backup::offcluster::OffClusterTarget::resolve().await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err("no off-cluster target is configured (set ZORVIA_BACKUP_S3_* or ZORVIA_BACKUP_ATLAS_BUCKET)".to_string()),
        Err(e) => Err(format!("off-cluster target invalid: {e}")),
    };
    let be = pairs::OpsBackend { target };
    let now = chrono::Utc::now();
    let res = PairStore::update(&path(), |s| {
        for p in s.pairs.iter_mut() {
            pairs::tick(p, now, &be, &pairs::NotificationSink);
            pairs::refresh_failover(p, &be, now);
        }
        Ok(())
    });
    if let Err(e) = res {
        log::error!("DR replication loop: {e}");
    }
}
