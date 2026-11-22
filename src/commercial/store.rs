// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! SQLite-backed commercial store.
//!
//! Single-instance only: SQLite in a single file is safe for exactly one
//! commercial API replica. Running several replicas requires a shared
//! transactional store (see docs/COMMERCIAL_OFFERINGS.md).
//!
//! All customer-visible reads go through [`Caller`], so organization
//! isolation is enforced here rather than being re-implemented per handler.

use super::catalog;
use super::model::*;
use anyhow::Context;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::sync::Mutex;

/// Ordered, append-only migrations. Never edit a shipped entry; add a new
/// version. There are no down-migrations: rollback is restoring the backup
/// taken before upgrade (docs/COMMERCIAL_OFFERINGS.md).
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "initial commercial schema",
        "CREATE TABLE orgs (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        created_at TEXT NOT NULL
    );
    CREATE TABLE org_members (
        org_id TEXT NOT NULL REFERENCES orgs(id),
        username TEXT NOT NULL,
        added_at TEXT NOT NULL,
        PRIMARY KEY (org_id, username)
    );
    CREATE TABLE quote_requests (
        id TEXT PRIMARY KEY,
        org_id TEXT REFERENCES orgs(id),
        requested_by TEXT NOT NULL,
        offering TEXT NOT NULL,
        company TEXT NOT NULL,
        contact_name TEXT NOT NULL,
        contact_email TEXT NOT NULL,
        cluster_count INTEGER NOT NULL,
        worker_node_count INTEGER NOT NULL,
        workload_size TEXT NOT NULL,
        region TEXT NOT NULL,
        desired_coverage TEXT NOT NULL,
        requirements TEXT NOT NULL,
        status TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE contracts (
        id TEXT PRIMARY KEY,
        org_id TEXT NOT NULL REFERENCES orgs(id),
        quote_request_id TEXT REFERENCES quote_requests(id),
        offering TEXT NOT NULL,
        status TEXT NOT NULL,
        payment_status TEXT NOT NULL DEFAULT 'not_invoiced',
        source TEXT NOT NULL,
        quote TEXT NOT NULL,
        entitlement TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX contracts_org ON contracts(org_id);
    CREATE TABLE contract_history (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        contract_id TEXT NOT NULL REFERENCES contracts(id),
        at TEXT NOT NULL,
        actor TEXT NOT NULL,
        event TEXT NOT NULL,
        detail TEXT NOT NULL
    );
    CREATE INDEX contract_history_contract ON contract_history(contract_id);",
    ),
    (
        2,
        "support cases",
        "CREATE TABLE support_cases (
            id TEXT PRIMARY KEY,
            org_id TEXT NOT NULL REFERENCES orgs(id),
            cluster_id TEXT NOT NULL,
            contract_id TEXT,
            severity TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            status TEXT NOT NULL,
            owner TEXT,
            created_by TEXT NOT NULL,
            coverage TEXT NOT NULL,
            first_response_due TEXT,
            first_response_at TEXT,
            resolution_due TEXT,
            resolution_paused_secs INTEGER,
            reopen_count INTEGER NOT NULL DEFAULT 0,
            resolved_at TEXT,
            closed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX support_cases_org ON support_cases(org_id);
        CREATE TABLE support_messages (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            case_id TEXT NOT NULL REFERENCES support_cases(id),
            at TEXT NOT NULL,
            author TEXT NOT NULL,
            author_kind TEXT NOT NULL,
            body TEXT NOT NULL,
            internal INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX support_messages_case ON support_messages(case_id);
        CREATE TABLE support_attachments (
            id TEXT PRIMARY KEY,
            case_id TEXT NOT NULL REFERENCES support_cases(id),
            message_seq INTEGER,
            filename TEXT NOT NULL,
            content_type TEXT NOT NULL,
            size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            data BLOB,
            uploaded_by TEXT NOT NULL,
            at TEXT NOT NULL,
            purged_at TEXT
        );
        CREATE INDEX support_attachments_case ON support_attachments(case_id);
        CREATE TABLE support_events (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            case_id TEXT NOT NULL REFERENCES support_cases(id),
            at TEXT NOT NULL,
            actor TEXT NOT NULL,
            event TEXT NOT NULL,
            detail TEXT NOT NULL
        );
        CREATE INDEX support_events_case ON support_events(case_id);",
    ),
    (
        3,
        "service engagements and managed operations",
        "CREATE TABLE engagements (
            id TEXT PRIMARY KEY,
            org_id TEXT NOT NULL REFERENCES orgs(id),
            contract_id TEXT,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            owner TEXT,
            created_by TEXT NOT NULL,
            accepted_by TEXT,
            accepted_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX engagements_org ON engagements(org_id);
        CREATE TABLE engagement_milestones (
            engagement_id TEXT NOT NULL REFERENCES engagements(id),
            key TEXT NOT NULL,
            position INTEGER NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            owner TEXT,
            due_at TEXT,
            completed_at TEXT,
            PRIMARY KEY (engagement_id, key)
        );
        CREATE TABLE engagement_evidence (
            id TEXT PRIMARY KEY,
            engagement_id TEXT NOT NULL REFERENCES engagements(id),
            milestone_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            reference TEXT,
            note TEXT,
            added_by TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE INDEX engagement_evidence_eng ON engagement_evidence(engagement_id);
        CREATE TABLE managed_enrollments (
            id TEXT PRIMARY KEY,
            org_id TEXT NOT NULL REFERENCES orgs(id),
            contract_id TEXT NOT NULL,
            cluster_id TEXT NOT NULL,
            status TEXT NOT NULL,
            covered_components TEXT NOT NULL,
            maintenance_windows TEXT NOT NULL,
            monitoring_signals TEXT NOT NULL,
            operational_owner TEXT,
            remote_enabled INTEGER NOT NULL DEFAULT 0,
            remote_scope TEXT NOT NULL DEFAULT '[]',
            remote_credential_ref TEXT,
            remote_enabled_by TEXT,
            remote_enabled_at TEXT,
            remote_revoked_by TEXT,
            remote_revoked_at TEXT,
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX managed_enrollments_org ON managed_enrollments(org_id);
        CREATE TABLE managed_tasks (
            id TEXT PRIMARY KEY,
            enrollment_id TEXT NOT NULL REFERENCES managed_enrollments(id),
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            window_text TEXT NOT NULL,
            status TEXT NOT NULL,
            proposed_by TEXT NOT NULL,
            approved_by TEXT,
            approved_at TEXT,
            completed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE managed_evidence (
            id TEXT PRIMARY KEY,
            enrollment_id TEXT NOT NULL REFERENCES managed_enrollments(id),
            kind TEXT NOT NULL,
            result TEXT NOT NULL,
            performed_at TEXT NOT NULL,
            reference TEXT,
            note TEXT,
            added_by TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE TABLE managed_incidents (
            id TEXT PRIMARY KEY,
            enrollment_id TEXT NOT NULL REFERENCES managed_enrollments(id),
            title TEXT NOT NULL,
            severity TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            opened_by TEXT NOT NULL,
            opened_at TEXT NOT NULL,
            resolved_at TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE managed_reports (
            id TEXT PRIMARY KEY,
            enrollment_id TEXT NOT NULL REFERENCES managed_enrollments(id),
            period_start TEXT NOT NULL,
            period_end TEXT NOT NULL,
            summary TEXT NOT NULL,
            reference TEXT,
            added_by TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE TABLE remediation_policies (
            id TEXT PRIMARY KEY,
            enrollment_id TEXT NOT NULL REFERENCES managed_enrollments(id),
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            permissions TEXT NOT NULL,
            status TEXT NOT NULL,
            created_by TEXT NOT NULL,
            authorized_by TEXT,
            authorized_at TEXT,
            revoked_by TEXT,
            revoked_at TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE service_events (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            subject TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            at TEXT NOT NULL,
            actor TEXT NOT NULL,
            event TEXT NOT NULL,
            detail TEXT NOT NULL
        );
        CREATE INDEX service_events_subject ON service_events(subject, subject_id);",
    ),
    (
        4,
        "capacity observations and invoice records",
        "CREATE TABLE capacity_observations (
            id TEXT PRIMARY KEY,
            cluster_id TEXT NOT NULL,
            observed_at TEXT NOT NULL,
            source TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('ok', 'error')),
            total_nodes INTEGER,
            control_plane_nodes INTEGER,
            worker_nodes INTEGER,
            error TEXT,
            recorded_by TEXT NOT NULL,
            recorded_at TEXT NOT NULL,
            CHECK (status = 'error' OR worker_nodes IS NOT NULL)
        );
        CREATE INDEX capacity_observations_cluster ON capacity_observations(cluster_id, observed_at);
        CREATE TABLE invoices (
            id TEXT PRIMARY KEY,
            org_id TEXT NOT NULL REFERENCES orgs(id),
            contract_id TEXT NOT NULL REFERENCES contracts(id),
            currency TEXT NOT NULL,
            period_start TEXT NOT NULL,
            period_end TEXT NOT NULL,
            status TEXT NOT NULL,
            total_minor INTEGER NOT NULL,
            external_reference TEXT,
            payment_reference TEXT,
            note TEXT,
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            issued_at TEXT,
            paid_at TEXT,
            voided_at TEXT,
            void_reason TEXT
        );
        CREATE INDEX invoices_org ON invoices(org_id);
        CREATE UNIQUE INDEX invoices_external_reference
            ON invoices(org_id, external_reference) WHERE external_reference IS NOT NULL;
        CREATE UNIQUE INDEX invoices_payment_reference
            ON invoices(org_id, payment_reference) WHERE payment_reference IS NOT NULL;
        CREATE TABLE invoice_lines (
            id TEXT PRIMARY KEY,
            invoice_id TEXT NOT NULL REFERENCES invoices(id),
            position INTEGER NOT NULL,
            description TEXT NOT NULL,
            quantity INTEGER NOT NULL,
            unit_price_minor INTEGER NOT NULL,
            amount_minor INTEGER NOT NULL,
            cluster_id TEXT
        );
        CREATE INDEX invoice_lines_invoice ON invoice_lines(invoice_id);",
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum CommercialError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
}

impl From<rusqlite::Error> for CommercialError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(e.into())
    }
}

pub(super) type Result<T> = std::result::Result<T, CommercialError>;

/// Who is asking. `admin` is a commercial administrator (users.admin) who
/// may see and change every organization; everyone else sees only the
/// organizations they are a member of.
#[derive(Debug, Clone)]
pub struct Caller {
    pub username: String,
    pub admin: bool,
}

pub(super) fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(super) fn parse_ts(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

pub(super) fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

pub(super) fn bounded(label: &str, s: &str, max: usize, required: bool) -> Result<String> {
    let s = s.trim();
    if required && s.is_empty() {
        return Err(CommercialError::Invalid(format!("{label} is required")));
    }
    if s.chars().count() > max {
        return Err(CommercialError::Invalid(format!(
            "{label} must be at most {max} characters"
        )));
    }
    Ok(s.to_string())
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QuoteRequestInput {
    pub org_id: Option<String>,
    pub offering: String,
    pub company: String,
    pub contact_name: String,
    pub contact_email: String,
    #[serde(default)]
    pub cluster_count: u32,
    #[serde(default)]
    pub worker_node_count: u32,
    #[serde(default)]
    pub workload_size: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub desired_coverage: String,
    #[serde(default)]
    pub requirements: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QuoteInput {
    /// Required when the request was submitted without an organization.
    pub org_id: Option<String>,
    pub currency: String,
    pub pricing_unit: String,
    pub unit_price_minor: Option<i64>,
    #[serde(default)]
    pub included_capacity: String,
    pub duration_months: u32,
    #[serde(default)]
    pub response_targets: Vec<String>,
    #[serde(default)]
    pub support_hours: String,
    #[serde(default)]
    pub extra_exclusions: Vec<String>,
    #[serde(default)]
    pub node_treatment: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImportBundle {
    pub org_id: String,
    pub offering: String,
    pub quote: Quote,
    pub entitlement: Entitlement,
    /// draft | awaiting_acceptance | active (default draft)
    #[serde(default)]
    pub status: Option<String>,
}

pub struct CommercialStore {
    pub(super) conn: Mutex<Connection>,
}

impl CommercialStore {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
            Connection::open(path)?
        };
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .ok();
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let path = std::env::var("ZORVIA_COMMERCIAL_DB").unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("zorvia")
                .join("commercial.db")
                .to_string_lossy()
                .to_string()
        });
        Self::open(&path).with_context(|| format!("opening commercial db {path}"))
    }

    fn migrate(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );",
        )?;
        let current: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        let known = MIGRATIONS.last().map(|m| m.0).unwrap_or(0);
        if current > known {
            // A newer binary wrote this database. Refuse rather than run old
            // code against a schema it does not understand.
            anyhow::bail!(
                "commercial database is at schema v{current} but this build only knows v{known}; \
                 upgrade Zorvia or restore the pre-upgrade backup"
            );
        }
        for (version, name, sql) in MIGRATIONS.iter().filter(|m| m.0 > current) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("migration v{version} ({name})"))?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                params![version, name, ts(Utc::now())],
            )?;
            tx.commit()?;
            log::info!("commercial db migrated to v{version}: {name}");
        }
        Ok(())
    }

    pub(super) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| CommercialError::Internal(anyhow::anyhow!("{e}")))
    }

    // ── organizations ────────────────────────────────────────────

    pub fn create_org(&self, name: &str, first_member: Option<&str>) -> Result<Org> {
        let name = bounded("name", name, 200, true)?;
        let conn = self.lock()?;
        let exists: Option<String> = conn
            .query_row("SELECT id FROM orgs WHERE name = ?1", [&name], |r| r.get(0))
            .optional()?;
        if exists.is_some() {
            return Err(CommercialError::Conflict(
                "an organization with this name already exists".into(),
            ));
        }
        let now = Utc::now();
        let org = Org {
            id: new_id("org"),
            name,
            created_at: now,
        };
        conn.execute(
            "INSERT INTO orgs (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![org.id, org.name, ts(now)],
        )?;
        if let Some(m) = first_member.filter(|m| !m.trim().is_empty()) {
            conn.execute(
                "INSERT OR IGNORE INTO org_members (org_id, username, added_at) VALUES (?1, ?2, ?3)",
                params![org.id, m.trim(), ts(now)],
            )?;
        }
        Ok(org)
    }

    pub fn add_member(&self, org_id: &str, username: &str) -> Result<()> {
        let username = bounded("username", username, 128, true)?;
        let conn = self.lock()?;
        org_exists(&conn, org_id)?;
        conn.execute(
            "INSERT OR IGNORE INTO org_members (org_id, username, added_at) VALUES (?1, ?2, ?3)",
            params![org_id, username, ts(Utc::now())],
        )?;
        Ok(())
    }

    pub fn remove_member(&self, org_id: &str, username: &str) -> Result<()> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM org_members WHERE org_id = ?1 AND username = ?2",
            params![org_id, username],
        )?;
        if n == 0 {
            return Err(CommercialError::NotFound);
        }
        Ok(())
    }

    pub fn list_orgs(&self, caller: &Caller) -> Result<Vec<Org>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at FROM orgs
             WHERE ?1 = 1 OR id IN (SELECT org_id FROM org_members WHERE username = ?2)
             ORDER BY name",
        )?;
        let rows = stmt.query_map(params![caller.admin as i64, caller.username], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, created) = row?;
            out.push(Org {
                id,
                name,
                created_at: parse_ts(&created)?,
            });
        }
        Ok(out)
    }

    pub(super) fn allowed(&self, conn: &Connection, caller: &Caller, org_id: &str) -> Result<bool> {
        let _ = self;
        if caller.admin {
            return Ok(true);
        }
        Ok(is_member(conn, org_id, &caller.username)?)
    }

    // ── quote requests ───────────────────────────────────────────

    pub fn create_quote_request(
        &self,
        caller: &Caller,
        input: QuoteRequestInput,
    ) -> Result<QuoteRequest> {
        let offering = catalog::find(&input.offering)
            .filter(|o| o.requestable)
            .ok_or_else(|| {
                CommercialError::Invalid("unknown or non-requestable offering".into())
            })?;
        let company = bounded("company", &input.company, 200, true)?;
        let contact_name = bounded("contact_name", &input.contact_name, 200, true)?;
        let contact_email = bounded("contact_email", &input.contact_email, 254, true)?;
        if !contact_email.contains('@') {
            return Err(CommercialError::Invalid(
                "contact_email must be an email address".into(),
            ));
        }
        if input.cluster_count > 10_000 || input.worker_node_count > 1_000_000 {
            return Err(CommercialError::Invalid(
                "cluster/node counts out of range".into(),
            ));
        }
        let workload_size = bounded("workload_size", &input.workload_size, 200, false)?;
        let region = bounded("region", &input.region, 100, false)?;
        let desired_coverage = bounded("desired_coverage", &input.desired_coverage, 2000, false)?;
        let requirements = bounded("requirements", &input.requirements, 4000, false)?;

        let conn = self.lock()?;
        if let Some(org) = &input.org_id {
            org_exists(&conn, org)?;
            if !self.allowed(&conn, caller, org)? {
                // Same answer as a missing org: don't confirm it exists.
                return Err(CommercialError::NotFound);
            }
        }
        let now = Utc::now();
        let req = QuoteRequest {
            id: new_id("qr"),
            org_id: input.org_id,
            requested_by: caller.username.clone(),
            offering: offering.id.to_string(),
            company,
            contact_name,
            contact_email,
            cluster_count: input.cluster_count,
            worker_node_count: input.worker_node_count,
            workload_size,
            region,
            desired_coverage,
            requirements,
            status: "submitted".into(),
            created_at: now,
            updated_at: now,
        };
        conn.execute(
            "INSERT INTO quote_requests (id, org_id, requested_by, offering, company, contact_name,
                contact_email, cluster_count, worker_node_count, workload_size, region,
                desired_coverage, requirements, status, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                req.id,
                req.org_id,
                req.requested_by,
                req.offering,
                req.company,
                req.contact_name,
                req.contact_email,
                req.cluster_count,
                req.worker_node_count,
                req.workload_size,
                req.region,
                req.desired_coverage,
                req.requirements,
                req.status,
                ts(now),
                ts(now)
            ],
        )?;
        Ok(req)
    }

    fn quote_request_from_row(
        r: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<(QuoteRequest, String, String)> {
        Ok((
            QuoteRequest {
                id: r.get(0)?,
                org_id: r.get(1)?,
                requested_by: r.get(2)?,
                offering: r.get(3)?,
                company: r.get(4)?,
                contact_name: r.get(5)?,
                contact_email: r.get(6)?,
                cluster_count: r.get(7)?,
                worker_node_count: r.get(8)?,
                workload_size: r.get(9)?,
                region: r.get(10)?,
                desired_coverage: r.get(11)?,
                requirements: r.get(12)?,
                status: r.get(13)?,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            r.get(14)?,
            r.get(15)?,
        ))
    }

    const QR_COLS: &'static str = "id, org_id, requested_by, offering, company, contact_name,
        contact_email, cluster_count, worker_node_count, workload_size, region, desired_coverage,
        requirements, status, created_at, updated_at";

    fn finish_qr(t: (QuoteRequest, String, String)) -> Result<QuoteRequest> {
        let (mut q, c, u) = t;
        q.created_at = parse_ts(&c)?;
        q.updated_at = parse_ts(&u)?;
        Ok(q)
    }

    fn load_quote_request(&self, conn: &Connection, id: &str) -> Result<Option<QuoteRequest>> {
        let _ = self;
        let sql = format!("SELECT {} FROM quote_requests WHERE id = ?1", Self::QR_COLS);
        conn.query_row(&sql, [id], Self::quote_request_from_row)
            .optional()?
            .map(Self::finish_qr)
            .transpose()
    }

    fn qr_visible(&self, conn: &Connection, caller: &Caller, q: &QuoteRequest) -> Result<bool> {
        if caller.admin || q.requested_by == caller.username {
            return Ok(true);
        }
        match &q.org_id {
            Some(o) => Ok(is_member(conn, o, &caller.username)?),
            None => Ok(false),
        }
    }

    pub fn get_quote_request(&self, caller: &Caller, id: &str) -> Result<QuoteRequest> {
        let conn = self.lock()?;
        let q = self
            .load_quote_request(&conn, id)?
            .ok_or(CommercialError::NotFound)?;
        if !self.qr_visible(&conn, caller, &q)? {
            return Err(CommercialError::NotFound);
        }
        Ok(q)
    }

    pub fn list_quote_requests(&self, caller: &Caller) -> Result<Vec<QuoteRequest>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {} FROM quote_requests ORDER BY created_at DESC",
            Self::QR_COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::quote_request_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            let q = Self::finish_qr(row?)?;
            if self.qr_visible(&conn, caller, &q)? {
                out.push(q);
            }
        }
        Ok(out)
    }

    /// Turn a request into a reviewable draft contract. Administrator only
    /// (enforced by the route); pricing comes solely from `input`.
    pub fn generate_quote(
        &self,
        actor: &str,
        request_id: &str,
        input: QuoteInput,
    ) -> Result<Contract> {
        let conn = self.lock()?;
        let req = self
            .load_quote_request(&conn, request_id)?
            .ok_or(CommercialError::NotFound)?;
        if req.status != "submitted" {
            return Err(CommercialError::Conflict(format!(
                "quote request is already '{}'",
                req.status
            )));
        }
        let offering = catalog::find(&req.offering)
            .ok_or_else(|| CommercialError::Invalid("offering no longer in catalog".into()))?;
        let org_id = input
            .org_id
            .clone()
            .or_else(|| req.org_id.clone())
            .ok_or_else(|| CommercialError::Invalid("org_id is required".into()))?;
        org_exists(&conn, &org_id)?;

        let currency = input.currency.trim().to_string();
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(CommercialError::Invalid(
                "currency must be a 3-letter ISO 4217 code".into(),
            ));
        }
        let pricing_unit = bounded("pricing_unit", &input.pricing_unit, 100, true)?;
        if input.unit_price_minor.is_some_and(|p| p < 0) {
            return Err(CommercialError::Invalid(
                "unit_price_minor must be >= 0".into(),
            ));
        }
        if !(1..=120).contains(&input.duration_months) {
            return Err(CommercialError::Invalid(
                "duration_months must be 1-120".into(),
            ));
        }
        let support_hours = if input.support_hours.trim().is_empty() {
            offering.support_hours.to_string()
        } else {
            bounded("support_hours", &input.support_hours, 500, false)?
        };
        if offering.grants_coverage && input.response_targets.iter().all(|t| t.trim().is_empty()) {
            return Err(CommercialError::Invalid(
                "response_targets are required for coverage offerings".into(),
            ));
        }

        let mut exclusions: Vec<String> =
            offering.exclusions.iter().map(|s| s.to_string()).collect();
        exclusions.extend(
            input
                .extra_exclusions
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );

        let quote = Quote {
            offering: offering.id.to_string(),
            services_covered: offering
                .included_scope
                .iter()
                .map(|s| s.to_string())
                .collect(),
            infrastructure_covered: format!(
                "{} cluster(s), {} worker node(s), region: {}",
                req.cluster_count,
                req.worker_node_count,
                if req.region.is_empty() {
                    "unspecified"
                } else {
                    &req.region
                }
            ),
            included_capacity: input.included_capacity.trim().to_string(),
            pricing_unit,
            unit_price_minor: input.unit_price_minor,
            currency,
            duration_months: input.duration_months,
            response_targets: input
                .response_targets
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            support_hours,
            customer_responsibilities: offering
                .customer_responsibilities
                .iter()
                .map(|s| s.to_string())
                .collect(),
            zyvor_responsibilities: offering
                .zyvor_responsibilities
                .iter()
                .map(|s| s.to_string())
                .collect(),
            exclusions,
            required_integrations: offering
                .required_integrations
                .iter()
                .map(|s| s.to_string())
                .collect(),
            node_treatment: input.node_treatment.trim().to_string(),
        };
        let entitlement = Entitlement {
            support_tier: if offering.grants_coverage {
                offering.id.to_string()
            } else {
                String::new()
            },
            ..Default::default()
        };

        let now = Utc::now();
        let id = new_id("ctr");
        conn.execute(
            "INSERT INTO contracts (id, org_id, quote_request_id, offering, status, payment_status,
                source, quote, entitlement, created_at, updated_at)
             VALUES (?1,?2,?3,?4,'draft','not_invoiced','quote',?5,?6,?7,?7)",
            params![
                id,
                org_id,
                req.id,
                offering.id,
                serde_json::to_string(&quote).map_err(anyhow::Error::from)?,
                serde_json::to_string(&entitlement).map_err(anyhow::Error::from)?,
                ts(now)
            ],
        )?;
        conn.execute(
            "UPDATE quote_requests SET status = 'quoted', org_id = COALESCE(org_id, ?2), updated_at = ?3 WHERE id = ?1",
            params![req.id, org_id, ts(now)],
        )?;
        record(
            &conn,
            &id,
            actor,
            "quote_generated",
            json!({ "quote_request_id": req.id }),
        )?;
        load_contract(&conn, &id, now)?.ok_or(CommercialError::NotFound)
    }

    // ── contracts ────────────────────────────────────────────────

    pub fn get_contract(&self, caller: &Caller, id: &str) -> Result<Contract> {
        let conn = self.lock()?;
        let c = load_contract(&conn, id, Utc::now())?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(&conn, caller, &c.org_id)? {
            return Err(CommercialError::NotFound);
        }
        Ok(c)
    }

    pub fn list_contracts(&self, caller: &Caller) -> Result<Vec<Contract>> {
        let conn = self.lock()?;
        let now = Utc::now();
        let mut stmt = conn.prepare("SELECT id, org_id FROM contracts ORDER BY created_at DESC")?;
        let ids: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = Vec::new();
        for (id, org) in ids {
            if self.allowed(&conn, caller, &org)? {
                if let Some(c) = load_contract(&conn, &id, now)? {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    pub fn history(&self, caller: &Caller, id: &str) -> Result<Vec<ContractEvent>> {
        let conn = self.lock()?;
        let c = load_contract(&conn, id, Utc::now())?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(&conn, caller, &c.org_id)? {
            return Err(CommercialError::NotFound);
        }
        let mut stmt = conn.prepare(
            "SELECT at, actor, event, detail FROM contract_history WHERE contract_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([id], |r| {
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
            out.push(ContractEvent {
                at: parse_ts(&at)?,
                actor,
                event,
                detail: serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(out)
    }

    /// Replace the entitlement. Only drafts are editable so the terms a
    /// customer accepted cannot change underneath them; to revise a
    /// proposal, transition it back to `draft` first.
    pub fn update_entitlement(&self, actor: &str, id: &str, ent: Entitlement) -> Result<Contract> {
        ent.validate().map_err(CommercialError::Invalid)?;
        let conn = self.lock()?;
        let now = Utc::now();
        let c = load_contract(&conn, id, now)?.ok_or(CommercialError::NotFound)?;
        if c.status != ContractStatus::Draft {
            return Err(CommercialError::Conflict(
                "entitlement can only be edited while the contract is a draft".into(),
            ));
        }
        conn.execute(
            "UPDATE contracts SET entitlement = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&ent).map_err(anyhow::Error::from)?,
                ts(now)
            ],
        )?;
        record(
            &conn,
            id,
            actor,
            "entitlement_updated",
            json!({ "before": c.entitlement, "after": ent }),
        )?;
        load_contract(&conn, id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Administrator-driven state change.
    pub fn transition(&self, actor: &str, id: &str, to: ContractStatus) -> Result<Contract> {
        if to == ContractStatus::Active {
            return Err(CommercialError::Invalid(
                "activation happens when the customer accepts (or via offline import)".into(),
            ));
        }
        let conn = self.lock()?;
        self.transition_locked(&conn, actor, id, to, None)
    }

    /// Customer acceptance: `awaiting_acceptance → active`. Only a member of
    /// the contract's organization (or an administrator acting on their
    /// behalf) may accept.
    pub fn accept(&self, caller: &Caller, id: &str) -> Result<Contract> {
        let conn = self.lock()?;
        let c = load_contract(&conn, id, Utc::now())?.ok_or(CommercialError::NotFound)?;
        if !self.allowed(&conn, caller, &c.org_id)? {
            return Err(CommercialError::NotFound);
        }
        self.transition_locked(
            &conn,
            &caller.username,
            id,
            ContractStatus::Active,
            Some("accepted"),
        )
    }

    fn transition_locked(
        &self,
        conn: &Connection,
        actor: &str,
        id: &str,
        to: ContractStatus,
        event: Option<&str>,
    ) -> Result<Contract> {
        let _ = self;
        let now = Utc::now();
        let c = load_contract(conn, id, now)?.ok_or(CommercialError::NotFound)?;
        // Judge against the stored status; expiry is applied separately.
        if !c.status.can_transition_to(to) {
            return Err(CommercialError::Conflict(format!(
                "cannot move a contract from {} to {}",
                c.status.as_str(),
                to.as_str()
            )));
        }
        if to == ContractStatus::AwaitingAcceptance || to == ContractStatus::Active {
            let grants = catalog::find(&c.offering).is_some_and(|o| o.grants_coverage);
            c.entitlement
                .validate_for_activation(grants)
                .map_err(CommercialError::Invalid)?;
            if to == ContractStatus::Active && c.entitlement.expires_at.is_some_and(|e| e <= now) {
                return Err(CommercialError::Invalid(
                    "expires_at is already in the past".into(),
                ));
            }
        }
        conn.execute(
            "UPDATE contracts SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, to.as_str(), ts(now)],
        )?;
        record(
            conn,
            id,
            actor,
            event.unwrap_or("status_changed"),
            json!({ "from": c.status.as_str(), "to": to.as_str() }),
        )?;
        load_contract(conn, id, now)?.ok_or(CommercialError::NotFound)
    }

    pub fn set_payment_status(
        &self,
        actor: &str,
        id: &str,
        status: PaymentStatus,
    ) -> Result<Contract> {
        let conn = self.lock()?;
        let now = Utc::now();
        let c = load_contract(&conn, id, now)?.ok_or(CommercialError::NotFound)?;
        conn.execute(
            "UPDATE contracts SET payment_status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), ts(now)],
        )?;
        record(
            &conn,
            id,
            actor,
            "payment_status_changed",
            json!({ "from": c.payment_status.as_str(), "to": status.as_str() }),
        )?;
        load_contract(&conn, id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Offline import for disconnected deployments. Administrator only.
    /// The bundle is validated like any other contract, but its authenticity
    /// is not cryptographically verified in this phase.
    pub fn import_contract(&self, actor: &str, bundle: ImportBundle) -> Result<Contract> {
        let offering = catalog::find(&bundle.offering)
            .filter(|o| o.requestable)
            .ok_or_else(|| {
                CommercialError::Invalid("unknown or non-requestable offering".into())
            })?;
        let status = match bundle.status.as_deref().unwrap_or("draft") {
            "draft" => ContractStatus::Draft,
            "awaiting_acceptance" => ContractStatus::AwaitingAcceptance,
            "active" => ContractStatus::Active,
            _ => {
                return Err(CommercialError::Invalid(
                    "status must be draft, awaiting_acceptance or active".into(),
                ))
            }
        };
        bundle
            .entitlement
            .validate()
            .map_err(CommercialError::Invalid)?;
        if status != ContractStatus::Draft {
            bundle
                .entitlement
                .validate_for_activation(offering.grants_coverage)
                .map_err(CommercialError::Invalid)?;
        }
        let conn = self.lock()?;
        org_exists(&conn, &bundle.org_id)?;
        let now = Utc::now();
        let id = new_id("ctr");
        conn.execute(
            "INSERT INTO contracts (id, org_id, quote_request_id, offering, status, payment_status,
                source, quote, entitlement, created_at, updated_at)
             VALUES (?1,?2,NULL,?3,?4,'not_invoiced','offline_import',?5,?6,?7,?7)",
            params![
                id,
                bundle.org_id,
                offering.id,
                status.as_str(),
                serde_json::to_string(&bundle.quote).map_err(anyhow::Error::from)?,
                serde_json::to_string(&bundle.entitlement).map_err(anyhow::Error::from)?,
                ts(now)
            ],
        )?;
        record(
            &conn,
            &id,
            actor,
            "imported",
            json!({ "status": status.as_str() }),
        )?;
        load_contract(&conn, &id, now)?.ok_or(CommercialError::NotFound)
    }

    /// Persist expiry for active contracts whose end date has passed.
    /// Purely commercial bookkeeping: touches only this database.
    pub fn sweep_expired(&self) -> Result<usize> {
        let conn = self.lock()?;
        let now = Utc::now();
        let mut stmt = conn.prepare("SELECT id FROM contracts WHERE status = 'active'")?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        let mut n = 0;
        for id in ids {
            let Some(c) = load_contract(&conn, &id, now)? else {
                continue;
            };
            if c.effective_status == ContractStatus::Expired {
                conn.execute(
                    "UPDATE contracts SET status = 'expired', updated_at = ?2 WHERE id = ?1",
                    params![id, ts(now)],
                )?;
                record(
                    &conn,
                    &id,
                    "system",
                    "expired",
                    json!({ "expires_at": c.entitlement.expires_at }),
                )?;
                n += 1;
            }
        }
        Ok(n)
    }

    // ── coverage ─────────────────────────────────────────────────

    /// Coverage per cluster, visible contracts only. A cluster with no
    /// contract simply has no entry: absence means "not covered", never an
    /// error and never a reason to restrict anything.
    pub fn coverage(&self, caller: &Caller, cluster: Option<&str>) -> Result<Vec<CoverageEntry>> {
        let contracts = self.list_contracts(caller)?;
        let mut out = Vec::new();
        for c in contracts {
            if !catalog::find(&c.offering).is_some_and(|o| o.grants_coverage) {
                continue;
            }
            for cl in &c.entitlement.covered_clusters {
                if cluster.is_some_and(|w| w != cl) {
                    continue;
                }
                out.push(CoverageEntry {
                    cluster_id: cl.clone(),
                    contract_id: c.id.clone(),
                    org_id: c.org_id.clone(),
                    offering: c.offering.clone(),
                    support_tier: c.entitlement.support_tier.clone(),
                    status: c.effective_status,
                    eligible: c.effective_status == ContractStatus::Active,
                    effective_at: c.entitlement.effective_at,
                    expires_at: c.entitlement.expires_at,
                    timezone: c.entitlement.timezone.clone(),
                    coverage_hours: c.entitlement.coverage_hours.clone(),
                    node_allowance: c.entitlement.node_allowance,
                    managed_permissions: c.entitlement.managed_permissions.clone(),
                    renewal: c.renewal.clone(),
                });
            }
        }
        Ok(out)
    }
}

pub(super) fn org_exists(conn: &Connection, org_id: &str) -> Result<()> {
    let found: Option<String> = conn
        .query_row("SELECT id FROM orgs WHERE id = ?1", [org_id], |r| r.get(0))
        .optional()?;
    found.map(|_| ()).ok_or(CommercialError::NotFound)
}

pub(super) fn is_member(conn: &Connection, org_id: &str, username: &str) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM org_members WHERE org_id = ?1 AND username = ?2",
        params![org_id, username],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub(super) fn record(
    conn: &Connection,
    contract_id: &str,
    actor: &str,
    event: &str,
    detail: serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO contract_history (contract_id, at, actor, event, detail) VALUES (?1,?2,?3,?4,?5)",
        params![contract_id, ts(Utc::now()), actor, event, detail.to_string()],
    )?;
    Ok(())
}

pub(super) fn load_contract(
    conn: &Connection,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Option<Contract>> {
    let row = conn
        .query_row(
            "SELECT id, org_id, quote_request_id, offering, status, payment_status, source,
                    quote, entitlement, created_at, updated_at
             FROM contracts WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                ))
            },
        )
        .optional()?;
    let Some((id, org_id, qr, offering, status, pay, source, quote, ent, created, updated)) = row
    else {
        return Ok(None);
    };
    let status = ContractStatus::parse(&status)
        .ok_or_else(|| anyhow::anyhow!("corrupt contract status '{status}'"))?;
    let entitlement: Entitlement = serde_json::from_str(&ent).map_err(anyhow::Error::from)?;
    let effective_status =
        if status == ContractStatus::Active && entitlement.expires_at.is_some_and(|e| e <= now) {
            ContractStatus::Expired
        } else {
            status
        };
    let renewal = (effective_status == ContractStatus::Expired).then(|| {
        "This contract has expired. Commercial support eligibility has lapsed; your VMs, \
         VM APIs and data are unaffected. Contact your Zyvor representative to renew."
            .to_string()
    });
    Ok(Some(Contract {
        id,
        org_id,
        quote_request_id: qr,
        offering,
        status,
        effective_status,
        payment_status: PaymentStatus::parse(&pay)
            .ok_or_else(|| anyhow::anyhow!("corrupt payment status '{pay}'"))?,
        source,
        quote: serde_json::from_str(&quote).map_err(anyhow::Error::from)?,
        entitlement,
        created_at: parse_ts(&created)?,
        updated_at: parse_ts(&updated)?,
        renewal,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn admin() -> Caller {
        Caller {
            username: "root".into(),
            admin: true,
        }
    }
    fn user(n: &str) -> Caller {
        Caller {
            username: n.into(),
            admin: false,
        }
    }

    fn request(org: Option<&str>) -> QuoteRequestInput {
        QuoteRequestInput {
            org_id: org.map(str::to_string),
            offering: "supported".into(),
            company: "Acme".into(),
            contact_name: "Ann".into(),
            contact_email: "ann@acme.test".into(),
            cluster_count: 2,
            worker_node_count: 12,
            workload_size: "300 VMs".into(),
            region: "eu".into(),
            desired_coverage: "business hours".into(),
            requirements: String::new(),
        }
    }

    fn quote_input() -> QuoteInput {
        QuoteInput {
            org_id: None,
            currency: "EUR".into(),
            pricing_unit: "worker node / year".into(),
            unit_price_minor: Some(100),
            included_capacity: "12 worker nodes".into(),
            duration_months: 12,
            response_targets: vec!["Sev1: 1 business hour".into()],
            support_hours: String::new(),
            extra_exclusions: vec![],
            node_treatment: "control-plane nodes are not billed".into(),
        }
    }

    fn entitlement(cluster: &str, expires_in_days: i64) -> Entitlement {
        Entitlement {
            covered_clusters: vec![cluster.into()],
            support_tier: "supported".into(),
            coverage_hours: CoverageHours {
                always: false,
                days: vec![1, 2, 3, 4, 5],
                start: "09:00".into(),
                end: "17:00".into(),
            },
            timezone: "Europe/Berlin".into(),
            authorized_contacts: vec![SupportContact {
                name: "Ann".into(),
                email: "ann@acme.test".into(),
            }],
            effective_at: Some(Utc::now() - Duration::days(1)),
            expires_at: Some(Utc::now() + Duration::days(expires_in_days)),
            node_allowance: Some(12),
            managed_permissions: vec![],
            ..Default::default()
        }
    }

    /// Org + member + submitted request quoted into a draft contract.
    fn draft(store: &CommercialStore, org_name: &str, member: &str, cluster: &str) -> Contract {
        let org = store.create_org(org_name, Some(member)).unwrap();
        let req = store
            .create_quote_request(&user(member), request(Some(&org.id)))
            .unwrap();
        let c = store
            .generate_quote("root", &req.id, quote_input())
            .unwrap();
        store
            .update_entitlement("root", &c.id, entitlement(cluster, 365))
            .unwrap()
    }

    #[test]
    fn full_lifecycle_and_history() {
        let s = CommercialStore::open(":memory:").unwrap();
        let c = draft(&s, "Acme", "ann", "cl-1");
        assert_eq!(c.status, ContractStatus::Draft);
        s.transition("root", &c.id, ContractStatus::AwaitingAcceptance)
            .unwrap();
        let c = s.accept(&user("ann"), &c.id).unwrap();
        assert_eq!(c.effective_status, ContractStatus::Active);
        let events: Vec<_> = s
            .history(&user("ann"), &c.id)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        assert_eq!(
            events,
            [
                "quote_generated",
                "entitlement_updated",
                "status_changed",
                "accepted"
            ]
        );
    }

    #[test]
    fn organizations_cannot_read_each_other() {
        let s = CommercialStore::open(":memory:").unwrap();
        let a = draft(&s, "Acme", "ann", "cl-a");
        let b = draft(&s, "Beta", "bob", "cl-b");
        assert!(matches!(
            s.get_contract(&user("bob"), &a.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.history(&user("bob"), &a.id),
            Err(CommercialError::NotFound)
        ));
        assert!(matches!(
            s.accept(&user("bob"), &a.id),
            Err(CommercialError::NotFound)
        ));
        let ids: Vec<_> = s
            .list_contracts(&user("bob"))
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec![b.id.clone()]);
        assert_eq!(s.list_contracts(&admin()).unwrap().len(), 2);
        // Coverage is isolated too.
        assert!(s.coverage(&user("bob"), Some("cl-a")).unwrap().is_empty());
        // A stranger cannot file a request into someone else's org.
        let org_a = a.org_id.clone();
        assert!(matches!(
            s.create_quote_request(&user("bob"), request(Some(&org_a))),
            Err(CommercialError::NotFound)
        ));
        // Quote requests are isolated as well.
        assert!(s
            .list_quote_requests(&user("bob"))
            .unwrap()
            .iter()
            .all(|q| q.org_id.as_deref() != Some(&org_a)));
    }

    #[test]
    fn invalid_transitions_rejected() {
        let s = CommercialStore::open(":memory:").unwrap();
        let c = draft(&s, "Acme", "ann", "cl-1");
        // Can't jump to active, or accept a draft.
        assert!(s.transition("root", &c.id, ContractStatus::Active).is_err());
        assert!(matches!(
            s.accept(&user("ann"), &c.id),
            Err(CommercialError::Conflict(_))
        ));
        s.transition("root", &c.id, ContractStatus::Cancelled)
            .unwrap();
        assert!(matches!(
            s.transition("root", &c.id, ContractStatus::Draft),
            Err(CommercialError::Conflict(_))
        ));
        // Accepted terms are not editable.
        let c2 = draft(&s, "Beta", "bob", "cl-2");
        s.transition("root", &c2.id, ContractStatus::AwaitingAcceptance)
            .unwrap();
        assert!(matches!(
            s.update_entitlement("root", &c2.id, entitlement("cl-x", 10)),
            Err(CommercialError::Conflict(_))
        ));
    }

    #[test]
    fn activation_needs_complete_entitlement() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = s.create_org("Acme", Some("ann")).unwrap();
        let req = s
            .create_quote_request(&user("ann"), request(Some(&org.id)))
            .unwrap();
        let c = s.generate_quote("root", &req.id, quote_input()).unwrap();
        // No dates/clusters yet.
        assert!(matches!(
            s.transition("root", &c.id, ContractStatus::AwaitingAcceptance),
            Err(CommercialError::Invalid(_))
        ));
    }

    #[test]
    fn expiry_changes_eligibility_only() {
        let s = CommercialStore::open(":memory:").unwrap();
        let c = draft(&s, "Acme", "ann", "cl-1");
        s.transition("root", &c.id, ContractStatus::AwaitingAcceptance)
            .unwrap();
        s.accept(&user("ann"), &c.id).unwrap();
        // Force the end date into the past directly in storage.
        {
            let conn = s.conn.lock().unwrap();
            let mut e = entitlement("cl-1", 365);
            e.expires_at = Some(Utc::now() - Duration::hours(1));
            conn.execute(
                "UPDATE contracts SET entitlement = ?2 WHERE id = ?1",
                params![c.id, serde_json::to_string(&e).unwrap()],
            )
            .unwrap();
        }
        let cov = s.coverage(&user("ann"), Some("cl-1")).unwrap();
        assert_eq!(cov.len(), 1);
        assert!(!cov[0].eligible);
        assert_eq!(cov[0].status, ContractStatus::Expired);
        assert!(cov[0].renewal.as_deref().unwrap().contains("unaffected"));
        // Data stays readable after expiry.
        assert!(s.get_contract(&user("ann"), &c.id).is_ok());
        assert_eq!(s.sweep_expired().unwrap(), 1);
        assert_eq!(s.sweep_expired().unwrap(), 0);
        assert_eq!(
            s.get_contract(&user("ann"), &c.id).unwrap().status,
            ContractStatus::Expired
        );
    }

    #[test]
    fn uncovered_cluster_has_no_entry() {
        let s = CommercialStore::open(":memory:").unwrap();
        assert!(s.coverage(&admin(), Some("nope")).unwrap().is_empty());
    }

    #[test]
    fn quote_validation_and_no_default_prices() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = s.create_org("Acme", Some("ann")).unwrap();
        let req = s
            .create_quote_request(&user("ann"), request(Some(&org.id)))
            .unwrap();
        let mut bad = quote_input();
        bad.currency = "eur".into();
        assert!(s.generate_quote("root", &req.id, bad).is_err());
        let mut no_targets = quote_input();
        no_targets.response_targets.clear();
        assert!(s.generate_quote("root", &req.id, no_targets).is_err());
        let mut unpriced = quote_input();
        unpriced.unit_price_minor = None;
        let c = s.generate_quote("root", &req.id, unpriced).unwrap();
        assert_eq!(c.quote.unit_price_minor, None);
        // A request can be quoted once.
        assert!(matches!(
            s.generate_quote("root", &req.id, quote_input()),
            Err(CommercialError::Conflict(_))
        ));
        // Unknown offerings can't be requested.
        let mut r = request(None);
        r.offering = "community".into();
        assert!(s.create_quote_request(&user("ann"), r).is_err());
    }

    #[test]
    fn payment_status_is_independent() {
        let s = CommercialStore::open(":memory:").unwrap();
        let c = draft(&s, "Acme", "ann", "cl-1");
        let c = s
            .set_payment_status("root", &c.id, PaymentStatus::Paid)
            .unwrap();
        assert_eq!(c.payment_status, PaymentStatus::Paid);
        assert_eq!(c.status, ContractStatus::Draft);
    }

    #[test]
    fn offline_import_and_newer_schema_guard() {
        let s = CommercialStore::open(":memory:").unwrap();
        let org = s.create_org("Acme", None).unwrap();
        let c = draft(&s, "Other", "x", "cl-9");
        let bundle = ImportBundle {
            org_id: org.id.clone(),
            offering: "supported".into(),
            quote: c.quote.clone(),
            entitlement: entitlement("cl-off", 100),
            status: Some("active".into()),
        };
        let imported = s.import_contract("root", bundle).unwrap();
        assert_eq!(imported.source, "offline_import");
        assert_eq!(imported.effective_status, ContractStatus::Active);

        let conn = Connection::open_in_memory().unwrap();
        CommercialStore::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations VALUES (999, 'future', 'now')",
            [],
        )
        .unwrap();
        assert!(CommercialStore::migrate(&conn).is_err());
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        CommercialStore::migrate(&conn).unwrap();
        CommercialStore::migrate(&conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, MIGRATIONS.len() as i64);
    }
}
