// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Capacity observations, reconciliation and invoice records.
//!
//! Billing unit: a registered Kubernetes worker node within a covered
//! cluster (docs/BILLING_UNITS.md). Two invariants drive this file:
//!
//! * A missing, stale or failed observation is *unknown*, never zero. The
//!   reconciliation refuses to produce a billable total while any covered
//!   cluster is unknown.
//! * Money is integer minor units, and marking an invoice paid is idempotent
//!   on a payment reference, so a repeated payment cannot be recorded twice.
//!
//! No payment provider is integrated: nothing is charged and no card data
//! is handled.

use super::model::PaymentStatus;
use super::store::{
    bounded, load_contract, new_id, parse_ts, record, ts, Caller, CommercialError, CommercialStore,
    Result,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write as _;

const OBSERVATION_SOURCES: &[&str] = &["collector", "manual", "offline_import"];

/// Observations older than this (relative to the reference time) are stale.
/// `ZORVIA_CAPACITY_STALE_DAYS`, default 7.
pub fn stale_after_days() -> i64 {
    std::env::var("ZORVIA_CAPACITY_STALE_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(7)
        .clamp(1, 365)
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewObservation {
    pub cluster_id: String,
    /// `collector`, `manual` or `offline_import`.
    pub source: String,
    /// `ok` (counts required) or `error` (counts absent, `error` explains).
    pub status: String,
    pub total_nodes: Option<u32>,
    pub control_plane_nodes: Option<u32>,
    pub worker_nodes: Option<u32>,
    pub error: Option<String>,
    /// RFC 3339; defaults to now.
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub id: String,
    pub cluster_id: String,
    pub observed_at: DateTime<Utc>,
    pub source: String,
    pub status: String,
    pub total_nodes: Option<u32>,
    pub control_plane_nodes: Option<u32>,
    pub worker_nodes: Option<u32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterCapacity {
    pub cluster_id: String,
    /// `ok`, `stale`, `error` or `missing`.
    pub state: &'static str,
    /// Set only when `state == "ok"`: the count usable for billing.
    pub worker_nodes: Option<u32>,
    pub control_plane_nodes: Option<u32>,
    /// For `stale`, the last value seen, for information only.
    pub last_known_worker_nodes: Option<u32>,
    pub observed_at: Option<DateTime<Utc>>,
    pub source: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reconciliation {
    pub contract_id: String,
    pub reference_time: DateTime<Utc>,
    pub stale_after_days: i64,
    pub clusters: Vec<ClusterCapacity>,
    /// Sum of worker nodes across covered clusters, present only when every
    /// covered cluster has a fresh observation. Never a partial sum.
    pub billable_worker_nodes: Option<u32>,
    pub complete: bool,
    pub unknown_clusters: Vec<String>,
    pub node_allowance: Option<u32>,
    pub over_allowance: Option<bool>,
    /// How control-plane, infra-only, temporary and disconnected nodes are
    /// treated, as stated in the contract.
    pub node_treatment: String,
    pub note: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewLine {
    pub description: String,
    pub quantity: i64,
    pub unit_price_minor: i64,
    pub cluster_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewInvoice {
    pub contract_id: String,
    pub period_start: String,
    pub period_end: String,
    /// Invoice number in the external billing system, if any. Unique per org.
    pub external_reference: Option<String>,
    pub note: Option<String>,
    pub lines: Vec<NewLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvoiceLine {
    pub position: i64,
    pub description: String,
    pub quantity: i64,
    pub unit_price_minor: i64,
    pub amount_minor: i64,
    pub cluster_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Invoice {
    pub id: String,
    pub org_id: String,
    pub contract_id: String,
    pub currency: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    /// `draft`, `issued`, `paid` or `void`. Drafts are invisible to customers.
    pub status: String,
    pub total_minor: i64,
    pub external_reference: Option<String>,
    pub payment_reference: Option<String>,
    pub note: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub issued_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub void_reason: Option<String>,
    pub lines: Vec<InvoiceLine>,
}

fn opt(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.as_deref().map(parse_ts).transpose().map_err(Into::into)
}

fn load_invoice(conn: &Connection, id: &str) -> Result<Option<Invoice>> {
    struct Raw {
        org_id: String,
        contract_id: String,
        currency: String,
        period_start: String,
        period_end: String,
        status: String,
        total: i64,
        ext: Option<String>,
        pay: Option<String>,
        note: Option<String>,
        created_by: String,
        created_at: String,
        issued: Option<String>,
        paid: Option<String>,
        voided: Option<String>,
        void_reason: Option<String>,
    }
    let raw = conn
        .query_row(
            "SELECT org_id, contract_id, currency, period_start, period_end, status, total_minor,
                    external_reference, payment_reference, note, created_by, created_at,
                    issued_at, paid_at, voided_at, void_reason
             FROM invoices WHERE id = ?1",
            [id],
            |r| {
                Ok(Raw {
                    org_id: r.get(0)?,
                    contract_id: r.get(1)?,
                    currency: r.get(2)?,
                    period_start: r.get(3)?,
                    period_end: r.get(4)?,
                    status: r.get(5)?,
                    total: r.get(6)?,
                    ext: r.get(7)?,
                    pay: r.get(8)?,
                    note: r.get(9)?,
                    created_by: r.get(10)?,
                    created_at: r.get(11)?,
                    issued: r.get(12)?,
                    paid: r.get(13)?,
                    voided: r.get(14)?,
                    void_reason: r.get(15)?,
                })
            },
        )
        .optional()?;
    let Some(raw) = raw else { return Ok(None) };
    let mut stmt = conn.prepare(
        "SELECT position, description, quantity, unit_price_minor, amount_minor, cluster_id
         FROM invoice_lines WHERE invoice_id = ?1 ORDER BY position",
    )?;
    let lines = stmt
        .query_map([id], |r| {
            Ok(InvoiceLine {
                position: r.get(0)?,
                description: r.get(1)?,
                quantity: r.get(2)?,
                unit_price_minor: r.get(3)?,
                amount_minor: r.get(4)?,
                cluster_id: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Some(Invoice {
        id: id.to_string(),
        org_id: raw.org_id,
        contract_id: raw.contract_id,
        currency: raw.currency,
        period_start: parse_ts(&raw.period_start)?,
        period_end: parse_ts(&raw.period_end)?,
        status: raw.status,
        total_minor: raw.total,
        external_reference: raw.ext,
        payment_reference: raw.pay,
        note: raw.note,
        created_by: raw.created_by,
        created_at: parse_ts(&raw.created_at)?,
        issued_at: opt(raw.issued)?,
        paid_at: opt(raw.paid)?,
        voided_at: opt(raw.voided)?,
        void_reason: raw.void_reason,
        lines,
    }))
}

/// Keep the contract's payment status in step with its invoices, without
/// overriding statuses an administrator set by hand (`overdue`, `waived`).
fn sync_contract_payment(conn: &Connection, contract_id: &str, actor: &str) -> Result<()> {
    let current: String = conn
        .query_row(
            "SELECT payment_status FROM contracts WHERE id = ?1",
            [contract_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(CommercialError::NotFound)?;
    if !matches!(current.as_str(), "not_invoiced" | "invoiced" | "paid") {
        return Ok(());
    }
    let count = |status: &str| -> rusqlite::Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM invoices WHERE contract_id = ?1 AND status = ?2",
            params![contract_id, status],
            |r| r.get(0),
        )
    };
    let next = if count("issued")? > 0 {
        PaymentStatus::Invoiced
    } else if count("paid")? > 0 {
        PaymentStatus::Paid
    } else {
        PaymentStatus::NotInvoiced
    };
    if next.as_str() != current {
        conn.execute(
            "UPDATE contracts SET payment_status = ?2, updated_at = ?3 WHERE id = ?1",
            params![contract_id, next.as_str(), ts(Utc::now())],
        )?;
        record(
            conn,
            contract_id,
            actor,
            "payment_status_changed",
            json!({ "from": current, "to": next.as_str(), "reason": "invoice" }),
        )?;
    }
    Ok(())
}

fn money(minor: i64, currency: &str) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:02} {currency}", abs / 100, abs % 100)
}

impl CommercialStore {
    // ── capacity ────────────────────────────────────────────────

    pub fn record_observation(
        &self,
        caller: &Caller,
        input: NewObservation,
    ) -> Result<Observation> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "capacity observations are recorded by the service desk or collector".into(),
            ));
        }
        if !OBSERVATION_SOURCES.contains(&input.source.as_str()) {
            return Err(CommercialError::Invalid(
                "source must be collector, manual or offline_import".into(),
            ));
        }
        let cluster_id = bounded("cluster_id", &input.cluster_id, 128, true)?;
        let observed_at = match &input.observed_at {
            Some(s) => parse_ts(s)
                .map_err(|_| CommercialError::Invalid("observed_at must be RFC 3339".into()))?,
            None => Utc::now(),
        };
        if observed_at > Utc::now() + Duration::minutes(5) {
            return Err(CommercialError::Invalid(
                "observed_at cannot be in the future".into(),
            ));
        }
        let (total, cp, workers, error) = match input.status.as_str() {
            "ok" => {
                let (Some(total), Some(workers)) = (input.total_nodes, input.worker_nodes) else {
                    return Err(CommercialError::Invalid(
                        "an ok observation needs total_nodes and worker_nodes; use status \"error\" when counts are unknown".into(),
                    ));
                };
                let cp = input.control_plane_nodes.unwrap_or(0);
                if total == 0 {
                    return Err(CommercialError::Invalid(
                        "zero nodes is not a usable observation; record it as an error (unknown)"
                            .into(),
                    ));
                }
                if u64::from(cp) + u64::from(workers) != u64::from(total) {
                    return Err(CommercialError::Invalid(
                        "control_plane_nodes + worker_nodes must equal total_nodes".into(),
                    ));
                }
                (Some(total), Some(cp), Some(workers), None)
            }
            "error" => {
                if input.total_nodes.is_some()
                    || input.worker_nodes.is_some()
                    || input.control_plane_nodes.is_some()
                {
                    return Err(CommercialError::Invalid(
                        "an error observation carries no counts".into(),
                    ));
                }
                (
                    None,
                    None,
                    None,
                    Some(bounded(
                        "error",
                        input.error.as_deref().unwrap_or(""),
                        500,
                        true,
                    )?),
                )
            }
            _ => {
                return Err(CommercialError::Invalid(
                    "status must be ok or error".into(),
                ))
            }
        };
        let conn = self.lock()?;
        let id = new_id("obs");
        conn.execute(
            "INSERT INTO capacity_observations (id, cluster_id, observed_at, source, status, total_nodes,
                control_plane_nodes, worker_nodes, error, recorded_by, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![id, cluster_id, ts(observed_at), input.source, input.status, total, cp, workers, error, caller.username, ts(Utc::now())],
        )?;
        Ok(Observation {
            id,
            cluster_id,
            observed_at,
            source: input.source,
            status: input.status,
            total_nodes: total,
            control_plane_nodes: cp,
            worker_nodes: workers,
            error,
        })
    }

    /// Raw observations, desk only.
    pub fn list_observations(
        &self,
        caller: &Caller,
        cluster_id: Option<&str>,
    ) -> Result<Vec<Observation>> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "raw observations are visible to the service desk".into(),
            ));
        }
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, cluster_id, observed_at, source, status, total_nodes, control_plane_nodes,
                    worker_nodes, error FROM capacity_observations
             WHERE (?1 IS NULL OR cluster_id = ?1) ORDER BY observed_at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([cluster_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<u32>>(5)?,
                r.get::<_, Option<u32>>(6)?,
                r.get::<_, Option<u32>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                cluster_id,
                at,
                source,
                status,
                total_nodes,
                control_plane_nodes,
                worker_nodes,
                error,
            ) = row?;
            out.push(Observation {
                id,
                cluster_id,
                observed_at: parse_ts(&at)?,
                source,
                status,
                total_nodes,
                control_plane_nodes,
                worker_nodes,
                error,
            });
        }
        Ok(out)
    }

    /// Reconcile a contract's covered clusters against observations as of
    /// `as_of` (default now). Visible to the contract's organization.
    pub fn reconcile_capacity(
        &self,
        caller: &Caller,
        contract_id: &str,
        as_of: Option<DateTime<Utc>>,
    ) -> Result<Reconciliation> {
        let contract = self.get_contract(caller, contract_id)?;
        let reference = as_of.unwrap_or_else(Utc::now).min(Utc::now());
        let stale_days = stale_after_days();
        let conn = self.lock()?;
        let mut clusters = Vec::new();
        for cluster in &contract.entitlement.covered_clusters {
            let latest = conn
                .query_row(
                    "SELECT observed_at, source, status, control_plane_nodes, worker_nodes, error
                     FROM capacity_observations WHERE cluster_id = ?1 AND observed_at <= ?2
                     ORDER BY observed_at DESC, rowid DESC LIMIT 1",
                    params![cluster, ts(reference)],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<u32>>(3)?,
                            r.get::<_, Option<u32>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()?;
            let entry = match latest {
                None => ClusterCapacity {
                    cluster_id: cluster.clone(),
                    state: "missing",
                    worker_nodes: None,
                    control_plane_nodes: None,
                    last_known_worker_nodes: None,
                    observed_at: None,
                    source: None,
                    error: None,
                },
                Some((at, source, status, cp, workers, error)) => {
                    let at = parse_ts(&at)?;
                    let fresh = reference - at <= Duration::days(stale_days);
                    let (state, usable) = match (status.as_str(), fresh) {
                        ("error", _) => ("error", false),
                        (_, true) => ("ok", true),
                        (_, false) => ("stale", false),
                    };
                    ClusterCapacity {
                        cluster_id: cluster.clone(),
                        state,
                        worker_nodes: if usable { workers } else { None },
                        control_plane_nodes: if usable { cp } else { None },
                        last_known_worker_nodes: if state == "stale" { workers } else { None },
                        observed_at: Some(at),
                        source: Some(source),
                        error,
                    }
                }
            };
            clusters.push(entry);
        }
        let unknown: Vec<String> = clusters
            .iter()
            .filter(|c| c.worker_nodes.is_none())
            .map(|c| c.cluster_id.clone())
            .collect();
        let complete = !clusters.is_empty() && unknown.is_empty();
        let billable =
            complete.then(|| clusters.iter().filter_map(|c| c.worker_nodes).sum::<u32>());
        let allowance = contract.entitlement.node_allowance;
        Ok(Reconciliation {
            contract_id: contract.id,
            reference_time: reference,
            stale_after_days: stale_days,
            clusters,
            over_allowance: billable.zip(allowance).map(|(b, a)| b > a),
            billable_worker_nodes: billable,
            complete,
            unknown_clusters: unknown,
            node_allowance: allowance,
            node_treatment: contract.quote.node_treatment,
            note: "Unknown clusters (missing, stale or failed observation) are never counted as zero. No billable total is produced until every covered cluster has a fresh observation.",
        })
    }

    // ── invoices ────────────────────────────────────────────────

    pub fn create_invoice(&self, caller: &Caller, input: NewInvoice) -> Result<Invoice> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "invoices are created by the service desk".into(),
            ));
        }
        let start = parse_ts(&input.period_start)
            .map_err(|_| CommercialError::Invalid("period_start must be RFC 3339".into()))?;
        let end = parse_ts(&input.period_end)
            .map_err(|_| CommercialError::Invalid("period_end must be RFC 3339".into()))?;
        if end <= start {
            return Err(CommercialError::Invalid(
                "period_end must be after period_start".into(),
            ));
        }
        if input.lines.is_empty() || input.lines.len() > 100 {
            return Err(CommercialError::Invalid(
                "an invoice needs 1-100 line items".into(),
            ));
        }
        let ext = input
            .external_reference
            .as_deref()
            .map(|e| bounded("external_reference", e, 128, true))
            .transpose()?;
        let note = input
            .note
            .as_deref()
            .map(|n| bounded("note", n, 2000, false))
            .transpose()?;
        let mut lines = Vec::new();
        let mut total: i64 = 0;
        for (i, l) in input.lines.iter().enumerate() {
            let description = bounded("description", &l.description, 300, true)?;
            if !(1..=1_000_000).contains(&l.quantity)
                || !(0..=1_000_000_000_000).contains(&l.unit_price_minor)
            {
                return Err(CommercialError::Invalid(
                    "quantity must be 1..1000000 and unit_price_minor a non-negative amount".into(),
                ));
            }
            let amount = l
                .quantity
                .checked_mul(l.unit_price_minor)
                .ok_or_else(|| CommercialError::Invalid("line amount overflows".into()))?;
            total = total
                .checked_add(amount)
                .ok_or_else(|| CommercialError::Invalid("invoice total overflows".into()))?;
            let cluster = l
                .cluster_id
                .as_deref()
                .map(|c| bounded("cluster_id", c, 128, true))
                .transpose()?;
            lines.push((
                i as i64 + 1,
                description,
                l.quantity,
                l.unit_price_minor,
                amount,
                cluster,
            ));
        }
        let conn = self.lock()?;
        let contract = load_contract(&conn, &input.contract_id, Utc::now())?
            .ok_or(CommercialError::NotFound)?;
        let id = new_id("inv");
        let now = ts(Utc::now());
        let inserted = conn.execute(
            "INSERT INTO invoices (id, org_id, contract_id, currency, period_start, period_end, status,
                total_minor, external_reference, note, created_by, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,'draft',?7,?8,?9,?10,?11,?11)",
            params![id, contract.org_id, contract.id, contract.quote.currency, ts(start), ts(end), total, ext, note, caller.username, now],
        );
        match inserted {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(CommercialError::Conflict(
                    "an invoice with this external_reference already exists".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        }
        for (pos, description, qty, unit, amount, cluster) in &lines {
            conn.execute(
                "INSERT INTO invoice_lines (id, invoice_id, position, description, quantity, unit_price_minor, amount_minor, cluster_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![new_id("il"), id, pos, description, qty, unit, amount, cluster],
            )?;
        }
        record(
            &conn,
            &contract.id,
            &caller.username,
            "invoice_drafted",
            json!({ "invoice": id, "total_minor": total }),
        )?;
        load_invoice(&conn, &id)?.ok_or(CommercialError::NotFound)
    }

    fn visible_invoice(&self, conn: &Connection, caller: &Caller, id: &str) -> Result<Invoice> {
        let inv = load_invoice(conn, id)?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(conn, caller, &inv.org_id)? || (!caller.admin && inv.status == "draft") {
            return Err(CommercialError::NotFound);
        }
        Ok(inv)
    }

    pub fn get_invoice(&self, caller: &Caller, id: &str) -> Result<Invoice> {
        let conn = self.lock()?;
        self.visible_invoice(&conn, caller, id)
    }

    pub fn list_invoices(&self, caller: &Caller) -> Result<Vec<Invoice>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT id FROM invoices ORDER BY created_at DESC, id")?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let mut out = Vec::new();
        for id in ids {
            match self.visible_invoice(&conn, caller, &id) {
                Ok(inv) => out.push(inv),
                Err(CommercialError::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    pub fn issue_invoice(&self, caller: &Caller, id: &str) -> Result<Invoice> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the service desk issues invoices".into(),
            ));
        }
        let conn = self.lock()?;
        let inv = self.visible_invoice(&conn, caller, id)?;
        if inv.status != "draft" {
            return Err(CommercialError::Conflict(format!(
                "the invoice is {}",
                inv.status
            )));
        }
        let now = ts(Utc::now());
        conn.execute(
            "UPDATE invoices SET status = 'issued', issued_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        record(
            &conn,
            &inv.contract_id,
            &caller.username,
            "invoice_issued",
            json!({ "invoice": id }),
        )?;
        sync_contract_payment(&conn, &inv.contract_id, &caller.username)?;
        load_invoice(&conn, id)?.ok_or(CommercialError::NotFound)
    }

    /// Record that an issued invoice was paid. Idempotent on
    /// `payment_reference`: repeating the same call changes nothing, and the
    /// same reference can never be applied to a second invoice.
    pub fn mark_invoice_paid(
        &self,
        caller: &Caller,
        id: &str,
        payment_reference: &str,
    ) -> Result<Invoice> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the service desk records payments".into(),
            ));
        }
        let reference = bounded("payment_reference", payment_reference, 128, true)?;
        let conn = self.lock()?;
        let inv = self.visible_invoice(&conn, caller, id)?;
        match inv.status.as_str() {
            "paid" if inv.payment_reference.as_deref() == Some(reference.as_str()) => {
                return Ok(inv)
            }
            "paid" => {
                return Err(CommercialError::Conflict(
                    "the invoice is already paid under a different payment reference".into(),
                ));
            }
            "issued" => {}
            other => {
                return Err(CommercialError::Conflict(format!(
                    "a {other} invoice cannot be paid"
                )))
            }
        }
        let now = ts(Utc::now());
        let updated = conn.execute(
            "UPDATE invoices SET status = 'paid', payment_reference = ?2, paid_at = ?3, updated_at = ?3 WHERE id = ?1",
            params![id, reference, now],
        );
        match updated {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(CommercialError::Conflict(
                    "this payment reference was already applied to another invoice".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        }
        record(
            &conn,
            &inv.contract_id,
            &caller.username,
            "invoice_paid",
            json!({ "invoice": id, "payment_reference": reference }),
        )?;
        sync_contract_payment(&conn, &inv.contract_id, &caller.username)?;
        load_invoice(&conn, id)?.ok_or(CommercialError::NotFound)
    }

    pub fn void_invoice(&self, caller: &Caller, id: &str, reason: &str) -> Result<Invoice> {
        if !caller.admin {
            return Err(CommercialError::Forbidden(
                "only the service desk voids invoices".into(),
            ));
        }
        let reason = bounded("reason", reason, 2000, true)?;
        let conn = self.lock()?;
        let inv = self.visible_invoice(&conn, caller, id)?;
        if !matches!(inv.status.as_str(), "draft" | "issued") {
            return Err(CommercialError::Conflict(format!(
                "a {} invoice cannot be voided",
                inv.status
            )));
        }
        let now = ts(Utc::now());
        conn.execute(
            "UPDATE invoices SET status = 'void', voided_at = ?2, void_reason = ?3, updated_at = ?2 WHERE id = ?1",
            params![id, now, reason],
        )?;
        record(
            &conn,
            &inv.contract_id,
            &caller.username,
            "invoice_voided",
            json!({ "invoice": id, "reason": reason }),
        )?;
        sync_contract_payment(&conn, &inv.contract_id, &caller.username)?;
        load_invoice(&conn, id)?.ok_or(CommercialError::NotFound)
    }

    /// Plain-text rendering for download. Tax treatment and legal issuance
    /// stay with the external billing system.
    pub fn invoice_document(&self, caller: &Caller, id: &str) -> Result<String> {
        let inv = self.get_invoice(caller, id)?;
        let mut out = String::new();
        let _ = writeln!(out, "INVOICE RECORD {}", inv.id);
        if let Some(e) = &inv.external_reference {
            let _ = writeln!(out, "External reference: {e}");
        }
        let _ = writeln!(out, "Status: {}", inv.status);
        let _ = writeln!(out, "Contract: {}", inv.contract_id);
        let _ = writeln!(
            out,
            "Period: {} to {}",
            inv.period_start.format("%Y-%m-%d"),
            inv.period_end.format("%Y-%m-%d")
        );
        let _ = writeln!(out, "Currency: {}", inv.currency);
        let _ = writeln!(out);
        for l in &inv.lines {
            let _ = writeln!(
                out,
                "{:>3}. {} x {} @ {} = {}{}",
                l.position,
                l.description,
                l.quantity,
                money(l.unit_price_minor, &inv.currency),
                money(l.amount_minor, &inv.currency),
                l.cluster_id
                    .as_deref()
                    .map(|c| format!(" [{c}]"))
                    .unwrap_or_default()
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "Total: {}", money(inv.total_minor, &inv.currency));
        if let Some(p) = &inv.payment_reference {
            let _ = writeln!(out, "Payment reference: {p}");
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Taxes are handled by the billing system and are not included in this record."
        );
        Ok(out)
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

    fn contract(
        s: &CommercialStore,
        name: &str,
        member: &str,
        clusters: &[&str],
        allowance: Option<u32>,
    ) -> (String, String) {
        let org = s.create_org(name, Some(member)).unwrap();
        let req = s
            .create_quote_request(
                &user(member),
                QuoteRequestInput {
                    org_id: Some(org.id.clone()),
                    offering: "supported".into(),
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
                    pricing_unit: "worker node / year".into(),
                    unit_price_minor: Some(60_000),
                    included_capacity: String::new(),
                    duration_months: 12,
                    response_targets: vec!["sev1: 1h".into()],
                    support_hours: String::new(),
                    extra_exclusions: vec![],
                    node_treatment: "control-plane nodes are not billed".into(),
                },
            )
            .unwrap();
        s.update_entitlement(
            "desk",
            &c.id,
            Entitlement {
                covered_clusters: clusters.iter().map(|c| c.to_string()).collect(),
                support_tier: "supported".into(),
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
                node_allowance: allowance,
                ..Default::default()
            },
        )
        .unwrap();
        (org.id, c.id)
    }

    fn ok_obs(cluster: &str, workers: u32, at: Option<String>) -> NewObservation {
        NewObservation {
            cluster_id: cluster.into(),
            source: "collector".into(),
            status: "ok".into(),
            total_nodes: Some(workers + 1),
            control_plane_nodes: Some(1),
            worker_nodes: Some(workers),
            error: None,
            observed_at: at,
        }
    }

    fn err_obs(cluster: &str, at: Option<String>) -> NewObservation {
        NewObservation {
            cluster_id: cluster.into(),
            source: "collector".into(),
            status: "error".into(),
            total_nodes: None,
            control_plane_nodes: None,
            worker_nodes: None,
            error: Some("API unreachable".into()),
            observed_at: at,
        }
    }

    #[test]
    fn missing_observation_is_never_zero() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["cl-1"], Some(10));
        let r = s.reconcile_capacity(&user("ann"), &c, None).unwrap();
        assert_eq!(r.clusters[0].state, "missing");
        assert_eq!(r.clusters[0].worker_nodes, None);
        assert!(!r.complete);
        assert_eq!(
            r.billable_worker_nodes, None,
            "no observation must not become a zero-node claim"
        );
        assert_eq!(r.over_allowance, None);
        assert_eq!(r.unknown_clusters, vec!["cl-1".to_string()]);
    }

    #[test]
    fn fresh_stale_and_failed_observations() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a", "b", "c"], Some(10));
        s.record_observation(&desk(), ok_obs("a", 4, None)).unwrap();
        let old = (Utc::now() - Duration::days(30)).to_rfc3339();
        s.record_observation(&desk(), ok_obs("b", 6, Some(old)))
            .unwrap();
        s.record_observation(&desk(), ok_obs("c", 5, None)).unwrap();
        // The newest observation for c is a failure: unknown, not the older 5.
        s.record_observation(&desk(), err_obs("c", None)).unwrap();
        let r = s.reconcile_capacity(&user("ann"), &c, None).unwrap();
        let by = |id: &str| r.clusters.iter().find(|x| x.cluster_id == id).unwrap();
        assert_eq!(by("a").state, "ok");
        assert_eq!(by("a").worker_nodes, Some(4));
        assert_eq!(by("b").state, "stale");
        assert_eq!(by("b").worker_nodes, None);
        assert_eq!(by("b").last_known_worker_nodes, Some(6));
        assert_eq!(by("c").state, "error");
        assert_eq!(r.billable_worker_nodes, None, "never a partial sum");
        assert!(!r.complete);
    }

    #[test]
    fn complete_reconciliation_sums_and_checks_allowance() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a", "b"], Some(8));
        s.record_observation(&desk(), ok_obs("a", 4, None)).unwrap();
        s.record_observation(&desk(), ok_obs("b", 5, None)).unwrap();
        let r = s.reconcile_capacity(&user("ann"), &c, None).unwrap();
        assert!(r.complete);
        assert_eq!(r.billable_worker_nodes, Some(9));
        assert_eq!(r.over_allowance, Some(true));
        assert!(r.node_treatment.contains("control-plane"));
    }

    #[test]
    fn observation_validation() {
        let s = CommercialStore::open(":memory:").unwrap();
        let mut bad = ok_obs("a", 3, None);
        bad.worker_nodes = None;
        assert!(s.record_observation(&desk(), bad).is_err());
        let mut zero = ok_obs("a", 0, None);
        zero.total_nodes = Some(0);
        zero.control_plane_nodes = Some(0);
        assert!(s.record_observation(&desk(), zero).is_err());
        let mut skew = ok_obs("a", 3, None);
        skew.total_nodes = Some(99);
        assert!(s.record_observation(&desk(), skew).is_err());
        let mut counts_on_error = err_obs("a", None);
        counts_on_error.worker_nodes = Some(0);
        assert!(s.record_observation(&desk(), counts_on_error).is_err());
        let future = (Utc::now() + Duration::days(3)).to_rfc3339();
        assert!(s
            .record_observation(&desk(), ok_obs("a", 3, Some(future)))
            .is_err());
        assert!(matches!(
            s.record_observation(&user("ann"), ok_obs("a", 3, None)),
            Err(CommercialError::Forbidden(_))
        ));
    }

    #[test]
    fn reconciliation_is_org_isolated() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a"], None);
        s.create_org("Beta", Some("bob")).unwrap();
        assert!(matches!(
            s.reconcile_capacity(&user("bob"), &c, None),
            Err(CommercialError::NotFound)
        ));
        assert!(s.list_observations(&user("ann"), None).is_err());
    }

    fn invoice_input(contract: &str) -> NewInvoice {
        NewInvoice {
            contract_id: contract.into(),
            period_start: "2026-01-01T00:00:00Z".into(),
            period_end: "2026-12-31T00:00:00Z".into(),
            external_reference: Some("INV-1001".into()),
            note: None,
            lines: vec![
                NewLine {
                    description: "Supported, worker nodes".into(),
                    quantity: 5,
                    unit_price_minor: 60_000,
                    cluster_id: Some("a".into()),
                },
                NewLine {
                    description: "Setup".into(),
                    quantity: 1,
                    unit_price_minor: 5_050,
                    cluster_id: None,
                },
            ],
        }
    }

    #[test]
    fn invoice_lifecycle_totals_and_visibility() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a"], None);
        let inv = s.create_invoice(&desk(), invoice_input(&c)).unwrap();
        assert_eq!(inv.total_minor, 5 * 60_000 + 5_050);
        assert_eq!(inv.currency, "USD");
        assert_eq!(inv.status, "draft");
        // Customers cannot see drafts.
        assert!(matches!(
            s.get_invoice(&user("ann"), &inv.id),
            Err(CommercialError::NotFound)
        ));
        assert!(s.list_invoices(&user("ann")).unwrap().is_empty());
        let issued = s.issue_invoice(&desk(), &inv.id).unwrap();
        assert_eq!(issued.status, "issued");
        assert!(s.get_invoice(&user("ann"), &inv.id).is_ok());
        assert_eq!(
            s.get_contract(&desk(), &c).unwrap().payment_status,
            PaymentStatus::Invoiced
        );
        assert!(matches!(
            s.issue_invoice(&desk(), &inv.id),
            Err(CommercialError::Conflict(_))
        ));
        let paid = s.mark_invoice_paid(&desk(), &inv.id, "PAY-1").unwrap();
        assert_eq!(paid.status, "paid");
        assert_eq!(
            s.get_contract(&desk(), &c).unwrap().payment_status,
            PaymentStatus::Paid
        );
        // A paid invoice cannot be voided.
        assert!(matches!(
            s.void_invoice(&desk(), &inv.id, "oops"),
            Err(CommercialError::Conflict(_))
        ));
    }

    #[test]
    fn repeated_payment_is_idempotent_and_never_duplicates() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a"], None);
        let inv = s.create_invoice(&desk(), invoice_input(&c)).unwrap();
        s.issue_invoice(&desk(), &inv.id).unwrap();
        let first = s.mark_invoice_paid(&desk(), &inv.id, "PAY-1").unwrap();
        let again = s.mark_invoice_paid(&desk(), &inv.id, "PAY-1").unwrap();
        assert_eq!(
            first.paid_at, again.paid_at,
            "replay must not change anything"
        );
        let events = s
            .history(&desk(), &c)
            .unwrap()
            .into_iter()
            .filter(|e| e.event == "invoice_paid")
            .count();
        assert_eq!(events, 1, "a replay must not be recorded twice");
        // A different reference on an already-paid invoice is a conflict.
        assert!(matches!(
            s.mark_invoice_paid(&desk(), &inv.id, "PAY-2"),
            Err(CommercialError::Conflict(_))
        ));
        // The same payment reference cannot settle a second invoice.
        let mut second = invoice_input(&c);
        second.external_reference = Some("INV-1002".into());
        let inv2 = s.create_invoice(&desk(), second).unwrap();
        s.issue_invoice(&desk(), &inv2.id).unwrap();
        assert!(matches!(
            s.mark_invoice_paid(&desk(), &inv2.id, "PAY-1"),
            Err(CommercialError::Conflict(_))
        ));
        assert_eq!(s.get_invoice(&desk(), &inv2.id).unwrap().status, "issued");
        // Duplicate external invoice number is rejected too.
        assert!(matches!(
            s.create_invoice(&desk(), invoice_input(&c)),
            Err(CommercialError::Conflict(_))
        ));
    }

    #[test]
    fn invoice_validation_and_isolation() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a"], None);
        s.create_org("Beta", Some("bob")).unwrap();
        let mut neg = invoice_input(&c);
        neg.lines[0].unit_price_minor = -1;
        assert!(s.create_invoice(&desk(), neg).is_err());
        let mut empty = invoice_input(&c);
        empty.lines.clear();
        assert!(s.create_invoice(&desk(), empty).is_err());
        let mut reversed = invoice_input(&c);
        reversed.period_end = "2025-01-01T00:00:00Z".into();
        assert!(s.create_invoice(&desk(), reversed).is_err());
        assert!(matches!(
            s.create_invoice(&user("ann"), invoice_input(&c)),
            Err(CommercialError::Forbidden(_))
        ));
        let inv = s.create_invoice(&desk(), invoice_input(&c)).unwrap();
        s.issue_invoice(&desk(), &inv.id).unwrap();
        assert!(matches!(
            s.get_invoice(&user("bob"), &inv.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.invoice_document(&user("bob"), &inv.id),
            Err(CommercialError::NotFound)
        ));
        assert!(s.list_invoices(&user("bob")).unwrap().is_empty());
    }

    #[test]
    fn document_and_voiding() {
        let s = CommercialStore::open(":memory:").unwrap();
        let (_, c) = contract(&s, "Acme", "ann", &["a"], None);
        let inv = s.create_invoice(&desk(), invoice_input(&c)).unwrap();
        s.issue_invoice(&desk(), &inv.id).unwrap();
        let doc = s.invoice_document(&user("ann"), &inv.id).unwrap();
        assert!(doc.contains("INV-1001"));
        assert!(doc.contains("3050.50 USD"));
        assert!(doc.contains("Taxes are handled"));
        let v = s.void_invoice(&desk(), &inv.id, "duplicate").unwrap();
        assert_eq!(v.status, "void");
        assert_eq!(
            s.get_contract(&desk(), &c).unwrap().payment_status,
            PaymentStatus::NotInvoiced
        );
    }

    #[test]
    fn money_formatting() {
        assert_eq!(money(305_050, "USD"), "3050.50 USD");
        assert_eq!(money(5, "EUR"), "0.05 EUR");
        assert_eq!(money(-250, "EUR"), "-2.50 EUR");
    }
}
