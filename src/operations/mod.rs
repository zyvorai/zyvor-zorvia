// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Durable long-running operations.
//!
//! Work such as image capture, backups and migrations used to live in
//! in-memory registries and lose all state on restart. `OperationsDb` persists
//! each operation (state, phase, progress, params, result) in SQLite (or PostgreSQL,
//! `ZORVIA_DATABASE_URL`, so replicas share one queue) so a
//! reconciler can resume it after a restart, honour cancellation, and stop
//! retrying after `max_attempts`. A restart of the process is not a failed
//! attempt: interrupted work is re-queued with its attempt refunded, bounded by
//! `MAX_INTERRUPTIONS` so an operation that keeps killing the process still ends.
//!
//! State machine:
//! `queued -> running -> succeeded | failed | cancelled`, with
//! `running -> queued` on a retryable failure or after a restart.

use crate::store::{Backend, Row, Value};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub mod runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl OpState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub kind: String,
    pub resource: String,
    pub namespace: String,
    pub state: OpState,
    pub phase: String,
    pub progress: u8,
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub attempts: u32,
    pub max_attempts: u32,
    /// Times this operation was interrupted by a restart or lost owner (these
    /// do not count against `max_attempts`).
    pub interruptions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub cancel_requested: bool,
    pub created: String,
    pub updated: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
}

/// Input for `OperationsDb::create`.
#[derive(Debug, Clone)]
pub struct NewOperation {
    pub kind: String,
    pub resource: String,
    pub namespace: String,
    pub params: serde_json::Value,
    pub max_attempts: u32,
    /// A second `create` with the same key returns the existing operation
    /// instead of starting duplicate work.
    pub idempotency_key: Option<String>,
    pub owner: Option<String>,
}

impl NewOperation {
    pub fn new(kind: &str, resource: &str, namespace: &str) -> Self {
        Self {
            kind: kind.to_string(),
            resource: resource.to_string(),
            namespace: namespace.to_string(),
            params: serde_json::Value::Null,
            max_attempts: 3,
            idempotency_key: None,
            owner: None,
        }
    }
}

impl OperationsDb {
    /// Process-wide store. Falls back to an in-memory DB (with a warning)
    /// if the file cannot be opened, so the server still starts.
    pub fn global() -> &'static std::sync::Arc<OperationsDb> {
        static DB: std::sync::OnceLock<std::sync::Arc<OperationsDb>> = std::sync::OnceLock::new();
        DB.get_or_init(|| {
            std::sync::Arc::new(Self::from_env().unwrap_or_else(|e| {
                log::warn!("operations DB unavailable ({e}); using in-memory store");
                Self::open(":memory:").expect("in-memory sqlite")
            }))
        })
    }
}

/// Where the operations database lives. `ZORVIA_OPS_DB` wins; otherwise it sits
/// next to the auth database (like the audit trail), so a deployment that puts
/// `ZORVIA_AUTH_DB` on a persistent volume keeps operations across restarts;
/// otherwise the per-user data directory. (Without this, a container restart
/// silently discarded every operation.)
pub fn ops_db_path(get: impl Fn(&str) -> Option<String>) -> String {
    if let Some(p) = get("ZORVIA_OPS_DB").filter(|p| !p.trim().is_empty()) {
        return p;
    }
    if let Some(auth) = get("ZORVIA_AUTH_DB").filter(|p| !p.trim().is_empty()) {
        if let Some(dir) = std::path::Path::new(&auth).parent() {
            if dir != std::path::Path::new("") {
                return dir.join("ops.db").to_string_lossy().to_string();
            }
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("zorvia")
        .join("ops.db")
        .to_string_lossy()
        .to_string()
}

pub struct OperationsDb {
    db: Backend,
}

/// How many times one operation may be interrupted (process restart or lost
/// owner) before it is failed instead of re-queued.
pub const MAX_INTERRUPTIONS: u32 = 10;

const COLS: &str = "id, kind, resource, namespace, state, phase, progress, params, result, \
                    error, attempts, max_attempts, idempotency_key, owner, cancel_requested, \
                    created, updated, completed, interruptions";

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn row_to_op(r: &Row) -> Operation {
    Operation {
        id: r.opt_text(0).unwrap_or_default(),
        kind: r.opt_text(1).unwrap_or_default(),
        resource: r.opt_text(2).unwrap_or_default(),
        namespace: r.opt_text(3).unwrap_or_default(),
        state: OpState::parse(&r.opt_text(4).unwrap_or_default()),
        phase: r.opt_text(5).unwrap_or_default(),
        progress: r.int(6).clamp(0, 100) as u8,
        params: r
            .opt_text(7)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null),
        result: r.opt_text(8).and_then(|s| serde_json::from_str(&s).ok()),
        error: r.opt_text(9),
        attempts: r.int(10).max(0) as u32,
        max_attempts: r.int(11).max(1) as u32,
        idempotency_key: r.opt_text(12),
        owner: r.opt_text(13),
        cancel_requested: r.int(14) != 0,
        created: r.opt_text(15).unwrap_or_default(),
        updated: r.opt_text(16).unwrap_or_default(),
        completed: r.opt_text(17),
        interruptions: r.int(18).max(0) as u32,
    }
}

const PG_SCHEMA: &str = "
SELECT pg_advisory_xact_lock(727272002);
CREATE TABLE IF NOT EXISTS operations (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    resource TEXT NOT NULL,
    namespace TEXT NOT NULL,
    state TEXT NOT NULL,
    phase TEXT NOT NULL DEFAULT '',
    progress BIGINT NOT NULL DEFAULT 0,
    params TEXT NOT NULL DEFAULT 'null',
    result TEXT,
    error TEXT,
    attempts BIGINT NOT NULL DEFAULT 0,
    max_attempts BIGINT NOT NULL DEFAULT 3,
    idempotency_key TEXT,
    owner TEXT,
    cancel_requested BIGINT NOT NULL DEFAULT 0,
    created TEXT NOT NULL,
    updated TEXT NOT NULL,
    completed TEXT,
    interruptions BIGINT NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_operations_idem
    ON operations(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_operations_state ON operations(state);";

/// A running operation whose heartbeat is older than this belongs to a dead process.
/// Used by startup recovery on a shared (PostgreSQL) store, where another replica's
/// healthy work must not be re-queued.
const SHARED_RECOVERY_STALE_SECS: i64 = 90;

impl OperationsDb {
    pub fn open(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            rusqlite::Connection::open_in_memory()?
        } else {
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
            rusqlite::Connection::open(path)?
        };
        let db = Backend::sqlite(conn);
        db.batch(
            "CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                resource TEXT NOT NULL,
                namespace TEXT NOT NULL,
                state TEXT NOT NULL,
                phase TEXT NOT NULL DEFAULT '',
                progress INTEGER NOT NULL DEFAULT 0,
                params TEXT NOT NULL DEFAULT 'null',
                result TEXT,
                error TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                idempotency_key TEXT,
                owner TEXT,
                cancel_requested INTEGER NOT NULL DEFAULT 0,
                created TEXT NOT NULL,
                updated TEXT NOT NULL,
                completed TEXT,
                interruptions INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_operations_idem
                ON operations(idempotency_key) WHERE idempotency_key IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_operations_state ON operations(state);",
        )?;
        // Added after the table above shipped: existing databases need the
        // column; a duplicate-column error on new ones is expected and ignored.
        let _ =
            db.batch("ALTER TABLE operations ADD COLUMN interruptions INTEGER NOT NULL DEFAULT 0;");
        Ok(Self { db })
    }

    pub fn open_postgres(url: &str) -> Result<Self> {
        let db = Backend::postgres(url)?;
        db.batch(PG_SCHEMA)?;
        Ok(Self { db })
    }

    pub fn is_postgres(&self) -> bool {
        self.db.is_postgres()
    }

    pub fn from_env() -> Result<Self> {
        match crate::store::database_url() {
            Some(url) => {
                let db = Self::open_postgres(&url)?;
                log::info!("operations store: PostgreSQL");
                Ok(db)
            }
            None => Self::open(&ops_db_path(|k| std::env::var(k).ok())),
        }
    }

    /// Stable newest-first ordering; SQLite breaks ties by insertion order.
    fn newest_first(&self) -> &'static str {
        if self.db.is_postgres() {
            "created DESC, id DESC"
        } else {
            "created DESC, rowid DESC"
        }
    }

    fn oldest_first(&self) -> &'static str {
        if self.db.is_postgres() {
            "created ASC, id ASC"
        } else {
            "created ASC, rowid ASC"
        }
    }

    fn fetch(&self, id: &str) -> Result<Option<Operation>> {
        Ok(self
            .db
            .query_one(
                &format!("SELECT {COLS} FROM operations WHERE id = ?1"),
                &[id.into()],
            )?
            .map(|r| row_to_op(&r)))
    }

    /// Create an operation. Returns `(operation, created)`; `created` is
    /// false when `idempotency_key` matched an existing operation. The key is
    /// claimed by the insert itself, so two replicas racing on one key still
    /// produce a single operation.
    pub fn create(&self, new: NewOperation) -> Result<(Operation, bool)> {
        let id = format!("op-{}", uuid::Uuid::new_v4().simple());
        let ts = now();
        let n = self
            .db
            .exec(
                "INSERT INTO operations (id, kind, resource, namespace, state, params, \
                 max_attempts, idempotency_key, owner, created, updated) \
                 VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?9) \
                 ON CONFLICT (idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
                &[
                    id.clone().into(),
                    new.kind.into(),
                    new.resource.into(),
                    new.namespace.into(),
                    new.params.to_string().into(),
                    new.max_attempts.max(1).into(),
                    new.idempotency_key.clone().into(),
                    new.owner.into(),
                    ts.into(),
                ],
            )
            .context("insert operation")?;
        if n == 0 {
            let key = new
                .idempotency_key
                .ok_or_else(|| anyhow!("operation was not inserted"))?;
            let existing = self
                .db
                .query_one(
                    &format!("SELECT {COLS} FROM operations WHERE idempotency_key = ?1"),
                    &[key.into()],
                )?
                .map(|r| row_to_op(&r))
                .ok_or_else(|| anyhow!("idempotent operation vanished"))?;
            return Ok((existing, false));
        }
        let op = self
            .fetch(&id)?
            .ok_or_else(|| anyhow!("operation {id} vanished after insert"))?;
        Ok((op, true))
    }

    pub fn get(&self, id: &str) -> Result<Option<Operation>> {
        self.fetch(id)
    }

    /// Newest first, optionally filtered by kind.
    pub fn list(&self, kind: Option<&str>, limit: usize) -> Result<Vec<Operation>> {
        let limit = limit.clamp(1, 500) as i64;
        let order = self.newest_first();
        let rows = match kind {
            Some(k) => self.db.query(
                &format!("SELECT {COLS} FROM operations WHERE kind = ?1 ORDER BY {order} LIMIT ?2"),
                &[k.into(), limit.into()],
            )?,
            None => self.db.query(
                &format!("SELECT {COLS} FROM operations ORDER BY {order} LIMIT ?1"),
                &[limit.into()],
            )?,
        };
        Ok(rows.iter().map(row_to_op).collect())
    }

    /// Operations the reconciler should still act on, oldest first.
    pub fn list_active(&self) -> Result<Vec<Operation>> {
        let rows = self.db.query(
            &format!(
                "SELECT {COLS} FROM operations WHERE state IN ('queued','running') ORDER BY {}",
                self.oldest_first()
            ),
            &[],
        )?;
        Ok(rows.iter().map(row_to_op).collect())
    }

    /// queued -> running, counting the attempt. Returns false if the
    /// operation was not queued (already claimed, cancelled, or finished).
    pub fn start(&self, id: &str) -> Result<bool> {
        let n = self.db.exec(
            "UPDATE operations SET state='running', attempts=attempts+1, updated=?2 \
             WHERE id=?1 AND state='queued'",
            &[id.into(), now().into()],
        )?;
        Ok(n == 1)
    }

    /// Record phase/progress of a running operation (progress is capped at
    /// 99 -- 100 is only set by `succeed`).
    pub fn set_progress(&self, id: &str, phase: &str, progress: u8) -> Result<()> {
        self.db.exec(
            "UPDATE operations SET phase=?2, progress=?3, updated=?4 \
             WHERE id=?1 AND state='running'",
            &[
                id.into(),
                phase.into(),
                (progress.min(99) as i64).into(),
                now().into(),
            ],
        )?;
        Ok(())
    }

    pub fn succeed(&self, id: &str, result: Option<&serde_json::Value>) -> Result<()> {
        self.db.exec(
            "UPDATE operations SET state='succeeded', progress=100, result=?2, error=NULL, \
             updated=?3, completed=?3 WHERE id=?1 AND state='running'",
            &[
                id.into(),
                result.map(|r| r.to_string()).into(),
                now().into(),
            ],
        )?;
        Ok(())
    }

    /// Record a failure. When `retryable` and attempts remain the operation
    /// goes back to `queued`; otherwise it becomes `failed`. Returns the
    /// resulting state.
    pub fn fail(&self, id: &str, error: &str, retryable: bool) -> Result<OpState> {
        let row = self
            .db
            .query_one(
                "SELECT attempts, max_attempts, cancel_requested FROM operations WHERE id=?1",
                &[id.into()],
            )?
            .ok_or_else(|| anyhow!("operation {id} not found"))?;
        let (attempts, max, cancel) = (row.int(0), row.int(1), row.int(2));
        let ts = now();
        if retryable && cancel == 0 && attempts < max {
            self.db.exec(
                "UPDATE operations SET state='queued', error=?2, updated=?3 \
                 WHERE id=?1 AND state='running'",
                &[id.into(), error.into(), ts.into()],
            )?;
            Ok(OpState::Queued)
        } else {
            self.db.exec(
                "UPDATE operations SET state='failed', error=?2, updated=?3, completed=?3 \
                 WHERE id=?1 AND state IN ('running','queued')",
                &[id.into(), error.into(), ts.into()],
            )?;
            Ok(OpState::Failed)
        }
    }

    /// Ask for cancellation. A queued operation is cancelled immediately; a
    /// running one is flagged so its handler can stop and call
    /// `mark_cancelled`. Returns the resulting state, or `None` if unknown.
    pub fn request_cancel(&self, id: &str) -> Result<Option<OpState>> {
        let ts = now();
        self.db.exec(
            "UPDATE operations SET state='cancelled', cancel_requested=1, updated=?2, \
             completed=?2 WHERE id=?1 AND state='queued'",
            &[id.into(), ts.clone().into()],
        )?;
        self.db.exec(
            "UPDATE operations SET cancel_requested=1, updated=?2 \
             WHERE id=?1 AND state='running'",
            &[id.into(), ts.into()],
        )?;
        Ok(self
            .db
            .query_one("SELECT state FROM operations WHERE id=?1", &[id.into()])?
            .and_then(|r| r.opt_text(0))
            .map(|s| OpState::parse(&s)))
    }

    pub fn mark_cancelled(&self, id: &str) -> Result<()> {
        let ts = now();
        self.db.exec(
            "UPDATE operations SET state='cancelled', updated=?2, completed=?2 \
             WHERE id=?1 AND state IN ('running','queued')",
            &[id.into(), ts.into()],
        )?;
        Ok(())
    }

    /// Heartbeat from the process running an operation, so other replicas
    /// can tell it apart from one orphaned by a dead leader.
    pub fn touch(&self, id: &str) -> Result<()> {
        self.db.exec(
            "UPDATE operations SET updated=?2 WHERE id=?1 AND state='running'",
            &[id.into(), now().into()],
        )?;
        Ok(())
    }

    /// running -> queued for an operation whose owner stopped heartbeating.
    /// The interrupted attempt is refunded (a lost owner is not a failed
    /// attempt); after `MAX_INTERRUPTIONS` the operation is failed instead.
    pub fn requeue_orphan(&self, id: &str) -> Result<OpState> {
        let ts = now();
        self.db.exec(
            "UPDATE operations SET state='failed', updated=?2, completed=?2, \
             error='owner stopped responding too many times' \
             WHERE id=?1 AND state='running' AND interruptions >= ?3",
            &[id.into(), ts.clone().into(), MAX_INTERRUPTIONS.into()],
        )?;
        self.db.exec(
            "UPDATE operations SET state='queued', updated=?2, \
             attempts=CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
             interruptions=interruptions+1 \
             WHERE id=?1 AND state='running'",
            &[id.into(), ts.into()],
        )?;
        let state = self
            .db
            .query_one("SELECT state FROM operations WHERE id=?1", &[id.into()])?
            .and_then(|r| r.opt_text(0))
            .ok_or_else(|| anyhow!("operation {id} not found"))?;
        Ok(OpState::parse(&state))
    }

    /// Startup recovery: anything left `running` by a previous process is
    /// re-queued with its attempt refunded (a restart is not a failure), unless
    /// it was being cancelled or has been interrupted `MAX_INTERRUPTIONS`
    /// times already. Returns how many were touched.
    ///
    /// On a shared PostgreSQL store only operations whose heartbeat has stopped
    /// are touched: a replica that starts (or takes the lease) while another one
    /// is still running its work must not steal it.
    pub fn recover_interrupted(&self) -> Result<usize> {
        let ts = now();
        let (stale, cutoff) = if self.db.is_postgres() {
            (
                " AND updated < ?3",
                (Utc::now() - chrono::Duration::seconds(SHARED_RECOVERY_STALE_SECS)).to_rfc3339(),
            )
        } else {
            ("", String::new())
        };
        let bind = |extra: Option<Value>| -> Vec<Value> {
            let mut v = vec![Value::from(ts.clone())];
            if let Some(e) = extra {
                v.push(e);
            }
            v
        };
        // Parameter numbering: ?1 timestamp, ?2 interruption cap or cutoff, ?3 cutoff.
        let cutoff_v = || {
            if self.db.is_postgres() {
                Some(Value::from(cutoff.clone()))
            } else {
                None
            }
        };
        let cancelled = {
            let sql = format!(
                "UPDATE operations SET state='cancelled', updated=?1, completed=?1 \
                 WHERE state='running' AND cancel_requested=1{}",
                stale.replace("?3", "?2")
            );
            self.db.exec(&sql, &bind(cutoff_v()))?
        };
        let failed = {
            let sql = format!(
                "UPDATE operations SET state='failed', updated=?1, completed=?1, \
                 error='interrupted by restart too many times' \
                 WHERE state='running' AND interruptions >= ?2{stale}"
            );
            let mut p = bind(Some(MAX_INTERRUPTIONS.into()));
            p.extend(cutoff_v());
            self.db.exec(&sql, &p)?
        };
        let requeued = {
            let sql = format!(
                "UPDATE operations SET state='queued', updated=?1, \
                 attempts=CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                 interruptions=interruptions+1 \
                 WHERE state='running'{}",
                stale.replace("?3", "?2")
            );
            self.db.exec(&sql, &bind(cutoff_v()))?
        };
        Ok((cancelled + failed + requeued) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> OperationsDb {
        OperationsDb::open(":memory:").unwrap()
    }

    fn new_op(key: Option<&str>) -> NewOperation {
        let mut n = NewOperation::new("golden-image", "vm-a", "default");
        n.idempotency_key = key.map(String::from);
        n.max_attempts = 2;
        n
    }

    #[test]
    fn lifecycle_reaches_succeeded_with_result() {
        let db = db();
        let (op, created) = db.create(new_op(None)).unwrap();
        assert!(created);
        assert_eq!(op.state, OpState::Queued);
        assert!(db.start(&op.id).unwrap());
        db.set_progress(&op.id, "cloning", 250).unwrap();
        let mid = db.get(&op.id).unwrap().unwrap();
        assert_eq!(
            (mid.state, mid.progress, mid.attempts),
            (OpState::Running, 99, 1)
        );
        db.succeed(&op.id, Some(&serde_json::json!({"dv": "img"})))
            .unwrap();
        let done = db.get(&op.id).unwrap().unwrap();
        assert_eq!(done.state, OpState::Succeeded);
        assert_eq!(done.progress, 100);
        assert_eq!(done.result.unwrap()["dv"], "img");
        assert!(done.completed.is_some());
    }

    #[test]
    fn idempotency_key_dedupes() {
        let db = db();
        let (a, created_a) = db.create(new_op(Some("k1"))).unwrap();
        let (b, created_b) = db.create(new_op(Some("k1"))).unwrap();
        assert!(created_a && !created_b);
        assert_eq!(a.id, b.id);
        let (c, _) = db.create(new_op(Some("k2"))).unwrap();
        assert_ne!(a.id, c.id);
        assert_eq!(db.list(None, 10).unwrap().len(), 2);
    }

    #[test]
    fn start_only_claims_queued_operations_once() {
        let db = db();
        let (op, _) = db.create(new_op(None)).unwrap();
        assert!(db.start(&op.id).unwrap());
        assert!(!db.start(&op.id).unwrap());
    }

    #[test]
    fn retry_limit_ends_in_failed() {
        let db = db();
        let (op, _) = db.create(new_op(None)).unwrap();
        db.start(&op.id).unwrap();
        assert_eq!(db.fail(&op.id, "boom", true).unwrap(), OpState::Queued);
        db.start(&op.id).unwrap();
        // attempts (2) == max_attempts (2): no more retries.
        assert_eq!(
            db.fail(&op.id, "boom again", true).unwrap(),
            OpState::Failed
        );
        let f = db.get(&op.id).unwrap().unwrap();
        assert_eq!(f.error.as_deref(), Some("boom again"));
        assert!(f.completed.is_some());
    }

    #[test]
    fn non_retryable_failure_is_final() {
        let db = db();
        let (op, _) = db.create(new_op(None)).unwrap();
        db.start(&op.id).unwrap();
        assert_eq!(
            db.fail(&op.id, "bad input", false).unwrap(),
            OpState::Failed
        );
    }

    #[test]
    fn cancel_queued_is_immediate_and_running_is_flagged() {
        let db = db();
        let (q, _) = db.create(new_op(None)).unwrap();
        assert_eq!(db.request_cancel(&q.id).unwrap(), Some(OpState::Cancelled));
        assert!(!db.start(&q.id).unwrap());

        let (r, _) = db.create(new_op(None)).unwrap();
        db.start(&r.id).unwrap();
        assert_eq!(db.request_cancel(&r.id).unwrap(), Some(OpState::Running));
        assert!(db.get(&r.id).unwrap().unwrap().cancel_requested);
        // A cancelled-while-running op must not be retried.
        assert_eq!(db.fail(&r.id, "stopped", true).unwrap(), OpState::Failed);
        assert_eq!(db.request_cancel("missing").unwrap(), None);
    }

    #[test]
    fn a_restart_requeues_running_work_without_using_an_attempt() {
        let db = db();
        let (a, _) = db.create(new_op(None)).unwrap();
        db.start(&a.id).unwrap(); // attempts 1 of 2
        let (b, _) = db.create(new_op(None)).unwrap();
        db.start(&b.id).unwrap();
        db.fail(&b.id, "x", true).unwrap();
        db.start(&b.id).unwrap(); // attempts 2 of 2: its own failures used the budget
        let (c, _) = db.create(new_op(None)).unwrap();
        db.start(&c.id).unwrap();
        db.request_cancel(&c.id).unwrap();

        assert_eq!(db.recover_interrupted().unwrap(), 3);
        let a = db.get(&a.id).unwrap().unwrap();
        assert_eq!(
            (a.state, a.attempts, a.interruptions),
            (OpState::Queued, 0, 1)
        );
        // b was on its last attempt, but the restart is not its failure: it
        // is re-queued with the attempt refunded.
        let b = db.get(&b.id).unwrap().unwrap();
        assert_eq!(
            (b.state, b.attempts, b.interruptions),
            (OpState::Queued, 1, 1)
        );
        assert_eq!(db.get(&c.id).unwrap().unwrap().state, OpState::Cancelled);
        assert_eq!(db.list_active().unwrap().len(), 2);
    }

    #[test]
    fn many_restarts_never_exhaust_the_retry_budget_but_are_bounded() {
        let db = db();
        let (op, _) = db.create(new_op(None)).unwrap(); // max_attempts 2
        for i in 0..MAX_INTERRUPTIONS {
            assert!(db.start(&op.id).unwrap());
            db.recover_interrupted().unwrap();
            let o = db.get(&op.id).unwrap().unwrap();
            assert_eq!(o.state, OpState::Queued, "restart {i} must re-queue");
            assert_eq!(o.attempts, 0, "restart {i} must not consume an attempt");
        }
        // One more interruption than the bound: the operation ends.
        assert!(db.start(&op.id).unwrap());
        db.recover_interrupted().unwrap();
        let o = db.get(&op.id).unwrap().unwrap();
        assert_eq!(o.state, OpState::Failed);
        assert!(o.error.unwrap().contains("too many times"));
    }

    #[test]
    fn orphan_requeue_refunds_the_attempt_and_is_bounded() {
        let db = db();
        let (op, _) = db.create(new_op(None)).unwrap();
        db.start(&op.id).unwrap();
        db.touch(&op.id).unwrap();
        assert_eq!(db.requeue_orphan(&op.id).unwrap(), OpState::Queued);
        assert_eq!(db.get(&op.id).unwrap().unwrap().attempts, 0);
        for _ in 1..MAX_INTERRUPTIONS {
            db.start(&op.id).unwrap();
            assert_eq!(db.requeue_orphan(&op.id).unwrap(), OpState::Queued);
        }
        db.start(&op.id).unwrap();
        assert_eq!(db.requeue_orphan(&op.id).unwrap(), OpState::Failed);
    }

    #[test]
    fn ops_db_path_prefers_explicit_then_sits_next_to_the_auth_db() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            ops_db_path(env(&[
                ("ZORVIA_OPS_DB", "/x/o.db"),
                ("ZORVIA_AUTH_DB", "/data/auth.db")
            ])),
            "/x/o.db"
        );
        assert_eq!(
            ops_db_path(env(&[("ZORVIA_AUTH_DB", "/data/auth.db")])),
            "/data/ops.db"
        );
        // A bare file name has no directory to reuse: fall back to the data dir.
        assert!(ops_db_path(env(&[("ZORVIA_AUTH_DB", "auth.db")])).ends_with("zorvia/ops.db"));
        assert!(ops_db_path(env(&[])).ends_with("zorvia/ops.db"));
    }

    #[test]
    fn state_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("zorvia-ops-{}", uuid::Uuid::new_v4()));
        let path = dir.join("ops.db").to_string_lossy().to_string();
        let id = {
            let db = OperationsDb::open(&path).unwrap();
            let (op, _) = db.create(new_op(Some("persist"))).unwrap();
            db.start(&op.id).unwrap();
            op.id
        };
        let db = OperationsDb::open(&path).unwrap();
        assert_eq!(db.get(&id).unwrap().unwrap().state, OpState::Running);
        db.recover_interrupted().unwrap();
        assert_eq!(db.get(&id).unwrap().unwrap().state, OpState::Queued);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Behaviour every backend must share: the idempotency key is claimed atomically
    /// by concurrent creators, only one claimer wins an operation, and ordering is stable.
    fn shared_queue_behaviour(make: &dyn Fn() -> OperationsDb) {
        let a = std::sync::Arc::new(make());
        let b = std::sync::Arc::new(make());
        // Two replicas creating the same idempotent operation at once: one op.
        let mut handles = Vec::new();
        for i in 0..8 {
            let db = if i % 2 == 0 { a.clone() } else { b.clone() };
            handles.push(std::thread::spawn(move || {
                db.create(new_op(Some("race-key"))).unwrap()
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
        let id = results[0].0.id.clone();
        assert!(results.iter().all(|(op, _)| op.id == id));
        // Two replicas claiming it: exactly one wins.
        let mut claims = Vec::new();
        for i in 0..8 {
            let db = if i % 2 == 0 { a.clone() } else { b.clone() };
            let id = id.clone();
            claims.push(std::thread::spawn(move || db.start(&id).unwrap()));
        }
        let won = claims.into_iter().map(|h| h.join().unwrap());
        assert_eq!(won.filter(|w| *w).count(), 1);
        assert_eq!(a.get(&id).unwrap().unwrap().attempts, 1);
        // The other replica sees progress and the result.
        a.set_progress(&id, "copy", 40).unwrap();
        assert_eq!(b.get(&id).unwrap().unwrap().progress, 40);
        b.succeed(&id, Some(&serde_json::json!({"ok": true})))
            .unwrap();
        let done = a.get(&id).unwrap().unwrap();
        assert_eq!(done.state, OpState::Succeeded);
        assert_eq!(done.result.unwrap()["ok"], true);
        // Newest first, kind filter, active list.
        let (x, _) = a.create(new_op(None)).unwrap();
        let (y, _) = b.create(new_op(None)).unwrap();
        let listed = a.list(Some("golden-image"), 10).unwrap();
        assert_eq!(listed[0].id, y.id);
        assert_eq!(listed[1].id, x.id);
        assert!(a.list(Some("nope"), 10).unwrap().is_empty());
        let active: Vec<_> = a.list_active().unwrap().into_iter().map(|o| o.id).collect();
        assert_eq!(active, vec![x.id.clone(), y.id.clone()]);
        // Retry accounting and orphan refund are visible to both.
        assert!(a.start(&x.id).unwrap());
        assert_eq!(b.fail(&x.id, "boom", true).unwrap(), OpState::Queued);
        assert!(b.start(&x.id).unwrap());
        assert_eq!(a.fail(&x.id, "boom", true).unwrap(), OpState::Failed);
        assert_eq!(a.request_cancel(&y.id).unwrap(), Some(OpState::Cancelled));
        assert_eq!(a.request_cancel("op-missing").unwrap(), None);
    }

    #[test]
    fn sqlite_shared_queue_behaviour() {
        let dir = std::env::temp_dir().join(format!("zorvia-ops-{}", uuid::Uuid::new_v4()));
        let path = dir.join("ops.db").to_string_lossy().to_string();
        // Two handles on one file stand in for two processes.
        shared_queue_behaviour(&|| OperationsDb::open(&path).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    /// Needs a throwaway PostgreSQL: `ZORVIA_TEST_POSTGRES_URL=postgres://...`.
    #[test]
    fn postgres_shared_queue_behaviour_and_stale_recovery() {
        let Ok(url) = std::env::var("ZORVIA_TEST_POSTGRES_URL") else {
            eprintln!("skipped: ZORVIA_TEST_POSTGRES_URL is not set");
            return;
        };
        let reset = || {
            let db = OperationsDb::open_postgres(&url).unwrap();
            db.db.batch("DROP TABLE IF EXISTS operations").unwrap();
        };
        reset();
        assert!(OperationsDb::open_postgres(&url).unwrap().is_postgres());
        shared_queue_behaviour(&|| OperationsDb::open_postgres(&url).unwrap());

        // Startup recovery on a shared store leaves a live replica's work alone and
        // re-queues work whose heartbeat stopped.
        reset();
        let db = OperationsDb::open_postgres(&url).unwrap();
        let (live, _) = db.create(new_op(None)).unwrap();
        let (dead, _) = db.create(new_op(None)).unwrap();
        db.start(&live.id).unwrap();
        db.start(&dead.id).unwrap();
        let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
        db.db
            .exec(
                "UPDATE operations SET updated=?2 WHERE id=?1",
                &[dead.id.as_str().into(), old.into()],
            )
            .unwrap();
        assert_eq!(db.recover_interrupted().unwrap(), 1);
        assert_eq!(db.get(&live.id).unwrap().unwrap().state, OpState::Running);
        let d = db.get(&dead.id).unwrap().unwrap();
        assert_eq!(
            (d.state, d.attempts, d.interruptions),
            (OpState::Queued, 0, 1)
        );
        reset();
    }
}
