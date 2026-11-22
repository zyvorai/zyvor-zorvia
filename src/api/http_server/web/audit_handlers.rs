// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real audit-log query/stats routes backed by `crate::audit_trail::AuditTrail`
//! (in-memory, reset on restart), fed by `record_audit()` calls in the VM
//! lifecycle handlers. Fixes a live bug: `web/src/api/audit.ts` and
//! `VMDetails.tsx`'s activity panel already called `/audit/logs` before this
//! route existed anywhere -- it silently 404'd.

use super::*;
use serde_json::json;
use std::collections::BTreeMap;

fn action_str(a: &crate::audit_trail::AuditAction) -> &'static str {
    use crate::audit_trail::AuditAction::*;
    match a {
        Create => "create",
        Update => "update",
        Delete => "delete",
        Start => "start",
        Stop => "stop",
        Restart => "restart",
        Migrate => "migrate",
        Clone => "clone",
        Snapshot => "snapshot",
        Restore => "restore",
        ConfigChange => "config_change",
        Login => "login",
        Logout => "logout",
        Export => "export",
        Import => "import",
        ScaleUp => "scale_up",
        ScaleDown => "scale_down",
        Approve => "approve",
        Reject => "reject",
        ScheduleChange => "schedule_change",
        Exec => "exec",
        ViewLogs => "view_logs",
    }
}

fn detail_string(e: &crate::audit_trail::AuditEntry) -> Option<String> {
    match &e.details {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Matches the frontend's AuditLog shape (web/src/api/audit.ts) exactly --
/// that contract predates this handler and VMDetails.tsx already renders
/// against it, so the response is shaped to fit the existing caller rather
/// than the other way around.
fn entry_json(e: &crate::audit_trail::AuditEntry) -> serde_json::Value {
    let details = detail_string(e);
    json!({
        "id": e.id,
        "timestamp": e.timestamp.to_rfc3339(),
        "user": e.user,
        "action": action_str(&e.action),
        "resource_type": e.resource_type,
        "resource_name": e.resource_name,
        "status": if e.success { "success" } else { "failed" },
        "ip_address": if e.ip_address.is_empty() { None } else { Some(e.ip_address.clone()) },
        "details": details,
        "error": if e.success { None } else { detail_string(e) },
    })
}

#[derive(Debug, Deserialize, Default)]
pub struct AuditLogQuery {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn list_audit_logs_handler(
    State(state): State<SharedState>,
    Query(q): Query<AuditLogQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let audit = s.audit.clone();
    drop(s);
    let trail = crate::audit_trail::read_fresh(&audit).await;

    let mut matched: Vec<&crate::audit_trail::AuditEntry> = trail
        .entries
        .iter()
        .filter(|e| {
            q.user.as_deref().is_none_or(|v| e.user == v)
                && q.resource_type
                    .as_deref()
                    .is_none_or(|v| e.resource_type == v)
                && q.resource_name
                    .as_deref()
                    .is_none_or(|v| e.resource_name == v)
                && q.action
                    .as_deref()
                    .is_none_or(|v| action_str(&e.action) == v)
                && q.status
                    .as_deref()
                    .is_none_or(|v| (v == "success") == e.success)
        })
        .collect();
    // Newest first -- entries are recorded in chronological order.
    matched.reverse();

    Json(matched.iter().map(|e| entry_json(e)).collect::<Vec<_>>()).into_response()
}

pub async fn audit_stats_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let audit = s.audit.clone();
    drop(s);
    let trail = crate::audit_trail::read_fresh(&audit).await;

    let mut by_action: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut by_user: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_status: BTreeMap<&'static str, u64> = BTreeMap::new();
    for e in &trail.entries {
        *by_action.entry(action_str(&e.action)).or_insert(0) += 1;
        *by_user.entry(e.user.clone()).or_insert(0) += 1;
        *by_status
            .entry(if e.success { "success" } else { "failed" })
            .or_insert(0) += 1;
    }
    let recent_failures = trail
        .entries
        .iter()
        .rev()
        .take(50)
        .filter(|e| !e.success)
        .count();

    Json(json!({
        "total_logs": trail.entries.len(),
        "by_action": by_action,
        "by_user": by_user,
        "by_status": by_status,
        "recent_failures": recent_failures,
    }))
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
pub struct AuditExportQuery {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default = "default_export_limit")]
    pub limit: usize,
}
fn default_export_limit() -> usize {
    5000
}

/// Admin-only JSONL export of the audit trail (for SIEM / Loki import).
pub async fn export_audit_logs_handler(
    State(state): State<SharedState>,
    Query(q): Query<AuditExportQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let audit = s.audit.clone();
    drop(s);
    let trail = crate::audit_trail::read_fresh(&audit).await;
    let limit = q.limit.clamp(1, 50_000);
    let body = trail.export_jsonl(q.user.as_deref(), q.resource_type.as_deref(), limit);
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/x-ndjson; charset=utf-8",
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"zorvia-audit.jsonl\"",
            ),
        ],
        body,
    )
        .into_response()
}
