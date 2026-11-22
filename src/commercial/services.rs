// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Service engagements: purchased deployment, migration and training
//! projects tracked as milestones with owners, evidence and customer
//! acceptance.
//!
//! This tracks delivery of a purchased service. It does not execute
//! anything: migrations remain governed by their own (experimental) APIs.

use super::store::{
    bounded, new_id, org_exists, parse_ts, ts, Caller, CommercialError, CommercialStore, Result,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const EXECUTION_NOTE: &str = "Tracks delivery of a purchased service. It does not run migrations or change clusters; experimental migration APIs stay experimental until verified.";

const DEPLOYMENT: &[(&str, &str)] = &[
    ("assessment", "Environment assessment"),
    ("install", "Installation and integration"),
    ("testing", "Functional and recovery testing"),
    ("handover", "Administrator handover"),
    ("acceptance", "Acceptance"),
];
const MIGRATION: &[(&str, &str)] = &[
    ("inventory", "Source inventory and assessment"),
    ("mapping", "Network and storage mapping"),
    ("pilot", "Pilot migration"),
    ("waves", "Approved migration waves"),
    ("validation", "Guest and application validation"),
    ("handover", "Handover"),
];
const TRAINING: &[(&str, &str)] = &[
    ("scheduling", "Scheduling"),
    ("delivery", "Workshop delivery"),
    ("followup", "Runbooks and follow-up"),
];

/// The milestone whose completion is the customer's acceptance.
const ACCEPTANCE_KEY: &str = "acceptance";

const MILESTONE_STATUSES: &[&str] = &["pending", "in_progress", "done", "blocked"];
const EVIDENCE_KINDS: &[&str] = &["note", "link", "document_ref", "test_result"];

fn template(kind: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match kind {
        "deployment" => Some(DEPLOYMENT),
        "migration" => Some(MIGRATION),
        "training" => Some(TRAINING),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub id: String,
    pub milestone_key: String,
    pub kind: String,
    pub title: String,
    pub reference: Option<String>,
    pub note: Option<String>,
    pub added_by: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Milestone {
    pub key: String,
    pub title: String,
    pub position: i64,
    pub status: String,
    pub owner: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Engagement {
    pub id: String,
    pub org_id: String,
    pub contract_id: Option<String>,
    pub kind: String,
    pub title: String,
    /// `planned`, `in_progress`, `awaiting_acceptance`, `accepted` or `cancelled`.
    pub status: String,
    pub owner: Option<String>,
    pub created_by: String,
    pub accepted_by: Option<String>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub milestones: Vec<Milestone>,
    pub execution_note: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEngagement {
    pub org_id: String,
    /// `deployment`, `migration` or `training`.
    pub kind: String,
    pub title: String,
    pub contract_id: Option<String>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MilestoneUpdate {
    pub status: Option<String>,
    pub owner: Option<String>,
    /// RFC 3339.
    pub due_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEvidence {
    pub kind: String,
    pub title: String,
    pub reference: Option<String>,
    pub note: Option<String>,
}

pub(super) fn service_event(
    conn: &Connection,
    subject: &str,
    subject_id: &str,
    actor: &str,
    event: &str,
    detail: serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO service_events (subject, subject_id, at, actor, event, detail)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            subject,
            subject_id,
            ts(Utc::now()),
            actor,
            event,
            detail.to_string()
        ],
    )?;
    Ok(())
}

fn load_engagement(conn: &Connection, id: &str) -> Result<Option<Engagement>> {
    let row = conn
        .query_row(
            "SELECT org_id, contract_id, kind, title, status, owner, created_by, accepted_by,
                    accepted_at, created_at, updated_at
             FROM engagements WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                ))
            },
        )
        .optional()?;
    let Some((
        org_id,
        contract_id,
        kind,
        title,
        status,
        owner,
        created_by,
        accepted_by,
        accepted_at,
        created,
        updated,
    )) = row
    else {
        return Ok(None);
    };

    let mut stmt = conn.prepare(
        "SELECT id, milestone_key, kind, title, reference, note, added_by, at
         FROM engagement_evidence WHERE engagement_id = ?1 ORDER BY at, id",
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
            r.get::<_, String>(7)?,
        ))
    })?;
    let mut evidence = Vec::new();
    for row in rows {
        let (eid, milestone_key, kind, title, reference, note, added_by, at) = row?;
        evidence.push(Evidence {
            id: eid,
            milestone_key,
            kind,
            title,
            reference,
            note,
            added_by,
            at: parse_ts(&at)?,
        });
    }
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT key, title, position, status, owner, due_at, completed_at
         FROM engagement_milestones WHERE engagement_id = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut milestones = Vec::new();
    for row in rows {
        let (key, mtitle, position, mstatus, mowner, due, completed) = row?;
        milestones.push(Milestone {
            evidence: evidence
                .iter()
                .filter(|e| e.milestone_key == key)
                .cloned()
                .collect(),
            key,
            title: mtitle,
            position,
            status: mstatus,
            owner: mowner,
            due_at: due.as_deref().map(parse_ts).transpose()?,
            completed_at: completed.as_deref().map(parse_ts).transpose()?,
        });
    }
    drop(stmt);

    Ok(Some(Engagement {
        id: id.to_string(),
        org_id,
        contract_id,
        kind,
        title,
        status,
        owner,
        created_by,
        accepted_by,
        accepted_at: accepted_at.as_deref().map(parse_ts).transpose()?,
        created_at: parse_ts(&created)?,
        updated_at: parse_ts(&updated)?,
        milestones,
        execution_note: EXECUTION_NOTE,
    }))
}

fn require_open(e: &Engagement) -> Result<()> {
    if matches!(e.status.as_str(), "accepted" | "cancelled") {
        return Err(CommercialError::Conflict(format!(
            "the engagement is {}",
            e.status
        )));
    }
    Ok(())
}

impl CommercialStore {
    pub fn create_engagement(&self, caller: &Caller, input: NewEngagement) -> Result<Engagement> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "engagements are created by the service desk".into(),
            ));
        }
        let tpl = template(&input.kind).ok_or_else(|| {
            CommercialError::Invalid("kind must be deployment, migration or training".into())
        })?;
        let title = bounded("title", &input.title, 200, true)?;
        let owner = input
            .owner
            .as_deref()
            .map(|o| bounded("owner", o, 128, true))
            .transpose()?;
        let conn = self.lock()?;
        org_exists(&conn, &input.org_id)?;
        if let Some(cid) = &input.contract_id {
            let (org, offering): (String, String) = conn
                .query_row(
                    "SELECT org_id, offering FROM contracts WHERE id = ?1",
                    [cid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or(CommercialError::NotFound)?;
            if org != input.org_id || offering != input.kind {
                return Err(CommercialError::Invalid(
                    "the contract must belong to the same organization and be a matching service"
                        .into(),
                ));
            }
        }
        let now = Utc::now();
        let id = new_id("eng");
        conn.execute(
            "INSERT INTO engagements (id, org_id, contract_id, kind, title, status, owner,
                created_by, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,'planned',?6,?7,?8,?8)",
            params![
                id,
                input.org_id,
                input.contract_id,
                input.kind,
                title,
                owner,
                caller.username,
                ts(now)
            ],
        )?;
        for (i, (key, mtitle)) in tpl.iter().enumerate() {
            conn.execute(
                "INSERT INTO engagement_milestones (engagement_id, key, position, title)
                 VALUES (?1,?2,?3,?4)",
                params![id, key, i as i64 + 1, mtitle],
            )?;
        }
        service_event(
            &conn,
            "engagement",
            &id,
            &caller.username,
            "created",
            json!({ "kind": input.kind }),
        )?;
        load_engagement(&conn, &id)?.ok_or(CommercialError::NotFound)
    }

    pub fn list_engagements(&self, caller: &Caller) -> Result<Vec<Engagement>> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT id, org_id FROM engagements ORDER BY created_at DESC")?;
        let ids: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let mut out = Vec::new();
        for (id, org) in ids {
            if self.allowed(&conn, caller, &org)? {
                if let Some(e) = load_engagement(&conn, &id)? {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    fn visible_engagement(
        &self,
        conn: &Connection,
        caller: &Caller,
        id: &str,
    ) -> Result<Engagement> {
        let e = load_engagement(conn, id)?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(conn, caller, &e.org_id)? {
            return Err(CommercialError::NotFound);
        }
        Ok(e)
    }

    pub fn get_engagement(&self, caller: &Caller, id: &str) -> Result<Engagement> {
        let conn = self.lock()?;
        self.visible_engagement(&conn, caller, id)
    }

    pub fn engagement_events(&self, caller: &Caller, id: &str) -> Result<Vec<serde_json::Value>> {
        let conn = self.lock()?;
        self.visible_engagement(&conn, caller, id)?;
        subject_events(&conn, "engagement", id)
    }

    /// Service desk: update a milestone. `done` needs at least one piece of
    /// evidence; the acceptance milestone is completed only by the customer.
    pub fn update_milestone(
        &self,
        caller: &Caller,
        engagement_id: &str,
        key: &str,
        update: MilestoneUpdate,
    ) -> Result<Engagement> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "milestones are updated by the service desk".into(),
            ));
        }
        let conn = self.lock()?;
        let e = self.visible_engagement(&conn, caller, engagement_id)?;
        require_open(&e)?;
        let m = e
            .milestones
            .iter()
            .find(|m| m.key == key)
            .ok_or(CommercialError::NotFound)?;
        let now = Utc::now();
        if let Some(status) = &update.status {
            if !MILESTONE_STATUSES.contains(&status.as_str()) {
                return Err(CommercialError::Invalid(
                    "status must be pending, in_progress, done or blocked".into(),
                ));
            }
            if status == "done" {
                if key == ACCEPTANCE_KEY && e.kind == "deployment" {
                    return Err(CommercialError::Invalid(
                        "the acceptance milestone is completed by the customer accepting the engagement".into(),
                    ));
                }
                if m.evidence.is_empty() {
                    return Err(CommercialError::Invalid(
                        "add evidence to a milestone before marking it done".into(),
                    ));
                }
            }
            conn.execute(
                "UPDATE engagement_milestones SET status = ?3, completed_at = ?4
                 WHERE engagement_id = ?1 AND key = ?2",
                params![
                    engagement_id,
                    key,
                    status,
                    (status == "done").then(|| ts(now))
                ],
            )?;
            if e.status == "planned" && status != "pending" {
                conn.execute(
                    "UPDATE engagements SET status = 'in_progress' WHERE id = ?1",
                    [engagement_id],
                )?;
            }
            if e.status == "awaiting_acceptance" && status != "done" {
                conn.execute(
                    "UPDATE engagements SET status = 'in_progress' WHERE id = ?1",
                    [engagement_id],
                )?;
            }
        }
        if let Some(owner) = &update.owner {
            let o = bounded("owner", owner, 128, false)?;
            conn.execute(
                "UPDATE engagement_milestones SET owner = ?3 WHERE engagement_id = ?1 AND key = ?2",
                params![
                    engagement_id,
                    key,
                    if o.is_empty() { None } else { Some(o) }
                ],
            )?;
        }
        if let Some(due) = &update.due_at {
            let d = parse_ts(due)
                .map_err(|_| CommercialError::Invalid("due_at must be RFC 3339".into()))?;
            conn.execute(
                "UPDATE engagement_milestones SET due_at = ?3 WHERE engagement_id = ?1 AND key = ?2",
                params![engagement_id, key, ts(d)],
            )?;
        }
        conn.execute(
            "UPDATE engagements SET updated_at = ?2 WHERE id = ?1",
            params![engagement_id, ts(now)],
        )?;
        service_event(
            &conn,
            "engagement",
            engagement_id,
            &caller.username,
            "milestone_updated",
            json!({ "milestone": key, "status": update.status, "owner": update.owner, "due_at": update.due_at }),
        )?;
        load_engagement(&conn, engagement_id)?.ok_or(CommercialError::NotFound)
    }

    /// Attach evidence to a milestone. The service desk and the customer's
    /// members may both add it (for example a customer sign-off).
    pub fn add_engagement_evidence(
        &self,
        caller: &Caller,
        engagement_id: &str,
        key: &str,
        input: NewEvidence,
    ) -> Result<Engagement> {
        if !EVIDENCE_KINDS.contains(&input.kind.as_str()) {
            return Err(CommercialError::Invalid(
                "kind must be note, link, document_ref or test_result".into(),
            ));
        }
        let title = bounded("title", &input.title, 200, true)?;
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
        let conn = self.lock()?;
        let e = self.visible_engagement(&conn, caller, engagement_id)?;
        require_open(&e)?;
        if !e.milestones.iter().any(|m| m.key == key) {
            return Err(CommercialError::NotFound);
        }
        conn.execute(
            "INSERT INTO engagement_evidence (id, engagement_id, milestone_key, kind, title,
                reference, note, added_by, at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                new_id("ev"),
                engagement_id,
                key,
                input.kind,
                title,
                reference,
                note,
                caller.username,
                ts(Utc::now())
            ],
        )?;
        service_event(
            &conn,
            "engagement",
            engagement_id,
            &caller.username,
            "evidence_added",
            json!({ "milestone": key, "kind": input.kind, "title": title }),
        )?;
        load_engagement(&conn, engagement_id)?.ok_or(CommercialError::NotFound)
    }

    /// Service desk: ask the customer to accept. Every milestone except the
    /// customer-completed acceptance one must be done.
    pub fn request_engagement_acceptance(
        &self,
        caller: &Caller,
        engagement_id: &str,
    ) -> Result<Engagement> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the service desk requests acceptance".into(),
            ));
        }
        let conn = self.lock()?;
        let e = self.visible_engagement(&conn, caller, engagement_id)?;
        require_open(&e)?;
        if e.status == "awaiting_acceptance" {
            return Err(CommercialError::Conflict(
                "acceptance has already been requested".into(),
            ));
        }
        let pending: Vec<&str> = e
            .milestones
            .iter()
            .filter(|m| !(m.key == ACCEPTANCE_KEY && e.kind == "deployment") && m.status != "done")
            .map(|m| m.title.as_str())
            .collect();
        if !pending.is_empty() {
            return Err(CommercialError::Conflict(format!(
                "milestones not done: {}",
                pending.join(", ")
            )));
        }
        conn.execute(
            "UPDATE engagements SET status = 'awaiting_acceptance', updated_at = ?2 WHERE id = ?1",
            params![engagement_id, ts(Utc::now())],
        )?;
        service_event(
            &conn,
            "engagement",
            engagement_id,
            &caller.username,
            "acceptance_requested",
            json!({}),
        )?;
        load_engagement(&conn, engagement_id)?.ok_or(CommercialError::NotFound)
    }

    /// Customer acceptance. Only after the desk has requested it.
    pub fn accept_engagement(&self, caller: &Caller, engagement_id: &str) -> Result<Engagement> {
        let conn = self.lock()?;
        let e = self.visible_engagement(&conn, caller, engagement_id)?;
        if e.status != "awaiting_acceptance" {
            return Err(CommercialError::Conflict(
                "the engagement is not awaiting acceptance".into(),
            ));
        }
        let now = Utc::now();
        conn.execute(
            "UPDATE engagements SET status = 'accepted', accepted_by = ?2, accepted_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![engagement_id, caller.username, ts(now)],
        )?;
        conn.execute(
            "UPDATE engagement_milestones SET status = 'done', completed_at = ?3
             WHERE engagement_id = ?1 AND key = ?2",
            params![engagement_id, ACCEPTANCE_KEY, ts(now)],
        )?;
        service_event(
            &conn,
            "engagement",
            engagement_id,
            &caller.username,
            "accepted",
            json!({}),
        )?;
        load_engagement(&conn, engagement_id)?.ok_or(CommercialError::NotFound)
    }

    pub fn cancel_engagement(
        &self,
        caller: &Caller,
        engagement_id: &str,
        reason: &str,
    ) -> Result<Engagement> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the service desk cancels engagements".into(),
            ));
        }
        let reason = bounded("reason", reason, 2000, true)?;
        let conn = self.lock()?;
        let e = self.visible_engagement(&conn, caller, engagement_id)?;
        require_open(&e)?;
        conn.execute(
            "UPDATE engagements SET status = 'cancelled', updated_at = ?2 WHERE id = ?1",
            params![engagement_id, ts(Utc::now())],
        )?;
        service_event(
            &conn,
            "engagement",
            engagement_id,
            &caller.username,
            "cancelled",
            json!({ "reason": reason }),
        )?;
        load_engagement(&conn, engagement_id)?.ok_or(CommercialError::NotFound)
    }
}

pub(super) fn subject_events(
    conn: &Connection,
    subject: &str,
    id: &str,
) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT at, actor, event, detail FROM service_events
         WHERE subject = ?1 AND subject_id = ?2 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![subject, id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (at, actor, event, detail) = row?;
        out.push(json!({
            "at": at,
            "actor": actor,
            "event": event,
            "detail": serde_json::from_str::<serde_json::Value>(&detail).unwrap_or(serde_json::Value::Null),
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn engagement(s: &CommercialStore, kind: &str) -> (String, Engagement) {
        let org = s.create_org("Acme", Some("ann")).unwrap();
        let e = s
            .create_engagement(
                &desk(),
                NewEngagement {
                    org_id: org.id.clone(),
                    kind: kind.into(),
                    title: "Rollout".into(),
                    contract_id: None,
                    owner: Some("lead".into()),
                },
            )
            .unwrap();
        (org.id, e)
    }

    fn ev() -> NewEvidence {
        NewEvidence {
            kind: "note".into(),
            title: "Signed off".into(),
            reference: None,
            note: Some("all good".into()),
        }
    }

    fn mark(s: &CommercialStore, id: &str, key: &str, status: &str) -> Result<Engagement> {
        s.update_milestone(
            &desk(),
            id,
            key,
            MilestoneUpdate {
                status: Some(status.into()),
                owner: None,
                due_at: None,
            },
        )
    }

    #[test]
    fn templates_match_the_spec() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, d) = engagement(&s, "deployment");
        let keys: Vec<_> = d.milestones.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["assessment", "install", "testing", "handover", "acceptance"]
        );
        let m_org = s.create_org("Beta", None).unwrap().id;
        let m = s
            .create_engagement(
                &desk(),
                NewEngagement {
                    org_id: m_org,
                    kind: "migration".into(),
                    title: "Move".into(),
                    contract_id: None,
                    owner: None,
                },
            )
            .unwrap();
        assert_eq!(m.milestones.len(), 6);
        assert!(m.execution_note.contains("experimental"));
    }

    #[test]
    fn only_the_desk_creates_and_updates() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (org, e) = engagement(&s, "deployment");
        assert!(matches!(
            s.create_engagement(
                &user("ann"),
                NewEngagement {
                    org_id: org,
                    kind: "deployment".into(),
                    title: "x".into(),
                    contract_id: None,
                    owner: None
                }
            ),
            Err(CommercialError::Forbidden(_))
        ));
        assert!(matches!(
            mark_as(&s, &user("ann"), &e.id, "assessment", "in_progress"),
            Err(CommercialError::Forbidden(_))
        ));
    }

    fn mark_as(
        s: &CommercialStore,
        c: &Caller,
        id: &str,
        key: &str,
        status: &str,
    ) -> Result<Engagement> {
        s.update_milestone(
            c,
            id,
            key,
            MilestoneUpdate {
                status: Some(status.into()),
                owner: None,
                due_at: None,
            },
        )
    }

    #[test]
    fn done_requires_evidence_and_full_flow_reaches_acceptance() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, e) = engagement(&s, "deployment");
        assert!(matches!(
            mark(&s, &e.id, "assessment", "done"),
            Err(CommercialError::Invalid(_))
        ));
        for key in ["assessment", "install", "testing", "handover"] {
            s.add_engagement_evidence(&desk(), &e.id, key, ev())
                .unwrap();
            mark(&s, &e.id, key, "done").unwrap();
        }
        // The desk can't complete acceptance itself.
        s.add_engagement_evidence(&desk(), &e.id, "acceptance", ev())
            .unwrap();
        assert!(matches!(
            mark(&s, &e.id, "acceptance", "done"),
            Err(CommercialError::Invalid(_))
        ));
        // Customer cannot accept before it is requested.
        assert!(matches!(
            s.accept_engagement(&user("ann"), &e.id),
            Err(CommercialError::Conflict(_))
        ));
        let e2 = s.request_engagement_acceptance(&desk(), &e.id).unwrap();
        assert_eq!(e2.status, "awaiting_acceptance");
        let done = s.accept_engagement(&user("ann"), &e.id).unwrap();
        assert_eq!(done.status, "accepted");
        assert_eq!(done.accepted_by.as_deref(), Some("ann"));
        assert_eq!(done.milestones.last().unwrap().status, "done");
        // Closed for changes.
        assert!(matches!(
            mark(&s, &e.id, "install", "blocked"),
            Err(CommercialError::Conflict(_))
        ));
    }

    #[test]
    fn acceptance_blocked_while_milestones_are_open() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, e) = engagement(&s, "deployment");
        assert!(matches!(
            s.request_engagement_acceptance(&desk(), &e.id),
            Err(CommercialError::Conflict(_))
        ));
        assert_eq!(s.get_engagement(&desk(), &e.id).unwrap().status, "planned");
    }

    #[test]
    fn organizations_are_isolated() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, e) = engagement(&s, "deployment");
        s.create_org("Beta", Some("bob")).unwrap();
        let bob = user("bob");
        assert!(matches!(
            s.get_engagement(&bob, &e.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.add_engagement_evidence(&bob, &e.id, "assessment", ev()),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.accept_engagement(&bob, &e.id),
            Err(CommercialError::NotFound)
        ));
        assert!(s.list_engagements(&bob).unwrap().is_empty());
        assert_eq!(s.list_engagements(&user("ann")).unwrap().len(), 1);
    }

    #[test]
    fn evidence_and_due_dates_are_validated() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, e) = engagement(&s, "training");
        let empty = NewEvidence {
            kind: "note".into(),
            title: "t".into(),
            reference: None,
            note: None,
        };
        assert!(s
            .add_engagement_evidence(&desk(), &e.id, "scheduling", empty)
            .is_err());
        assert!(matches!(
            s.add_engagement_evidence(&desk(), &e.id, "nope", ev()),
            Err(CommercialError::NotFound)
        ));
        let bad = MilestoneUpdate {
            status: None,
            owner: None,
            due_at: Some("tomorrow".into()),
        };
        assert!(s
            .update_milestone(&desk(), &e.id, "scheduling", bad)
            .is_err());
        let ok = MilestoneUpdate {
            status: None,
            owner: Some("trainer".into()),
            due_at: Some("2026-11-01T09:00:00Z".into()),
        };
        let e2 = s
            .update_milestone(&desk(), &e.id, "scheduling", ok)
            .unwrap();
        assert_eq!(e2.milestones[0].owner.as_deref(), Some("trainer"));
        assert!(e2.milestones[0].due_at.is_some());
    }

    #[test]
    fn cancel_records_reason() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, e) = engagement(&s, "deployment");
        assert!(matches!(
            s.cancel_engagement(&user("ann"), &e.id, "x"),
            Err(CommercialError::Forbidden(_))
        ));
        let c = s
            .cancel_engagement(&desk(), &e.id, "customer paused")
            .unwrap();
        assert_eq!(c.status, "cancelled");
        let events = s.engagement_events(&user("ann"), &e.id).unwrap();
        assert!(events.iter().any(|v| v["event"] == "cancelled"));
    }
}
