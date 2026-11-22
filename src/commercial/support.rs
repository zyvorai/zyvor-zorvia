// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Support cases: lifecycle, customer-visible responses, attachments,
//! contract-coverage snapshot, first-response timers and escalation history.
//!
//! Lives in the commercial store, so organization isolation uses the same
//! `Caller` rules as contracts: non-members get `NotFound`.
//!
//! Who is "support"? A caller with `users.admin` is the support desk
//! (`Caller::admin`); everyone else is a customer member of an
//! organization. Internal notes are visible only to the desk.

use super::catalog;
use super::model::*;
use super::store::{
    bounded, is_member, load_contract, new_id, org_exists, parse_ts, ts, Caller, CommercialError,
    CommercialStore, Result,
};
use super::timers::Schedule;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Open,
    Triaged,
    Investigating,
    WaitingOnCustomer,
    Resolved,
    Closed,
}

impl CaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Triaged => "triaged",
            Self::Investigating => "investigating",
            Self::WaitingOnCustomer => "waiting_on_customer",
            Self::Resolved => "resolved",
            Self::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => Self::Open,
            "triaged" => Self::Triaged,
            "investigating" => Self::Investigating,
            "waiting_on_customer" => Self::WaitingOnCustomer,
            "resolved" => Self::Resolved,
            "closed" => Self::Closed,
            _ => return None,
        })
    }

    /// `open → triaged → investigating ⇄ waiting_on_customer → resolved →
    /// closed`, with reopening (`resolved`/`closed → investigating`).
    pub fn can_transition_to(self, next: Self) -> bool {
        use CaseStatus::*;
        matches!(
            (self, next),
            (Open, Triaged)
                | (Triaged, Investigating)
                | (Investigating, WaitingOnCustomer)
                | (WaitingOnCustomer, Investigating)
                | (Investigating, Resolved)
                | (WaitingOnCustomer, Resolved)
                | (Resolved, Closed)
                | (Resolved, Investigating)
                | (Closed, Investigating)
        )
    }
}

/// Contract coverage frozen at case creation. Later contract edits do not
/// move the goalposts of an open case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageSnapshot {
    pub covered: bool,
    pub reason: Option<String>,
    pub contract_id: Option<String>,
    pub support_tier: String,
    pub timezone: String,
    pub coverage_hours: CoverageHours,
    pub holidays: Vec<String>,
    /// This case's severity is covered 24x7 rather than by `coverage_hours`.
    pub always_on: bool,
    pub first_response_target_minutes: Option<u32>,
    pub resolution_estimate_minutes: Option<u32>,
    pub captured_at: DateTime<Utc>,
}

impl CoverageSnapshot {
    fn schedule(&self) -> Option<Schedule> {
        if !self.covered {
            return None;
        }
        if self.always_on {
            Schedule::always(&self.timezone).ok()
        } else {
            Schedule::new(&self.timezone, &self.coverage_hours, &self.holidays).ok()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseTimer {
    pub target_minutes: Option<u32>,
    pub due: Option<DateTime<Utc>>,
    pub responded_at: Option<DateTime<Utc>>,
    /// `not_applicable`, `pending`, `met` or `breached`.
    pub state: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolutionTimer {
    pub estimate_minutes: Option<u32>,
    pub due: Option<DateTime<Utc>>,
    pub paused: bool,
    pub remaining_covered_seconds: Option<i64>,
    /// Always present: an estimate is not a commitment.
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupportCase {
    pub id: String,
    pub org_id: String,
    pub cluster_id: String,
    pub contract_id: Option<String>,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub status: CaseStatus,
    pub owner: Option<String>,
    pub created_by: String,
    pub coverage: CoverageSnapshot,
    pub first_response: ResponseTimer,
    pub resolution: ResolutionTimer,
    pub reopen_count: u32,
    pub resolved_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseMessage {
    pub seq: i64,
    pub at: DateTime<Utc>,
    pub author: String,
    /// `customer` or `support`.
    pub author_kind: String,
    pub body: String,
    pub internal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentMeta {
    pub id: String,
    pub message_seq: Option<i64>,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub uploaded_by: String,
    pub at: DateTime<Utc>,
    pub purged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseEvent {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub event: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseDetail {
    pub case: SupportCase,
    pub messages: Vec<CaseMessage>,
    pub attachments: Vec<AttachmentMeta>,
    pub events: Vec<CaseEvent>,
    /// Subset of `events`: every escalation, oldest first.
    pub escalations: Vec<CaseEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewCase {
    pub org_id: String,
    pub cluster_id: String,
    pub severity: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Attachment limits, configurable per deployment.
#[derive(Debug, Clone, Copy)]
pub struct SupportLimits {
    pub attachment_max_bytes: usize,
    pub retention_days: i64,
}

impl SupportLimits {
    pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 5;

    /// `ZORVIA_SUPPORT_ATTACHMENT_MAX_BYTES` (default 5 MiB, at most 6 MiB so
    /// a base64 upload fits the 10 MiB request limit) and
    /// `ZORVIA_SUPPORT_ATTACHMENT_RETENTION_DAYS` (default 90).
    pub fn from_env() -> Self {
        let bytes = std::env::var("ZORVIA_SUPPORT_ATTACHMENT_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(5 * 1024 * 1024)
            .clamp(1024, 6 * 1024 * 1024);
        let days = std::env::var("ZORVIA_SUPPORT_ATTACHMENT_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(90)
            .clamp(1, 3650);
        Self {
            attachment_max_bytes: bytes,
            retention_days: days,
        }
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned
    }
}

fn case_event(
    conn: &Connection,
    case_id: &str,
    actor: &str,
    event: &str,
    detail: serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO support_events (case_id, at, actor, event, detail) VALUES (?1,?2,?3,?4,?5)",
        params![case_id, ts(Utc::now()), actor, event, detail.to_string()],
    )?;
    Ok(())
}

struct RawCase {
    id: String,
    org_id: String,
    cluster_id: String,
    contract_id: Option<String>,
    severity: String,
    title: String,
    description: String,
    status: String,
    owner: Option<String>,
    created_by: String,
    coverage: String,
    first_response_due: Option<String>,
    first_response_at: Option<String>,
    resolution_due: Option<String>,
    resolution_paused_secs: Option<i64>,
    reopen_count: u32,
    resolved_at: Option<String>,
    closed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

const CASE_COLS: &str = "id, org_id, cluster_id, contract_id, severity, title, description, status,
    owner, created_by, coverage, first_response_due, first_response_at, resolution_due,
    resolution_paused_secs, reopen_count, resolved_at, closed_at, created_at, updated_at";

fn opt_ts(s: &Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.as_deref().map(parse_ts).transpose().map_err(Into::into)
}

fn load_case(conn: &Connection, id: &str, now: DateTime<Utc>) -> Result<Option<SupportCase>> {
    let sql = format!("SELECT {CASE_COLS} FROM support_cases WHERE id = ?1");
    let raw = conn
        .query_row(&sql, [id], |r| {
            Ok(RawCase {
                id: r.get(0)?,
                org_id: r.get(1)?,
                cluster_id: r.get(2)?,
                contract_id: r.get(3)?,
                severity: r.get(4)?,
                title: r.get(5)?,
                description: r.get(6)?,
                status: r.get(7)?,
                owner: r.get(8)?,
                created_by: r.get(9)?,
                coverage: r.get(10)?,
                first_response_due: r.get(11)?,
                first_response_at: r.get(12)?,
                resolution_due: r.get(13)?,
                resolution_paused_secs: r.get(14)?,
                reopen_count: r.get(15)?,
                resolved_at: r.get(16)?,
                closed_at: r.get(17)?,
                created_at: r.get(18)?,
                updated_at: r.get(19)?,
            })
        })
        .optional()?;
    let Some(raw) = raw else { return Ok(None) };
    let status = CaseStatus::parse(&raw.status)
        .ok_or_else(|| anyhow::anyhow!("corrupt case status '{}'", raw.status))?;
    let coverage: CoverageSnapshot =
        serde_json::from_str(&raw.coverage).map_err(anyhow::Error::from)?;
    let due = opt_ts(&raw.first_response_due)?;
    let responded_at = opt_ts(&raw.first_response_at)?;
    let state = match (due, responded_at) {
        (None, _) => "not_applicable",
        (Some(d), Some(r)) => {
            if r <= d {
                "met"
            } else {
                "breached"
            }
        }
        (Some(d), None) => {
            if now > d {
                "breached"
            } else {
                "pending"
            }
        }
    };
    let first_response = ResponseTimer {
        target_minutes: coverage.first_response_target_minutes,
        due,
        responded_at,
        state,
    };
    let resolution = ResolutionTimer {
        estimate_minutes: coverage.resolution_estimate_minutes,
        due: opt_ts(&raw.resolution_due)?,
        paused: raw.resolution_paused_secs.is_some(),
        remaining_covered_seconds: raw.resolution_paused_secs,
        note: "Estimate only; not a resolution commitment.",
    };
    Ok(Some(SupportCase {
        id: raw.id,
        org_id: raw.org_id,
        cluster_id: raw.cluster_id,
        contract_id: raw.contract_id,
        severity: raw.severity,
        title: raw.title,
        description: raw.description,
        status,
        owner: raw.owner,
        created_by: raw.created_by,
        coverage,
        first_response,
        resolution,
        reopen_count: raw.reopen_count,
        resolved_at: opt_ts(&raw.resolved_at)?,
        closed_at: opt_ts(&raw.closed_at)?,
        created_at: parse_ts(&raw.created_at)?,
        updated_at: parse_ts(&raw.updated_at)?,
    }))
}

/// Freeze the org's best active coverage for `cluster` at `now`.
fn snapshot_coverage(
    conn: &Connection,
    org_id: &str,
    cluster: &str,
    severity: &str,
    now: DateTime<Utc>,
) -> Result<CoverageSnapshot> {
    let mut stmt =
        conn.prepare("SELECT id FROM contracts WHERE org_id = ?1 AND status = 'active'")?;
    let ids: Vec<String> = stmt
        .query_map([org_id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let mut best: Option<Contract> = None;
    for id in ids {
        let Some(c) = load_contract(conn, &id, now)? else {
            continue;
        };
        let covers = c.effective_status == ContractStatus::Active
            && catalog::find(&c.offering).is_some_and(|o| o.grants_coverage)
            && c.entitlement.covered_clusters.iter().any(|x| x == cluster);
        if covers
            && best
                .as_ref()
                .is_none_or(|b| c.entitlement.expires_at > b.entitlement.expires_at)
        {
            best = Some(c);
        }
    }
    let Some(c) = best else {
        return Ok(CoverageSnapshot {
            covered: false,
            reason: Some(
                "No active coverage contract for this cluster. The case is recorded without response commitments."
                    .into(),
            ),
            contract_id: None,
            support_tier: String::new(),
            timezone: String::new(),
            coverage_hours: CoverageHours::default(),
            holidays: vec![],
            always_on: false,
            first_response_target_minutes: None,
            resolution_estimate_minutes: None,
            captured_at: now,
        });
    };
    let e = &c.entitlement;
    Ok(CoverageSnapshot {
        covered: true,
        reason: None,
        contract_id: Some(c.id.clone()),
        support_tier: e.support_tier.clone(),
        timezone: e.timezone.clone(),
        coverage_hours: e.coverage_hours.clone(),
        holidays: e.holidays.clone(),
        always_on: e.always_on_severities.iter().any(|s| s == severity),
        first_response_target_minutes: e.response_target_minutes.get(severity).copied(),
        resolution_estimate_minutes: e.resolution_estimate_minutes.get(severity).copied(),
        captured_at: now,
    })
}

fn add_minutes(
    snap: &CoverageSnapshot,
    from: DateTime<Utc>,
    minutes: Option<u32>,
) -> Option<DateTime<Utc>> {
    let m = minutes?;
    snap.schedule()?
        .add_covered_seconds(from, i64::from(m) * 60)
}

impl CommercialStore {
    pub fn create_case(&self, caller: &Caller, input: NewCase) -> Result<SupportCase> {
        if !SEVERITIES.contains(&input.severity.as_str()) {
            return Err(CommercialError::Invalid(
                "severity must be sev1..sev4".into(),
            ));
        }
        let cluster_id = bounded("cluster_id", &input.cluster_id, 128, true)?;
        let title = bounded("title", &input.title, 200, true)?;
        let description = bounded("description", &input.description, 8000, true)?;

        let conn = self.lock()?;
        org_exists(&conn, &input.org_id)?;
        if !self.allowed(&conn, caller, &input.org_id)? {
            return Err(CommercialError::NotFound);
        }
        let now = Utc::now();
        let snap = snapshot_coverage(&conn, &input.org_id, &cluster_id, &input.severity, now)?;
        let first_due = add_minutes(&snap, now, snap.first_response_target_minutes);
        let res_due = add_minutes(&snap, now, snap.resolution_estimate_minutes);
        let id = new_id("case");
        conn.execute(
            "INSERT INTO support_cases (id, org_id, cluster_id, contract_id, severity, title,
                description, status, created_by, coverage, first_response_due, resolution_due,
                created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'open',?8,?9,?10,?11,?12,?12)",
            params![
                id,
                input.org_id,
                cluster_id,
                snap.contract_id,
                input.severity,
                title,
                description,
                caller.username,
                serde_json::to_string(&snap).map_err(anyhow::Error::from)?,
                first_due.map(ts),
                res_due.map(ts),
                ts(now)
            ],
        )?;
        conn.execute(
            "INSERT INTO support_messages (case_id, at, author, author_kind, body, internal)
             VALUES (?1,?2,?3,'customer',?4,0)",
            params![id, ts(now), caller.username, description],
        )?;
        case_event(
            &conn,
            &id,
            &caller.username,
            "created",
            json!({ "severity": input.severity, "covered": snap.covered, "contract_id": snap.contract_id }),
        )?;
        load_case(&conn, &id, now)?.ok_or(CommercialError::NotFound)
    }

    pub fn list_cases(&self, caller: &Caller) -> Result<Vec<SupportCase>> {
        let conn = self.lock()?;
        let now = Utc::now();
        let mut stmt =
            conn.prepare("SELECT id, org_id FROM support_cases ORDER BY created_at DESC")?;
        let ids: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let mut out = Vec::new();
        for (id, org) in ids {
            if self.allowed(&conn, caller, &org)? {
                if let Some(c) = load_case(&conn, &id, now)? {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    /// The case, or `NotFound` when it does not exist *or* the caller is not
    /// in its organization.
    fn visible_case(&self, conn: &Connection, caller: &Caller, id: &str) -> Result<SupportCase> {
        let c = load_case(conn, id, Utc::now())?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(conn, caller, &c.org_id)? {
            return Err(CommercialError::NotFound);
        }
        Ok(c)
    }

    pub fn get_case(&self, caller: &Caller, id: &str) -> Result<SupportCase> {
        let conn = self.lock()?;
        self.visible_case(&conn, caller, id)
    }

    pub fn case_detail(&self, caller: &Caller, id: &str) -> Result<CaseDetail> {
        let conn = self.lock()?;
        let case = self.visible_case(&conn, caller, id)?;

        let mut stmt = conn.prepare(
            "SELECT seq, at, author, author_kind, body, internal FROM support_messages
             WHERE case_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (seq, at, author, author_kind, body, internal) = row?;
            if internal != 0 && !caller.admin {
                continue;
            }
            messages.push(CaseMessage {
                seq,
                at: parse_ts(&at)?,
                author,
                author_kind,
                body,
                internal: internal != 0,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT id, message_seq, filename, content_type, size, sha256, uploaded_by, at, purged_at
             FROM support_attachments WHERE case_id = ?1 ORDER BY at, id",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut attachments = Vec::new();
        for row in rows {
            let (aid, message_seq, filename, content_type, size, sha256, uploaded_by, at, purged) =
                row?;
            // An attachment on a message the caller cannot see is invisible too.
            if let Some(seq) = message_seq {
                if !caller.admin && !messages.iter().any(|m| m.seq == seq) {
                    continue;
                }
            }
            attachments.push(AttachmentMeta {
                id: aid,
                message_seq,
                filename,
                content_type,
                size,
                sha256,
                uploaded_by,
                at: parse_ts(&at)?,
                purged_at: purged.as_deref().map(parse_ts).transpose()?,
            });
        }
        drop(stmt);

        let mut stmt = conn.prepare(
            "SELECT at, actor, event, detail FROM support_events WHERE case_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (at, actor, event, detail) = row?;
            events.push(CaseEvent {
                at: parse_ts(&at)?,
                actor,
                event,
                detail: serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null),
            });
        }
        let escalations = events
            .iter()
            .filter(|e| e.event == "escalated")
            .cloned()
            .collect();
        Ok(CaseDetail {
            case,
            messages,
            attachments,
            events,
            escalations,
        })
    }

    /// Add a response. Support-desk replies that are not internal notes stop
    /// the first-response clock; a customer reply to a case that is
    /// `waiting_on_customer` moves it back to `investigating` and resumes
    /// the resolution estimate. `as_customer` makes an administrator act as
    /// the customer (used for customer-initiated uploads such as diagnostics).
    pub fn add_message(
        &self,
        caller: &Caller,
        case_id: &str,
        body: &str,
        internal: bool,
        as_customer: bool,
        attachments: Vec<NewAttachment>,
    ) -> Result<SupportCase> {
        let body = bounded("body", body, 20_000, true)?;
        let limits = SupportLimits::from_env();
        if attachments.len() > SupportLimits::MAX_ATTACHMENTS_PER_MESSAGE {
            return Err(CommercialError::Invalid(format!(
                "at most {} attachments per message",
                SupportLimits::MAX_ATTACHMENTS_PER_MESSAGE
            )));
        }
        for a in &attachments {
            if a.data.is_empty() {
                return Err(CommercialError::Invalid(
                    "attachments must not be empty".into(),
                ));
            }
            if a.data.len() > limits.attachment_max_bytes {
                return Err(CommercialError::Invalid(format!(
                    "attachment '{}' exceeds the {} byte limit",
                    sanitize_filename(&a.filename),
                    limits.attachment_max_bytes
                )));
            }
        }
        if internal && (!caller.admin || as_customer) {
            return Err(CommercialError::Forbidden(
                "internal notes are limited to the support desk".into(),
            ));
        }

        let conn = self.lock()?;
        let case = self.visible_case(&conn, caller, case_id)?;
        if case.status == CaseStatus::Closed {
            return Err(CommercialError::Conflict(
                "the case is closed; reopen it to add a response".into(),
            ));
        }
        let staff = caller.admin && !as_customer;
        let now = Utc::now();
        conn.execute(
            "INSERT INTO support_messages (case_id, at, author, author_kind, body, internal)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                case_id,
                ts(now),
                caller.username,
                if staff { "support" } else { "customer" },
                body,
                internal as i64
            ],
        )?;
        let seq = conn.last_insert_rowid();
        for a in &attachments {
            conn.execute(
                "INSERT INTO support_attachments (id, case_id, message_seq, filename, content_type,
                    size, sha256, data, uploaded_by, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    new_id("att"),
                    case_id,
                    seq,
                    sanitize_filename(&a.filename),
                    bounded("content_type", &a.content_type, 100, false)?,
                    a.data.len() as i64,
                    sha256_hex(&a.data),
                    a.data,
                    caller.username,
                    ts(now)
                ],
            )?;
        }
        conn.execute(
            "UPDATE support_cases SET updated_at = ?2 WHERE id = ?1",
            params![case_id, ts(now)],
        )?;
        case_event(
            &conn,
            case_id,
            &caller.username,
            if internal { "internal_note" } else { "message" },
            json!({ "seq": seq, "attachments": attachments.len(), "author_kind": if staff { "support" } else { "customer" } }),
        )?;

        if staff && !internal && case.first_response.responded_at.is_none() {
            conn.execute(
                "UPDATE support_cases SET first_response_at = ?2 WHERE id = ?1",
                params![case_id, ts(now)],
            )?;
            case_event(
                &conn,
                case_id,
                &caller.username,
                "first_response",
                json!({}),
            )?;
        }
        if !staff && case.status == CaseStatus::WaitingOnCustomer {
            self.apply_status(
                &conn,
                &case,
                CaseStatus::Investigating,
                &caller.username,
                "customer_replied",
                now,
            )?;
        }
        load_case(&conn, case_id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Change status. The support desk may make any valid transition;
    /// customers may only close a resolved case or reopen it.
    pub fn set_case_status(
        &self,
        caller: &Caller,
        case_id: &str,
        to: CaseStatus,
    ) -> Result<SupportCase> {
        let conn = self.lock()?;
        let case = self.visible_case(&conn, caller, case_id)?;
        if !caller.admin
            && !matches!(
                (case.status, to),
                (CaseStatus::Resolved, CaseStatus::Closed)
                    | (CaseStatus::Resolved, CaseStatus::Investigating)
            )
        {
            return Err(CommercialError::Forbidden(
                "customers can close a resolved case or reopen it; other changes are made by support".into(),
            ));
        }
        if !case.status.can_transition_to(to) {
            return Err(CommercialError::Conflict(format!(
                "cannot move a case from {} to {}",
                case.status.as_str(),
                to.as_str()
            )));
        }
        if to == CaseStatus::WaitingOnCustomer && case.first_response.responded_at.is_none() {
            return Err(CommercialError::Invalid(
                "post a customer-visible response before waiting on the customer".into(),
            ));
        }
        let now = Utc::now();
        let event = if matches!(case.status, CaseStatus::Resolved | CaseStatus::Closed)
            && to == CaseStatus::Investigating
        {
            "reopened"
        } else {
            "status_changed"
        };
        self.apply_status(&conn, &case, to, &caller.username, event, now)?;
        load_case(&conn, case_id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Apply a validated transition and its timer side effects.
    fn apply_status(
        &self,
        conn: &Connection,
        case: &SupportCase,
        to: CaseStatus,
        actor: &str,
        event: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let _ = self;
        let id = case.id.as_str();
        let snap = &case.coverage;
        match to {
            CaseStatus::WaitingOnCustomer => {
                // Pause the resolution estimate: remember the covered time left.
                if let (Some(due), Some(sch)) = (case.resolution.due, snap.schedule()) {
                    let remaining = sch.covered_seconds_between(now, due);
                    conn.execute(
                        "UPDATE support_cases SET resolution_paused_secs = ?2 WHERE id = ?1",
                        params![id, remaining],
                    )?;
                }
            }
            CaseStatus::Investigating => {
                if matches!(case.status, CaseStatus::Resolved | CaseStatus::Closed) {
                    // Reopen: the old estimate no longer applies.
                    conn.execute(
                        "UPDATE support_cases SET reopen_count = reopen_count + 1, resolved_at = NULL,
                            closed_at = NULL, resolution_due = NULL, resolution_paused_secs = NULL
                         WHERE id = ?1",
                        [id],
                    )?;
                } else if let Some(rem) = case.resolution.remaining_covered_seconds {
                    // Resume from waiting_on_customer.
                    let due = snap
                        .schedule()
                        .and_then(|s| s.add_covered_seconds(now, rem));
                    conn.execute(
                        "UPDATE support_cases SET resolution_due = ?2, resolution_paused_secs = NULL WHERE id = ?1",
                        params![id, due.map(ts)],
                    )?;
                }
            }
            CaseStatus::Resolved => {
                conn.execute(
                    "UPDATE support_cases SET resolved_at = ?2, resolution_paused_secs = NULL WHERE id = ?1",
                    params![id, ts(now)],
                )?;
            }
            CaseStatus::Closed => {
                conn.execute(
                    "UPDATE support_cases SET closed_at = ?2 WHERE id = ?1",
                    params![id, ts(now)],
                )?;
            }
            _ => {}
        }
        conn.execute(
            "UPDATE support_cases SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, to.as_str(), ts(now)],
        )?;
        case_event(
            conn,
            id,
            actor,
            event,
            json!({ "from": case.status.as_str(), "to": to.as_str() }),
        )
    }

    /// Set or clear the assigned owner. Support desk only.
    pub fn assign_case(
        &self,
        caller: &Caller,
        case_id: &str,
        owner: Option<&str>,
    ) -> Result<SupportCase> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the support desk assigns owners".into(),
            ));
        }
        let owner = owner.map(|o| bounded("owner", o, 128, true)).transpose()?;
        let conn = self.lock()?;
        let case = self.visible_case(&conn, caller, case_id)?;
        let now = Utc::now();
        conn.execute(
            "UPDATE support_cases SET owner = ?2, updated_at = ?3 WHERE id = ?1",
            params![case_id, owner, ts(now)],
        )?;
        case_event(
            &conn,
            case_id,
            &caller.username,
            "assigned",
            json!({ "from": case.owner, "to": owner }),
        )?;
        load_case(&conn, case_id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Record an escalation request (customer or support desk).
    pub fn escalate_case(
        &self,
        caller: &Caller,
        case_id: &str,
        reason: &str,
    ) -> Result<SupportCase> {
        let reason = bounded("reason", reason, 2000, true)?;
        let conn = self.lock()?;
        let case = self.visible_case(&conn, caller, case_id)?;
        if case.status == CaseStatus::Closed {
            return Err(CommercialError::Conflict("the case is closed".into()));
        }
        let now = Utc::now();
        conn.execute(
            "UPDATE support_cases SET updated_at = ?2 WHERE id = ?1",
            params![case_id, ts(now)],
        )?;
        case_event(
            &conn,
            case_id,
            &caller.username,
            "escalated",
            json!({ "reason": reason, "by_support": caller.admin }),
        )?;
        load_case(&conn, case_id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Attachment bytes, subject to the same visibility as the case detail.
    pub fn get_attachment(
        &self,
        caller: &Caller,
        case_id: &str,
        attachment_id: &str,
    ) -> Result<(AttachmentMeta, Vec<u8>)> {
        let conn = self.lock()?;
        self.visible_case(&conn, caller, case_id)?;
        let row = conn
            .query_row(
                "SELECT a.message_seq, a.filename, a.content_type, a.size, a.sha256, a.uploaded_by,
                        a.at, a.purged_at, a.data, COALESCE(m.internal, 0)
                 FROM support_attachments a
                 LEFT JOIN support_messages m ON m.seq = a.message_seq
                 WHERE a.id = ?1 AND a.case_id = ?2",
                params![attachment_id, case_id],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<Vec<u8>>>(8)?,
                        r.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            message_seq,
            filename,
            content_type,
            size,
            sha256,
            uploaded_by,
            at,
            purged,
            data,
            internal,
        )) = row
        else {
            return Err(CommercialError::NotFound);
        };
        if internal != 0 && !caller.admin {
            return Err(CommercialError::NotFound);
        }
        let Some(data) = data else {
            return Err(CommercialError::Conflict(
                "the attachment was purged under the retention policy".into(),
            ));
        };
        Ok((
            AttachmentMeta {
                id: attachment_id.to_string(),
                message_seq,
                filename,
                content_type,
                size,
                sha256,
                uploaded_by,
                at: parse_ts(&at)?,
                purged_at: purged.as_deref().map(parse_ts).transpose()?,
            },
            data,
        ))
    }

    /// Drop attachment bytes older than the retention window (metadata is
    /// kept so the timeline stays truthful). Returns how many were purged.
    pub fn purge_expired_attachments(&self) -> Result<usize> {
        let limits = SupportLimits::from_env();
        let conn = self.lock()?;
        let now = Utc::now();
        let cutoff = ts(now - Duration::days(limits.retention_days));
        let n = conn.execute(
            "UPDATE support_attachments SET data = NULL, purged_at = ?1
             WHERE data IS NOT NULL AND at < ?2",
            params![ts(now), cutoff],
        )?;
        Ok(n)
    }

    /// Membership check helper for handlers that need to authorize before
    /// doing expensive work.
    pub fn can_access_org(&self, caller: &Caller, org_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        Ok(caller.admin || is_member(&conn, org_id, &caller.username)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commercial::store::{QuoteInput, QuoteRequestInput};
    use std::collections::BTreeMap;

    fn admin() -> Caller {
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

    /// Org + active Supported Plus-style contract covering `cluster`, with
    /// 24x7 hours so timers are exact.
    fn covered(store: &CommercialStore, name: &str, member: &str, cluster: &str) -> String {
        let org = store.create_org(name, Some(member)).unwrap();
        let req = store
            .create_quote_request(
                &user(member),
                QuoteRequestInput {
                    org_id: Some(org.id.clone()),
                    offering: "supported_plus".into(),
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
        let c = store
            .generate_quote(
                "desk",
                &req.id,
                QuoteInput {
                    org_id: None,
                    currency: "USD".into(),
                    pricing_unit: "worker node / year".into(),
                    unit_price_minor: None,
                    included_capacity: String::new(),
                    duration_months: 12,
                    response_targets: vec!["sev2: 30 min".into()],
                    support_hours: String::new(),
                    extra_exclusions: vec![],
                    node_treatment: String::new(),
                },
            )
            .unwrap();
        let mut targets = BTreeMap::new();
        targets.insert("sev2".to_string(), 30u32);
        let mut estimates = BTreeMap::new();
        estimates.insert("sev2".to_string(), 240u32);
        store
            .update_entitlement(
                "desk",
                &c.id,
                Entitlement {
                    covered_clusters: vec![cluster.into()],
                    support_tier: "supported_plus".into(),
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
                    response_target_minutes: targets,
                    resolution_estimate_minutes: estimates,
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .transition("desk", &c.id, ContractStatus::AwaitingAcceptance)
            .unwrap();
        store.accept(&user(member), &c.id).unwrap();
        org.id
    }

    fn new_case(org: &str, cluster: &str, sev: &str) -> NewCase {
        NewCase {
            org_id: org.into(),
            cluster_id: cluster.into(),
            severity: sev.into(),
            title: "VMs will not start".into(),
            description: "Details".into(),
        }
    }

    #[test]
    fn state_machine() {
        use CaseStatus::*;
        assert!(Open.can_transition_to(Triaged));
        assert!(Investigating.can_transition_to(WaitingOnCustomer));
        assert!(Resolved.can_transition_to(Investigating));
        assert!(Closed.can_transition_to(Investigating));
        assert!(!Open.can_transition_to(Resolved));
        assert!(!Closed.can_transition_to(Resolved));
        for s in [
            Open,
            Triaged,
            Investigating,
            WaitingOnCustomer,
            Resolved,
            Closed,
        ] {
            assert_eq!(CaseStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn coverage_snapshot_sets_timers() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let c = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        assert!(c.coverage.covered);
        assert_eq!(c.coverage.support_tier, "supported_plus");
        assert_eq!(c.first_response.state, "pending");
        let secs = (c.first_response.due.unwrap() - c.created_at).num_seconds();
        assert!((1799..=1801).contains(&secs), "{secs}");
        assert!(c.resolution.due.is_some());
        // A severity with no target has no timer.
        let c3 = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev3"))
            .unwrap();
        assert_eq!(c3.first_response.state, "not_applicable");
    }

    #[test]
    fn uncovered_cluster_is_recorded_without_commitments() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let c = s
            .create_case(&user("ann"), new_case(&org, "other", "sev1"))
            .unwrap();
        assert!(!c.coverage.covered);
        assert!(c.coverage.reason.is_some());
        assert_eq!(c.first_response.state, "not_applicable");
        assert!(c.resolution.due.is_none());
    }

    #[test]
    fn organizations_are_isolated() {
        let s = CommercialStore::open(":memory:").unwrap();
        let a = covered(&s, "Acme", "ann", "cl-a");
        let _b = covered(&s, "Beta", "bob", "cl-b");
        let case = s
            .create_case(&user("ann"), new_case(&a, "cl-a", "sev2"))
            .unwrap();
        let att = vec![NewAttachment {
            filename: "x.txt".into(),
            content_type: "text/plain".into(),
            data: b"hi".to_vec(),
        }];
        s.add_message(&user("ann"), &case.id, "see file", false, false, att)
            .unwrap();
        let detail = s.case_detail(&user("ann"), &case.id).unwrap();
        let att_id = detail.attachments[0].id.clone();

        let bob = user("bob");
        assert!(matches!(
            s.get_case(&bob, &case.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.case_detail(&bob, &case.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.add_message(&bob, &case.id, "hi", false, false, vec![]),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.get_attachment(&bob, &case.id, &att_id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.escalate_case(&bob, &case.id, "x"),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.create_case(&bob, new_case(&a, "cl-a", "sev2")),
            Err(CommercialError::NotFound)
        ));
        assert!(s.list_cases(&bob).unwrap().is_empty());
        assert_eq!(s.list_cases(&user("ann")).unwrap().len(), 1);
        assert_eq!(s.list_cases(&admin()).unwrap().len(), 1);
        assert!(s.get_attachment(&user("ann"), &case.id, &att_id).is_ok());
    }

    #[test]
    fn first_response_and_internal_notes() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        // Internal note does not count as a response and is hidden.
        s.add_message(&admin(), &case.id, "looks like CDI", true, false, vec![])
            .unwrap();
        let c = s.get_case(&user("ann"), &case.id).unwrap();
        assert_eq!(c.first_response.state, "pending");
        let seen = s.case_detail(&user("ann"), &case.id).unwrap();
        assert!(seen
            .messages
            .iter()
            .all(|m| !m.internal && m.body != "looks like CDI"));
        assert_eq!(s.case_detail(&admin(), &case.id).unwrap().messages.len(), 2);
        // Customers cannot post internal notes.
        assert!(matches!(
            s.add_message(&user("ann"), &case.id, "x", true, false, vec![]),
            Err(CommercialError::Forbidden(_))
        ));
        // A public desk reply meets the target.
        let c = s
            .add_message(&admin(), &case.id, "on it", false, false, vec![])
            .unwrap();
        assert_eq!(c.first_response.state, "met");
    }

    #[test]
    fn waiting_on_customer_pauses_and_resumes() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        s.set_case_status(&admin(), &case.id, CaseStatus::Triaged)
            .unwrap();
        s.set_case_status(&admin(), &case.id, CaseStatus::Investigating)
            .unwrap();
        // Must respond before waiting on the customer.
        assert!(matches!(
            s.set_case_status(&admin(), &case.id, CaseStatus::WaitingOnCustomer),
            Err(CommercialError::Invalid(_))
        ));
        s.add_message(&admin(), &case.id, "please send logs", false, false, vec![])
            .unwrap();
        let waiting = s
            .set_case_status(&admin(), &case.id, CaseStatus::WaitingOnCustomer)
            .unwrap();
        assert!(waiting.resolution.paused);
        assert!(waiting.resolution.remaining_covered_seconds.unwrap() > 14_300);
        // Customer reply resumes automatically.
        let resumed = s
            .add_message(
                &user("ann"),
                &case.id,
                "logs attached",
                false,
                false,
                vec![],
            )
            .unwrap();
        assert_eq!(resumed.status, CaseStatus::Investigating);
        assert!(!resumed.resolution.paused);
        let drift = (resumed.resolution.due.unwrap() - case.resolution.due.unwrap())
            .num_seconds()
            .abs();
        assert!(drift < 10, "due drifted by {drift}s");
    }

    #[test]
    fn customer_powers_are_limited_and_reopen_works() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        assert!(matches!(
            s.set_case_status(&user("ann"), &case.id, CaseStatus::Resolved),
            Err(CommercialError::Forbidden(_))
        ));
        assert!(matches!(
            s.assign_case(&user("ann"), &case.id, Some("me")),
            Err(CommercialError::Forbidden(_))
        ));
        for st in [
            CaseStatus::Triaged,
            CaseStatus::Investigating,
            CaseStatus::Resolved,
        ] {
            s.set_case_status(&admin(), &case.id, st).unwrap();
        }
        let reopened = s
            .set_case_status(&user("ann"), &case.id, CaseStatus::Investigating)
            .unwrap();
        assert_eq!(reopened.reopen_count, 1);
        assert!(reopened.resolved_at.is_none());
        s.set_case_status(&admin(), &case.id, CaseStatus::Resolved)
            .unwrap();
        s.set_case_status(&user("ann"), &case.id, CaseStatus::Closed)
            .unwrap();
        // Closed: no replies, and only the desk can reopen.
        assert!(matches!(
            s.add_message(&user("ann"), &case.id, "x", false, false, vec![]),
            Err(CommercialError::Conflict(_))
        ));
        assert!(matches!(
            s.set_case_status(&user("ann"), &case.id, CaseStatus::Investigating),
            Err(CommercialError::Forbidden(_))
        ));
        s.set_case_status(&admin(), &case.id, CaseStatus::Investigating)
            .unwrap();
    }

    #[test]
    fn escalation_history_and_assignment_are_recorded() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        s.escalate_case(&user("ann"), &case.id, "production impact")
            .unwrap();
        let c = s.assign_case(&admin(), &case.id, Some("eng-lead")).unwrap();
        assert_eq!(c.owner.as_deref(), Some("eng-lead"));
        let d = s.case_detail(&user("ann"), &case.id).unwrap();
        assert_eq!(d.escalations.len(), 1);
        assert!(d.events.iter().any(|e| e.event == "assigned"));
    }

    #[test]
    fn attachment_limits_sanitizing_and_retention() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&user("ann"), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        let big = vec![NewAttachment {
            filename: "big".into(),
            content_type: "x".into(),
            data: vec![0u8; SupportLimits::from_env().attachment_max_bytes + 1],
        }];
        assert!(matches!(
            s.add_message(&user("ann"), &case.id, "x", false, false, big),
            Err(CommercialError::Invalid(_))
        ));
        let evil = vec![NewAttachment {
            filename: "../../etc/passwd".into(),
            content_type: "text/plain".into(),
            data: b"data".to_vec(),
        }];
        s.add_message(&user("ann"), &case.id, "x", false, false, evil)
            .unwrap();
        let d = s.case_detail(&user("ann"), &case.id).unwrap();
        assert_eq!(d.attachments[0].filename, "passwd");
        assert_eq!(d.attachments[0].sha256.len(), 64);
        // Age the attachment past retention, then purge.
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                "UPDATE support_attachments SET at = '2000-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        }
        assert_eq!(s.purge_expired_attachments().unwrap(), 1);
        assert!(matches!(
            s.get_attachment(&user("ann"), &case.id, &d.attachments[0].id),
            Err(CommercialError::Conflict(_))
        ));
        let after = s.case_detail(&user("ann"), &case.id).unwrap();
        assert!(after.attachments[0].purged_at.is_some());
    }

    #[test]
    fn diagnostics_upload_as_customer_does_not_count_as_response() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = covered(&s, "Acme", "ann", "cl-1");
        let case = s
            .create_case(&admin(), new_case(&org, "cl-1", "sev2"))
            .unwrap();
        let c = s
            .add_message(&admin(), &case.id, "bundle", false, true, vec![])
            .unwrap();
        assert_eq!(c.first_response.state, "pending");
        let d = s.case_detail(&admin(), &case.id).unwrap();
        assert_eq!(d.messages.last().unwrap().author_kind, "customer");
    }
}
