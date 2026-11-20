// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Managed-operations records: explicit enrollment, maintenance windows,
//! approved tasks, backup / restore-test evidence, incidents, service
//! reports and remediation policies.
//!
//! These are **records**. Nothing here connects to a cluster or acts on it.
//! Buying a contract grants no access: remote operation is off by default,
//! must be enabled by a member of the customer's organization with a scoped
//! credential *reference*, is revocable at any time, and is only ever
//! `effective` while the server gate is on and the Managed contract is
//! active.

use super::model::ContractStatus;
use super::services::{service_event, subject_events};
use super::store::{
    bounded, is_member, load_contract, new_id, org_exists, parse_ts, ts, Caller, CommercialError,
    CommercialStore, Result,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const REMOTE_SCOPES: &[&str] = &[
    "monitoring_read",
    "maintenance_tasks",
    "remediation_actions",
];

/// `ZORVIA_MANAGED_REMOTE_ENABLED=1` turns on the ability to enable remote
/// operation at all. Off by default.
pub fn remote_management_enabled() -> bool {
    matches!(
        std::env::var("ZORVIA_MANAGED_REMOTE_ENABLED")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteOperation {
    /// What the customer last asked for.
    pub requested: bool,
    /// Whether it can actually be used right now: requested, server gate on,
    /// enrollment active and the Managed contract active.
    pub effective: bool,
    pub server_gate_enabled: bool,
    pub scope: Vec<String>,
    /// A reference to a customer-issued credential. The credential itself is
    /// never stored here.
    pub credential_ref: Option<String>,
    pub enabled_by: Option<String>,
    pub enabled_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Enrollment {
    pub id: String,
    pub org_id: String,
    pub contract_id: String,
    pub cluster_id: String,
    /// `enrolled` or `withdrawn`.
    pub status: String,
    pub covered_components: Vec<String>,
    pub maintenance_windows: Vec<String>,
    pub monitoring_signals: Vec<String>,
    pub operational_owner: Option<String>,
    pub remote_operation: RemoteOperation,
    /// The Managed contract is active. False after expiry: the managed
    /// service lapses, VMs and data are untouched.
    pub service_eligible: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub window: String,
    /// `proposed`, `approved`, `scheduled`, `done`, `cancelled`.
    pub status: String,
    pub proposed_by: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryEvidence {
    pub id: String,
    pub kind: String,
    pub result: String,
    pub performed_at: DateTime<Utc>,
    pub reference: Option<String>,
    pub note: Option<String>,
    pub added_by: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Incident {
    pub id: String,
    pub title: String,
    pub severity: String,
    /// `open`, `mitigated`, `resolved`.
    pub status: String,
    pub summary: String,
    pub opened_by: String,
    pub opened_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceReport {
    pub id: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub summary: String,
    pub reference: Option<String>,
    pub added_by: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemediationPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub permissions: Vec<String>,
    /// `draft`, `authorized`, `revoked`. Only `authorized` policies permit
    /// remediation, and no executor exists yet.
    pub status: String,
    pub created_by: String,
    pub authorized_by: Option<String>,
    pub authorized_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrollmentDetail {
    pub enrollment: Enrollment,
    pub tasks: Vec<Task>,
    pub recovery_evidence: Vec<RecoveryEvidence>,
    pub incidents: Vec<Incident>,
    pub reports: Vec<ServiceReport>,
    pub policies: Vec<RemediationPolicy>,
    pub events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEnrollment {
    pub org_id: String,
    pub cluster_id: String,
    #[serde(default)]
    pub covered_components: Vec<String>,
    #[serde(default)]
    pub maintenance_windows: Vec<String>,
    #[serde(default)]
    pub monitoring_signals: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteRequest {
    pub enable: bool,
    #[serde(default)]
    pub scope: Vec<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTask {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub window: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewRecoveryEvidence {
    /// `backup` or `restore_test`.
    pub kind: String,
    /// `pass` or `fail`.
    pub result: String,
    /// RFC 3339.
    pub performed_at: String,
    pub reference: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewIncident {
    pub title: String,
    pub severity: String,
    pub summary: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncidentUpdate {
    /// `open`, `mitigated` or `resolved`.
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewReport {
    pub period_start: String,
    pub period_end: String,
    pub summary: String,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewPolicy {
    pub name: String,
    pub description: String,
    pub permissions: Vec<String>,
}

fn clean_list(label: &str, items: &[String]) -> Result<Vec<String>> {
    if items.len() > 50 {
        return Err(CommercialError::Invalid(format!(
            "{label}: at most 50 entries"
        )));
    }
    items.iter().map(|i| bounded(label, i, 200, true)).collect()
}

fn json_list(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn contract_eligible(
    conn: &Connection,
    contract_id: &str,
    cluster: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    Ok(load_contract(conn, contract_id, now)?.is_some_and(|c| {
        c.offering == "managed"
            && c.effective_status == ContractStatus::Active
            && c.entitlement.covered_clusters.iter().any(|x| x == cluster)
    }))
}

fn load_enrollment(
    conn: &Connection,
    id: &str,
    now: DateTime<Utc>,
    gate: bool,
) -> Result<Option<Enrollment>> {
    struct Raw {
        org_id: String,
        contract_id: String,
        cluster_id: String,
        status: String,
        components: String,
        windows: String,
        signals: String,
        owner: Option<String>,
        remote_enabled: i64,
        scope: String,
        cred: Option<String>,
        en_by: Option<String>,
        en_at: Option<String>,
        rev_by: Option<String>,
        rev_at: Option<String>,
        created_by: String,
        created: String,
        updated: String,
    }
    let raw = conn
        .query_row(
            "SELECT org_id, contract_id, cluster_id, status, covered_components, maintenance_windows,
                    monitoring_signals, operational_owner, remote_enabled, remote_scope,
                    remote_credential_ref, remote_enabled_by, remote_enabled_at, remote_revoked_by,
                    remote_revoked_at, created_by, created_at, updated_at
             FROM managed_enrollments WHERE id = ?1",
            [id],
            |r| {
                Ok(Raw {
                    org_id: r.get(0)?,
                    contract_id: r.get(1)?,
                    cluster_id: r.get(2)?,
                    status: r.get(3)?,
                    components: r.get(4)?,
                    windows: r.get(5)?,
                    signals: r.get(6)?,
                    owner: r.get(7)?,
                    remote_enabled: r.get(8)?,
                    scope: r.get(9)?,
                    cred: r.get(10)?,
                    en_by: r.get(11)?,
                    en_at: r.get(12)?,
                    rev_by: r.get(13)?,
                    rev_at: r.get(14)?,
                    created_by: r.get(15)?,
                    created: r.get(16)?,
                    updated: r.get(17)?,
                })
            },
        )
        .optional()?;
    let Some(raw) = raw else { return Ok(None) };
    let eligible = contract_eligible(conn, &raw.contract_id, &raw.cluster_id, now)?;
    let requested = raw.remote_enabled != 0;
    Ok(Some(Enrollment {
        id: id.to_string(),
        org_id: raw.org_id,
        contract_id: raw.contract_id,
        cluster_id: raw.cluster_id,
        remote_operation: RemoteOperation {
            requested,
            effective: requested && gate && eligible && raw.status == "enrolled",
            server_gate_enabled: gate,
            scope: json_list(&raw.scope),
            credential_ref: raw.cred,
            enabled_by: raw.en_by,
            enabled_at: raw.en_at.as_deref().map(parse_ts).transpose()?,
            revoked_by: raw.rev_by,
            revoked_at: raw.rev_at.as_deref().map(parse_ts).transpose()?,
        },
        status: raw.status,
        covered_components: json_list(&raw.components),
        maintenance_windows: json_list(&raw.windows),
        monitoring_signals: json_list(&raw.signals),
        operational_owner: raw.owner,
        service_eligible: eligible,
        created_by: raw.created_by,
        created_at: parse_ts(&raw.created)?,
        updated_at: parse_ts(&raw.updated)?,
    }))
}

fn opt(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.as_deref().map(parse_ts).transpose().map_err(Into::into)
}

impl CommercialStore {
    /// Enrollment, remote access and authorizations are the customer's
    /// decisions: the caller must be a member of the organization. The
    /// service desk is deliberately not enough.
    fn customer_only(&self, conn: &Connection, caller: &Caller, org_id: &str) -> Result<()> {
        let _ = self;
        if is_member(conn, org_id, &caller.username)? {
            Ok(())
        } else if caller.admin {
            Err(CommercialError::Forbidden(
                "this is a customer decision: a member of the organization must do it".into(),
            ))
        } else {
            Err(CommercialError::NotFound)
        }
    }

    fn visible_enrollment(
        &self,
        conn: &Connection,
        caller: &Caller,
        id: &str,
        gate: bool,
    ) -> Result<Enrollment> {
        let e = load_enrollment(conn, id, Utc::now(), gate)?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(conn, caller, &e.org_id)? {
            return Err(CommercialError::NotFound);
        }
        Ok(e)
    }

    fn desk_only(caller: &Caller) -> Result<()> {
        if caller.admin {
            Ok(())
        } else {
            Err(CommercialError::Forbidden(
                "this action is limited to the service desk".into(),
            ))
        }
    }

    pub fn enroll_cluster(&self, caller: &Caller, input: NewEnrollment) -> Result<Enrollment> {
        self.enroll_cluster_with_gate(caller, input, remote_management_enabled())
    }

    pub(super) fn enroll_cluster_with_gate(
        &self,
        caller: &Caller,
        input: NewEnrollment,
        gate: bool,
    ) -> Result<Enrollment> {
        let cluster = bounded("cluster_id", &input.cluster_id, 128, true)?;
        let components = clean_list("covered_components", &input.covered_components)?;
        let windows = clean_list("maintenance_windows", &input.maintenance_windows)?;
        let signals = clean_list("monitoring_signals", &input.monitoring_signals)?;
        let conn = self.lock()?;
        org_exists(&conn, &input.org_id)?;
        self.customer_only(&conn, caller, &input.org_id)?;
        let now = Utc::now();

        let mut stmt =
            conn.prepare("SELECT id FROM contracts WHERE org_id = ?1 AND status = 'active'")?;
        let ids: Vec<String> = stmt
            .query_map([&input.org_id], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let mut contract_id = None;
        for id in ids {
            if contract_eligible(&conn, &id, &cluster, now)? {
                contract_id = Some(id);
                break;
            }
        }
        let contract_id = contract_id.ok_or_else(|| {
            CommercialError::Conflict("no active Managed contract covers this cluster".into())
        })?;
        let dup: Option<String> = conn
            .query_row(
                "SELECT id FROM managed_enrollments WHERE org_id = ?1 AND cluster_id = ?2 AND status = 'enrolled'",
                params![input.org_id, cluster],
                |r| r.get(0),
            )
            .optional()?;
        if dup.is_some() {
            return Err(CommercialError::Conflict(
                "this cluster is already enrolled".into(),
            ));
        }
        let id = new_id("enr");
        conn.execute(
            "INSERT INTO managed_enrollments (id, org_id, contract_id, cluster_id, status,
                covered_components, maintenance_windows, monitoring_signals, created_by,
                created_at, updated_at)
             VALUES (?1,?2,?3,?4,'enrolled',?5,?6,?7,?8,?9,?9)",
            params![
                id,
                input.org_id,
                contract_id,
                cluster,
                serde_json::to_string(&components).map_err(anyhow::Error::from)?,
                serde_json::to_string(&windows).map_err(anyhow::Error::from)?,
                serde_json::to_string(&signals).map_err(anyhow::Error::from)?,
                caller.username,
                ts(now)
            ],
        )?;
        service_event(
            &conn,
            "enrollment",
            &id,
            &caller.username,
            "enrolled",
            json!({ "cluster": cluster, "contract_id": contract_id }),
        )?;
        load_enrollment(&conn, &id, now, gate)?.ok_or(CommercialError::NotFound)
    }

    pub fn list_enrollments(&self, caller: &Caller) -> Result<Vec<Enrollment>> {
        let gate = remote_management_enabled();
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT id, org_id FROM managed_enrollments ORDER BY created_at DESC")?;
        let ids: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let now = Utc::now();
        let mut out = Vec::new();
        for (id, org) in ids {
            if self.allowed(&conn, caller, &org)? {
                if let Some(e) = load_enrollment(&conn, &id, now, gate)? {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    pub fn enrollment_detail(&self, caller: &Caller, id: &str) -> Result<EnrollmentDetail> {
        let conn = self.lock()?;
        let enrollment = self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;

        let mut stmt = conn.prepare(
            "SELECT id, title, description, window_text, status, proposed_by, approved_by,
                    approved_at, completed_at FROM managed_tasks WHERE enrollment_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            let (
                tid,
                title,
                description,
                window,
                status,
                proposed_by,
                approved_by,
                approved_at,
                completed_at,
            ) = row?;
            tasks.push(Task {
                id: tid,
                title,
                description,
                window,
                status,
                proposed_by,
                approved_by,
                approved_at: opt(approved_at)?,
                completed_at: opt(completed_at)?,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT id, kind, result, performed_at, reference, note, added_by
             FROM managed_evidence WHERE enrollment_id = ?1 ORDER BY performed_at DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut recovery_evidence = Vec::new();
        for row in rows {
            let (eid, kind, result, performed, reference, note, added_by) = row?;
            recovery_evidence.push(RecoveryEvidence {
                id: eid,
                kind,
                result,
                performed_at: parse_ts(&performed)?,
                reference,
                note,
                added_by,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT id, title, severity, status, summary, opened_by, opened_at, resolved_at
             FROM managed_incidents WHERE enrollment_id = ?1 ORDER BY opened_at DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut incidents = Vec::new();
        for row in rows {
            let (iid, title, severity, status, summary, opened_by, opened_at, resolved_at) = row?;
            incidents.push(Incident {
                id: iid,
                title,
                severity,
                status,
                summary,
                opened_by,
                opened_at: parse_ts(&opened_at)?,
                resolved_at: opt(resolved_at)?,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT id, period_start, period_end, summary, reference, added_by
             FROM managed_reports WHERE enrollment_id = ?1 ORDER BY period_end DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut reports = Vec::new();
        for row in rows {
            let (rid, start, end, summary, reference, added_by) = row?;
            reports.push(ServiceReport {
                id: rid,
                period_start: parse_ts(&start)?,
                period_end: parse_ts(&end)?,
                summary,
                reference,
                added_by,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT id, name, description, permissions, status, created_by, authorized_by,
                    authorized_at, revoked_by, revoked_at
             FROM remediation_policies WHERE enrollment_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        })?;
        let mut policies = Vec::new();
        for row in rows {
            let (
                pid,
                name,
                description,
                permissions,
                status,
                created_by,
                authorized_by,
                authorized_at,
                revoked_by,
                revoked_at,
            ) = row?;
            policies.push(RemediationPolicy {
                id: pid,
                name,
                description,
                permissions: json_list(&permissions),
                status,
                created_by,
                authorized_by,
                authorized_at: opt(authorized_at)?,
                revoked_by,
                revoked_at: opt(revoked_at)?,
            });
        }
        drop(stmt);

        let events = subject_events(&conn, "enrollment", id)?;
        Ok(EnrollmentDetail {
            enrollment,
            tasks,
            recovery_evidence,
            incidents,
            reports,
            policies,
            events,
        })
    }

    pub fn set_remote_operation(
        &self,
        caller: &Caller,
        id: &str,
        req: RemoteRequest,
    ) -> Result<Enrollment> {
        self.set_remote_operation_with_gate(caller, id, req, remote_management_enabled())
    }

    pub(super) fn set_remote_operation_with_gate(
        &self,
        caller: &Caller,
        id: &str,
        req: RemoteRequest,
        gate: bool,
    ) -> Result<Enrollment> {
        let conn = self.lock()?;
        let e = self.visible_enrollment(&conn, caller, id, gate)?;
        let now = Utc::now();
        if req.enable {
            self.customer_only(&conn, caller, &e.org_id)?;
            if !gate {
                return Err(CommercialError::Forbidden(
                    "remote operation is disabled on this server (ZORVIA_MANAGED_REMOTE_ENABLED)"
                        .into(),
                ));
            }
            if e.status != "enrolled" || !e.service_eligible {
                return Err(CommercialError::Conflict(
                    "remote operation needs an enrolled cluster with an active Managed contract"
                        .into(),
                ));
            }
            if req.scope.is_empty()
                || req
                    .scope
                    .iter()
                    .any(|s| !REMOTE_SCOPES.contains(&s.as_str()))
            {
                return Err(CommercialError::Invalid(format!(
                    "scope must be a non-empty subset of {REMOTE_SCOPES:?}"
                )));
            }
            let cred = bounded(
                "credential_ref",
                req.credential_ref.as_deref().unwrap_or(""),
                200,
                true,
            )?;
            if req.scope.iter().any(|s| s == "remediation_actions") {
                let authorized: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM remediation_policies WHERE enrollment_id = ?1 AND status = 'authorized'",
                    [id],
                    |r| r.get(0),
                )?;
                if authorized == 0 {
                    return Err(CommercialError::Conflict(
                        "remediation_actions needs at least one authorized remediation policy"
                            .into(),
                    ));
                }
            }
            conn.execute(
                "UPDATE managed_enrollments SET remote_enabled = 1, remote_scope = ?2,
                    remote_credential_ref = ?3, remote_enabled_by = ?4, remote_enabled_at = ?5,
                    remote_revoked_by = NULL, remote_revoked_at = NULL, updated_at = ?5 WHERE id = ?1",
                params![
                    id,
                    serde_json::to_string(&req.scope).map_err(anyhow::Error::from)?,
                    cred,
                    caller.username,
                    ts(now)
                ],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "remote_operation_enabled",
                json!({ "scope": req.scope, "credential_ref": cred }),
            )?;
        } else {
            // Revoking is always allowed to the customer and to the desk.
            conn.execute(
                "UPDATE managed_enrollments SET remote_enabled = 0, remote_revoked_by = ?2,
                    remote_revoked_at = ?3, updated_at = ?3 WHERE id = ?1",
                params![id, caller.username, ts(now)],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "remote_operation_revoked",
                json!({}),
            )?;
        }
        load_enrollment(&conn, id, now, gate)?.ok_or(CommercialError::NotFound)
    }

    pub fn withdraw_enrollment(&self, caller: &Caller, id: &str) -> Result<Enrollment> {
        let gate = remote_management_enabled();
        let conn = self.lock()?;
        let e = self.visible_enrollment(&conn, caller, id, gate)?;
        self.customer_only(&conn, caller, &e.org_id)?;
        if e.status != "enrolled" {
            return Err(CommercialError::Conflict(
                "the cluster is already withdrawn".into(),
            ));
        }
        let now = Utc::now();
        conn.execute(
            "UPDATE managed_enrollments SET status = 'withdrawn', remote_enabled = 0,
                remote_revoked_by = ?2, remote_revoked_at = ?3, updated_at = ?3 WHERE id = ?1",
            params![id, caller.username, ts(now)],
        )?;
        service_event(
            &conn,
            "enrollment",
            id,
            &caller.username,
            "withdrawn",
            json!({}),
        )?;
        load_enrollment(&conn, id, now, gate)?.ok_or(CommercialError::NotFound)
    }

    pub fn set_operational_owner(
        &self,
        caller: &Caller,
        id: &str,
        owner: Option<&str>,
    ) -> Result<Enrollment> {
        Self::desk_only(caller)?;
        let owner = owner.map(|o| bounded("owner", o, 128, true)).transpose()?;
        let gate = remote_management_enabled();
        let conn = self.lock()?;
        let e = self.visible_enrollment(&conn, caller, id, gate)?;
        conn.execute(
            "UPDATE managed_enrollments SET operational_owner = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, owner, ts(Utc::now())],
        )?;
        service_event(
            &conn,
            "enrollment",
            id,
            &caller.username,
            "owner_set",
            json!({ "from": e.operational_owner, "to": owner }),
        )?;
        load_enrollment(&conn, id, Utc::now(), gate)?.ok_or(CommercialError::NotFound)
    }

    // ── maintenance tasks ────────────────────────────────────────

    pub fn propose_task(
        &self,
        caller: &Caller,
        id: &str,
        input: NewTask,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        let title = bounded("title", &input.title, 200, true)?;
        let description = bounded("description", &input.description, 4000, true)?;
        let window = bounded("window", &input.window, 200, false)?;
        {
            let conn = self.lock()?;
            let e = self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            if e.status != "enrolled" {
                return Err(CommercialError::Conflict("the cluster is withdrawn".into()));
            }
            let now = ts(Utc::now());
            let tid = new_id("task");
            conn.execute(
                "INSERT INTO managed_tasks (id, enrollment_id, title, description, window_text, status,
                    proposed_by, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,'proposed',?6,?7,?7)",
                params![tid, id, title, description, window, caller.username, now],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "task_proposed",
                json!({ "task": tid, "title": title }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    /// The customer approves a proposed task before it can be scheduled.
    pub fn approve_task(
        &self,
        caller: &Caller,
        id: &str,
        task_id: &str,
    ) -> Result<EnrollmentDetail> {
        {
            let conn = self.lock()?;
            let e = self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            self.customer_only(&conn, caller, &e.org_id)?;
            let now = ts(Utc::now());
            let n = conn.execute(
                "UPDATE managed_tasks SET status = 'approved', approved_by = ?3, approved_at = ?4, updated_at = ?4
                 WHERE id = ?2 AND enrollment_id = ?1 AND status = 'proposed'",
                params![id, task_id, caller.username, now],
            )?;
            if n == 0 {
                return Err(CommercialError::Conflict(
                    "task not found or not awaiting approval".into(),
                ));
            }
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "task_approved",
                json!({ "task": task_id }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    pub fn set_task_status(
        &self,
        caller: &Caller,
        id: &str,
        task_id: &str,
        to: &str,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        if !matches!(to, "scheduled" | "done" | "cancelled") {
            return Err(CommercialError::Invalid(
                "status must be scheduled, done or cancelled".into(),
            ));
        }
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            let allowed_from: &[&str] = match to {
                "scheduled" => &["approved"],
                "done" => &["approved", "scheduled"],
                _ => &["proposed", "approved", "scheduled"],
            };
            let current: String = conn
                .query_row(
                    "SELECT status FROM managed_tasks WHERE id = ?2 AND enrollment_id = ?1",
                    params![id, task_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(CommercialError::NotFound)?;
            if !allowed_from.contains(&current.as_str()) {
                return Err(CommercialError::Conflict(format!(
                    "a {current} task cannot become {to}; tasks need customer approval first"
                )));
            }
            let now = ts(Utc::now());
            conn.execute(
                "UPDATE managed_tasks SET status = ?3, completed_at = ?4, updated_at = ?5 WHERE id = ?2 AND enrollment_id = ?1",
                params![id, task_id, to, (to == "done").then(|| now.clone()), now],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "task_status",
                json!({ "task": task_id, "from": current, "to": to }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    // ── evidence, incidents, reports ─────────────────────────────

    pub fn add_recovery_evidence(
        &self,
        caller: &Caller,
        id: &str,
        input: NewRecoveryEvidence,
    ) -> Result<EnrollmentDetail> {
        if !matches!(input.kind.as_str(), "backup" | "restore_test") {
            return Err(CommercialError::Invalid(
                "kind must be backup or restore_test".into(),
            ));
        }
        if !matches!(input.result.as_str(), "pass" | "fail") {
            return Err(CommercialError::Invalid(
                "result must be pass or fail".into(),
            ));
        }
        let performed = parse_ts(&input.performed_at)
            .map_err(|_| CommercialError::Invalid("performed_at must be RFC 3339".into()))?;
        if performed > Utc::now() + Duration::minutes(5) {
            return Err(CommercialError::Invalid(
                "performed_at cannot be in the future".into(),
            ));
        }
        let reference = input
            .reference
            .as_deref()
            .map(|r| bounded("reference", r, 500, false))
            .transpose()?;
        let note = input
            .note
            .as_deref()
            .map(|n| bounded("note", n, 4000, false))
            .transpose()?;
        if reference.as_deref().unwrap_or("").is_empty() && note.as_deref().unwrap_or("").is_empty()
        {
            return Err(CommercialError::Invalid(
                "evidence needs a reference or a note".into(),
            ));
        }
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            conn.execute(
                "INSERT INTO managed_evidence (id, enrollment_id, kind, result, performed_at, reference, note, added_by, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![new_id("rev"), id, input.kind, input.result, ts(performed), reference, note, caller.username, ts(Utc::now())],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "recovery_evidence_added",
                json!({ "kind": input.kind, "result": input.result }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    pub fn open_incident(
        &self,
        caller: &Caller,
        id: &str,
        input: NewIncident,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        if !super::model::SEVERITIES.contains(&input.severity.as_str()) {
            return Err(CommercialError::Invalid(
                "severity must be sev1..sev4".into(),
            ));
        }
        let title = bounded("title", &input.title, 200, true)?;
        let summary = bounded("summary", &input.summary, 4000, true)?;
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            let now = ts(Utc::now());
            let iid = new_id("inc");
            conn.execute(
                "INSERT INTO managed_incidents (id, enrollment_id, title, severity, status, summary, opened_by, opened_at, updated_at)
                 VALUES (?1,?2,?3,?4,'open',?5,?6,?7,?7)",
                params![iid, id, title, input.severity, summary, caller.username, now],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "incident_opened",
                json!({ "incident": iid, "severity": input.severity }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    pub fn update_incident(
        &self,
        caller: &Caller,
        id: &str,
        incident_id: &str,
        input: IncidentUpdate,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        if !matches!(input.status.as_str(), "open" | "mitigated" | "resolved") {
            return Err(CommercialError::Invalid(
                "status must be open, mitigated or resolved".into(),
            ));
        }
        let summary = input
            .summary
            .as_deref()
            .map(|s| bounded("summary", s, 4000, true))
            .transpose()?;
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            let now = ts(Utc::now());
            let n = conn.execute(
                "UPDATE managed_incidents SET status = ?3, summary = COALESCE(?4, summary),
                    resolved_at = CASE WHEN ?3 = 'resolved' THEN ?5 ELSE NULL END, updated_at = ?5
                 WHERE id = ?2 AND enrollment_id = ?1",
                params![id, incident_id, input.status, summary, now],
            )?;
            if n == 0 {
                return Err(CommercialError::NotFound);
            }
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "incident_updated",
                json!({ "incident": incident_id, "status": input.status }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    pub fn add_service_report(
        &self,
        caller: &Caller,
        id: &str,
        input: NewReport,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        let start = parse_ts(&input.period_start)
            .map_err(|_| CommercialError::Invalid("period_start must be RFC 3339".into()))?;
        let end = parse_ts(&input.period_end)
            .map_err(|_| CommercialError::Invalid("period_end must be RFC 3339".into()))?;
        if end <= start {
            return Err(CommercialError::Invalid(
                "period_end must be after period_start".into(),
            ));
        }
        let summary = bounded("summary", &input.summary, 8000, true)?;
        let reference = input
            .reference
            .as_deref()
            .map(|r| bounded("reference", r, 500, false))
            .transpose()?;
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            conn.execute(
                "INSERT INTO managed_reports (id, enrollment_id, period_start, period_end, summary, reference, added_by, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![new_id("rep"), id, ts(start), ts(end), summary, reference, caller.username, ts(Utc::now())],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "report_added",
                json!({ "from": ts(start), "to": ts(end) }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    // ── remediation policies ─────────────────────────────────────

    /// The desk drafts a documented policy; nothing is permitted until the
    /// customer authorizes it.
    pub fn create_policy(
        &self,
        caller: &Caller,
        id: &str,
        input: NewPolicy,
    ) -> Result<EnrollmentDetail> {
        Self::desk_only(caller)?;
        let name = bounded("name", &input.name, 200, true)?;
        let description = bounded("description", &input.description, 8000, true)?;
        if description.chars().count() < 20 {
            return Err(CommercialError::Invalid(
                "a remediation policy must be documented: describe what it does and when (20+ characters)".into(),
            ));
        }
        if input.permissions.is_empty() || input.permissions.len() > 20 {
            return Err(CommercialError::Invalid(
                "list the exact permissions the policy needs (1-20)".into(),
            ));
        }
        let permissions = clean_list("permissions", &input.permissions)?;
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            let pid = new_id("pol");
            conn.execute(
                "INSERT INTO remediation_policies (id, enrollment_id, name, description, permissions, status, created_by, created_at)
                 VALUES (?1,?2,?3,?4,?5,'draft',?6,?7)",
                params![
                    pid,
                    id,
                    name,
                    description,
                    serde_json::to_string(&permissions).map_err(anyhow::Error::from)?,
                    caller.username,
                    ts(Utc::now())
                ],
            )?;
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "policy_drafted",
                json!({ "policy": pid, "permissions": permissions }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    pub fn authorize_policy(
        &self,
        caller: &Caller,
        id: &str,
        policy_id: &str,
    ) -> Result<EnrollmentDetail> {
        {
            let conn = self.lock()?;
            let e = self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            self.customer_only(&conn, caller, &e.org_id)?;
            let now = ts(Utc::now());
            let n = conn.execute(
                "UPDATE remediation_policies SET status = 'authorized', authorized_by = ?3, authorized_at = ?4
                 WHERE id = ?2 AND enrollment_id = ?1 AND status = 'draft'",
                params![id, policy_id, caller.username, now],
            )?;
            if n == 0 {
                return Err(CommercialError::Conflict(
                    "policy not found or not a draft".into(),
                ));
            }
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "policy_authorized",
                json!({ "policy": policy_id }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }

    /// Revocation is open to the customer and the desk.
    pub fn revoke_policy(
        &self,
        caller: &Caller,
        id: &str,
        policy_id: &str,
    ) -> Result<EnrollmentDetail> {
        {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, remote_management_enabled())?;
            let now = ts(Utc::now());
            let n = conn.execute(
                "UPDATE remediation_policies SET status = 'revoked', revoked_by = ?3, revoked_at = ?4
                 WHERE id = ?2 AND enrollment_id = ?1 AND status IN ('draft','authorized')",
                params![id, policy_id, caller.username, now],
            )?;
            if n == 0 {
                return Err(CommercialError::Conflict(
                    "policy not found or already revoked".into(),
                ));
            }
            service_event(
                &conn,
                "enrollment",
                id,
                &caller.username,
                "policy_revoked",
                json!({ "policy": policy_id }),
            )?;
        }
        self.enrollment_detail(caller, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commercial::model::{CoverageHours, Entitlement, SupportContact};
    use crate::commercial::store::{QuoteInput, QuoteRequestInput};

    fn desk() -> Caller {
        Caller {
            username: "desk".into(),
            admin: true,
        }
    }
    fn user(n: &str) -> Caller {
        Caller {
            username: n.into(),
            admin: false,
        }
    }

    /// Org with an active contract of `offering` covering `cluster`.
    fn org_with(
        s: &CommercialStore,
        name: &str,
        member: &str,
        offering: &str,
        cluster: &str,
    ) -> (String, String) {
        let org = s.create_org(name, Some(member)).unwrap();
        let req = s
            .create_quote_request(
                &user(member),
                QuoteRequestInput {
                    org_id: Some(org.id.clone()),
                    offering: offering.into(),
                    company: name.into(),
                    contact_name: "C".into(),
                    contact_email: "c@x.test".into(),
                    cluster_count: 1,
                    worker_node_count: 3,
                    workload_size: String::new(),
                    region: String::new(),
                    desired_coverage: String::new(),
                    requirements: String::new(),
                },
            )
            .unwrap();
        let c = s
            .generate_quote(
                "desk",
                &req.id,
                QuoteInput {
                    org_id: None,
                    currency: "USD".into(),
                    pricing_unit: "cluster / month".into(),
                    unit_price_minor: None,
                    included_capacity: String::new(),
                    duration_months: 12,
                    response_targets: vec!["sev1: 1h".into()],
                    support_hours: String::new(),
                    extra_exclusions: vec![],
                    node_treatment: String::new(),
                },
            )
            .unwrap();
        s.update_entitlement(
            "desk",
            &c.id,
            Entitlement {
                covered_clusters: vec![cluster.into()],
                support_tier: offering.into(),
                coverage_hours: CoverageHours {
                    always: true,
                    ..Default::default()
                },
                timezone: "UTC".into(),
                authorized_contacts: vec![SupportContact {
                    name: "C".into(),
                    email: "c@x.test".into(),
                }],
                effective_at: Some(Utc::now() - Duration::days(1)),
                expires_at: Some(Utc::now() + Duration::days(300)),
                ..Default::default()
            },
        )
        .unwrap();
        s.transition("desk", &c.id, ContractStatus::AwaitingAcceptance)
            .unwrap();
        s.accept(&user(member), &c.id).unwrap();
        (org.id, c.id)
    }

    fn enroll(s: &CommercialStore, org: &str, cluster: &str) -> Enrollment {
        s.enroll_cluster_with_gate(
            &user("ann"),
            NewEnrollment {
                org_id: org.into(),
                cluster_id: cluster.into(),
                covered_components: vec!["zorvia".into(), "kubevirt".into()],
                maintenance_windows: vec!["Sun 02:00-04:00 UTC".into()],
                monitoring_signals: vec!["node ready".into()],
            },
            true,
        )
        .unwrap()
    }

    #[test]
    fn enrollment_is_a_customer_decision_and_needs_managed() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let new = |c: &str| NewEnrollment {
            org_id: org.clone(),
            cluster_id: c.into(),
            covered_components: vec![],
            maintenance_windows: vec![],
            monitoring_signals: vec![],
        };
        // The desk cannot enroll on the customer's behalf.
        assert!(matches!(
            s.enroll_cluster_with_gate(&desk(), new("cl-1"), true),
            Err(CommercialError::Forbidden(_))
        ));
        // Strangers learn nothing.
        assert!(matches!(
            s.enroll_cluster_with_gate(&user("bob"), new("cl-1"), true),
            Err(CommercialError::NotFound)
        ));
        // A cluster the Managed contract does not cover.
        assert!(matches!(
            s.enroll_cluster_with_gate(&user("ann"), new("cl-9"), true),
            Err(CommercialError::Conflict(_))
        ));
        let e = enroll(&s, &org, "cl-1");
        assert_eq!(e.status, "enrolled");
        assert!(e.service_eligible);
        assert!(
            !e.remote_operation.requested,
            "purchase must not grant access"
        );
        assert!(matches!(
            s.enroll_cluster_with_gate(&user("ann"), new("cl-1"), true),
            Err(CommercialError::Conflict(_))
        ));
    }

    #[test]
    fn supported_contract_cannot_enroll_in_managed() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "supported", "cl-1");
        let r = s.enroll_cluster_with_gate(
            &user("ann"),
            NewEnrollment {
                org_id: org,
                cluster_id: "cl-1".into(),
                covered_components: vec![],
                maintenance_windows: vec![],
                monitoring_signals: vec![],
            },
            true,
        );
        assert!(matches!(r, Err(CommercialError::Conflict(_))));
    }

    #[test]
    fn remote_operation_is_gated_scoped_and_revocable() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let e = enroll(&s, &org, "cl-1");
        let ask = |scope: &[&str], cred: Option<&str>| RemoteRequest {
            enable: true,
            scope: scope.iter().map(|s| s.to_string()).collect(),
            credential_ref: cred.map(str::to_string),
        };
        // Server gate off (the default): refused.
        assert!(matches!(
            s.set_remote_operation_with_gate(
                &user("ann"),
                &e.id,
                ask(&["monitoring_read"], Some("secret/ref")),
                false
            ),
            Err(CommercialError::Forbidden(_))
        ));
        // The desk cannot turn on access to a customer's cluster.
        assert!(matches!(
            s.set_remote_operation_with_gate(
                &desk(),
                &e.id,
                ask(&["monitoring_read"], Some("secret/ref")),
                true
            ),
            Err(CommercialError::Forbidden(_))
        ));
        // Scope and credential reference are mandatory and validated.
        assert!(s
            .set_remote_operation_with_gate(&user("ann"), &e.id, ask(&[], Some("c")), true)
            .is_err());
        assert!(s
            .set_remote_operation_with_gate(&user("ann"), &e.id, ask(&["root"], Some("c")), true)
            .is_err());
        assert!(s
            .set_remote_operation_with_gate(
                &user("ann"),
                &e.id,
                ask(&["monitoring_read"], None),
                true
            )
            .is_err());
        // Remediation scope needs an authorized policy.
        assert!(matches!(
            s.set_remote_operation_with_gate(
                &user("ann"),
                &e.id,
                ask(&["remediation_actions"], Some("c")),
                true
            ),
            Err(CommercialError::Conflict(_))
        ));
        let on = s
            .set_remote_operation_with_gate(
                &user("ann"),
                &e.id,
                ask(&["monitoring_read"], Some("secret/ref")),
                true,
            )
            .unwrap();
        assert!(on.remote_operation.requested && on.remote_operation.effective);
        assert_eq!(
            on.remote_operation.credential_ref.as_deref(),
            Some("secret/ref")
        );
        // Either side can revoke, immediately.
        let off = s
            .set_remote_operation_with_gate(
                &desk(),
                &e.id,
                RemoteRequest {
                    enable: false,
                    scope: vec![],
                    credential_ref: None,
                },
                true,
            )
            .unwrap();
        assert!(!off.remote_operation.requested && !off.remote_operation.effective);
        assert_eq!(off.remote_operation.revoked_by.as_deref(), Some("desk"));
    }

    #[test]
    fn expiry_ends_the_service_and_effective_access() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, contract) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let e = enroll(&s, &org, "cl-1");
        s.set_remote_operation_with_gate(
            &user("ann"),
            &e.id,
            RemoteRequest {
                enable: true,
                scope: vec!["monitoring_read".into()],
                credential_ref: Some("ref".into()),
            },
            true,
        )
        .unwrap();
        {
            let conn = s.conn.lock().unwrap();
            let mut ent = Entitlement {
                covered_clusters: vec!["cl-1".into()],
                support_tier: "managed".into(),
                coverage_hours: CoverageHours {
                    always: true,
                    ..Default::default()
                },
                timezone: "UTC".into(),
                effective_at: Some(Utc::now() - Duration::days(30)),
                expires_at: Some(Utc::now() - Duration::hours(1)),
                ..Default::default()
            };
            ent.authorized_contacts = vec![];
            conn.execute(
                "UPDATE contracts SET entitlement = ?2 WHERE id = ?1",
                params![contract, serde_json::to_string(&ent).unwrap()],
            )
            .unwrap();
        }
        let after = s
            .get_enrollment_with_gate(&user("ann"), &e.id, true)
            .unwrap();
        assert!(!after.service_eligible);
        assert!(after.remote_operation.requested, "the record stays");
        assert!(!after.remote_operation.effective, "but it cannot be used");
    }

    #[test]
    fn tasks_need_customer_approval_before_they_can_run() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let e = enroll(&s, &org, "cl-1");
        let task = NewTask {
            title: "Upgrade KubeVirt".into(),
            description: "Minor upgrade in window".into(),
            window: "Sun 02:00".into(),
        };
        assert!(matches!(
            s.propose_task(&user("ann"), &e.id, task.clone()),
            Err(CommercialError::Forbidden(_))
        ));
        let d = s.propose_task(&desk(), &e.id, task).unwrap();
        let tid = d.tasks[0].id.clone();
        assert_eq!(d.tasks[0].status, "proposed");
        assert!(matches!(
            s.set_task_status(&desk(), &e.id, &tid, "done"),
            Err(CommercialError::Conflict(_))
        ));
        assert!(matches!(
            s.set_task_status(&desk(), &e.id, &tid, "scheduled"),
            Err(CommercialError::Conflict(_))
        ));
        assert!(matches!(
            s.approve_task(&desk(), &e.id, &tid),
            Err(CommercialError::Forbidden(_))
        ));
        s.approve_task(&user("ann"), &e.id, &tid).unwrap();
        s.set_task_status(&desk(), &e.id, &tid, "scheduled")
            .unwrap();
        let d = s.set_task_status(&desk(), &e.id, &tid, "done").unwrap();
        assert_eq!(d.tasks[0].status, "done");
        assert_eq!(d.tasks[0].approved_by.as_deref(), Some("ann"));
        assert!(d.tasks[0].completed_at.is_some());
    }

    #[test]
    fn remediation_policy_needs_documentation_and_customer_authorization() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let e = enroll(&s, &org, "cl-1");
        let thin = NewPolicy {
            name: "restart".into(),
            description: "short".into(),
            permissions: vec!["vm.restart".into()],
        };
        assert!(s.create_policy(&desk(), &e.id, thin).is_err());
        let none = NewPolicy {
            name: "restart".into(),
            description: "Restart a VM that fails its health check twice".into(),
            permissions: vec![],
        };
        assert!(s.create_policy(&desk(), &e.id, none).is_err());
        let ok = NewPolicy {
            name: "restart".into(),
            description: "Restart a VM that fails its health check twice".into(),
            permissions: vec!["vm.restart".into()],
        };
        let d = s.create_policy(&desk(), &e.id, ok).unwrap();
        let pid = d.policies[0].id.clone();
        assert_eq!(d.policies[0].status, "draft");
        assert!(matches!(
            s.authorize_policy(&desk(), &e.id, &pid),
            Err(CommercialError::Forbidden(_))
        ));
        let d = s.authorize_policy(&user("ann"), &e.id, &pid).unwrap();
        assert_eq!(d.policies[0].status, "authorized");
        assert_eq!(d.policies[0].authorized_by.as_deref(), Some("ann"));
        let d = s.revoke_policy(&user("ann"), &e.id, &pid).unwrap();
        assert_eq!(d.policies[0].status, "revoked");
        assert!(d.events.iter().any(|v| v["event"] == "policy_authorized"));
        assert!(d.events.iter().any(|v| v["event"] == "policy_revoked"));
    }

    #[test]
    fn evidence_incidents_reports_and_isolation() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let (_o2, _) = org_with(&s, "Beta", "bob", "managed", "cl-2");
        let e = enroll(&s, &org, "cl-1");
        let ev = |result: &str, at: &str| NewRecoveryEvidence {
            kind: "restore_test".into(),
            result: result.into(),
            performed_at: at.into(),
            reference: Some("runbook#7".into()),
            note: None,
        };
        assert!(s
            .add_recovery_evidence(&user("ann"), &e.id, ev("maybe", "2026-01-01T00:00:00Z"))
            .is_err());
        let future = (Utc::now() + Duration::days(2)).to_rfc3339();
        assert!(s
            .add_recovery_evidence(&user("ann"), &e.id, ev("pass", &future))
            .is_err());
        let d = s
            .add_recovery_evidence(&user("ann"), &e.id, ev("pass", "2026-01-01T00:00:00Z"))
            .unwrap();
        assert_eq!(d.recovery_evidence.len(), 1);

        assert!(matches!(
            s.open_incident(
                &user("ann"),
                &e.id,
                NewIncident {
                    title: "x".into(),
                    severity: "sev2".into(),
                    summary: "y".into()
                }
            ),
            Err(CommercialError::Forbidden(_))
        ));
        let d = s
            .open_incident(
                &desk(),
                &e.id,
                NewIncident {
                    title: "Node NotReady".into(),
                    severity: "sev2".into(),
                    summary: "node-3".into(),
                },
            )
            .unwrap();
        let iid = d.incidents[0].id.clone();
        let d = s
            .update_incident(
                &desk(),
                &e.id,
                &iid,
                IncidentUpdate {
                    status: "resolved".into(),
                    summary: Some("replaced".into()),
                },
            )
            .unwrap();
        assert!(d.incidents[0].resolved_at.is_some());
        let d = s
            .add_service_report(
                &desk(),
                &e.id,
                NewReport {
                    period_start: "2026-01-01T00:00:00Z".into(),
                    period_end: "2026-02-01T00:00:00Z".into(),
                    summary: "Jan".into(),
                    reference: None,
                },
            )
            .unwrap();
        assert_eq!(d.reports.len(), 1);
        assert!(s
            .add_service_report(
                &desk(),
                &e.id,
                NewReport {
                    period_start: "2026-02-01T00:00:00Z".into(),
                    period_end: "2026-01-01T00:00:00Z".into(),
                    summary: "bad".into(),
                    reference: None
                },
            )
            .is_err());

        let bob = user("bob");
        assert!(matches!(
            s.enrollment_detail(&bob, &e.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.approve_task(&bob, &e.id, "x"),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.add_recovery_evidence(&bob, &e.id, ev("pass", "2026-01-01T00:00:00Z")),
            Err(CommercialError::NotFound)
        ));
        assert!(s
            .list_enrollments(&bob)
            .unwrap()
            .iter()
            .all(|x| x.org_id != org));
    }

    #[test]
    fn withdrawal_revokes_remote_access() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, _) = org_with(&s, "Acme", "ann", "managed", "cl-1");
        let e = enroll(&s, &org, "cl-1");
        s.set_remote_operation_with_gate(
            &user("ann"),
            &e.id,
            RemoteRequest {
                enable: true,
                scope: vec!["monitoring_read".into()],
                credential_ref: Some("ref".into()),
            },
            true,
        )
        .unwrap();
        assert!(matches!(
            s.withdraw_enrollment(&desk(), &e.id),
            Err(CommercialError::Forbidden(_))
        ));
        let w = s.withdraw_enrollment(&user("ann"), &e.id).unwrap();
        assert_eq!(w.status, "withdrawn");
        assert!(!w.remote_operation.requested);
        // Can re-enroll after withdrawing.
        let again = enroll(&s, &org, "cl-1");
        assert_ne!(again.id, e.id);
    }

    #[test]
    fn remote_gate_env_parsing() {
        // Default (unset in the test environment) is off.
        if std::env::var("ZORVIA_MANAGED_REMOTE_ENABLED").is_err() {
            assert!(!remote_management_enabled());
        }
    }

    impl CommercialStore {
        pub(super) fn get_enrollment_with_gate(
            &self,
            caller: &Caller,
            id: &str,
            gate: bool,
        ) -> Result<Enrollment> {
            let conn = self.lock()?;
            self.visible_enrollment(&conn, caller, id, gate)
        }
    }
}
