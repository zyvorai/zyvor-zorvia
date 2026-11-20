// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Audit Trail - Comprehensive audit event tracking with optional SQLite persistence,
//! or PostgreSQL (`ZORVIA_DATABASE_URL`) so every replica writes to and reads from one trail.

#[cfg(feature = "web")]
use crate::store::{Backend, Row};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "web")]
use std::path::Path;

use crate::utils::generate_id;

#[derive(Debug)]
pub struct AuditTrail {
    pub entries: Vec<AuditEntry>,
    pub max_entries: usize,
    /// When set, every `record` is also written to SQLite or PostgreSQL (web builds only).
    #[cfg(feature = "web")]
    db: Option<Backend>,
    /// When the in-memory window was last read from a shared database.
    #[cfg(feature = "web")]
    refreshed: Option<std::time::Instant>,
}

impl Clone for AuditTrail {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            max_entries: self.max_entries,
            #[cfg(feature = "web")]
            db: None,
            #[cfg(feature = "web")]
            refreshed: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub user: String,
    pub action: AuditAction,
    pub resource_type: String,
    pub resource_name: String,
    pub namespace: String,
    pub details: Value,
    pub ip_address: String,
    pub success: bool,
    pub severity: AuditSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditAction {
    Create,
    Update,
    Delete,
    Start,
    Stop,
    Restart,
    Migrate,
    Clone,
    Snapshot,
    Restore,
    ConfigChange,
    Login,
    Logout,
    Export,
    Import,
    ScaleUp,
    ScaleDown,
    Approve,
    Reject,
    ScheduleChange,
    Exec,
    ViewLogs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum AuditSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl AuditTrail {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
            #[cfg(feature = "web")]
            db: None,
            #[cfg(feature = "web")]
            refreshed: None,
        }
    }

    /// Prefer `ZORVIA_AUDIT_DB`, else alongside auth DB as `audit.db`, else memory-only.
    pub fn from_env() -> Self {
        #[cfg(feature = "web")]
        {
            if let Some(url) = crate::store::database_url() {
                return match Self::open_postgres(&url, 10000) {
                    Ok(mut t) => {
                        if std::env::var("ZORVIA_DATABASE_IMPORT").as_deref() == Ok("1") {
                            if let Some(p) = Self::sqlite_path_from_env() {
                                match t.import_from_sqlite(&p) {
                                    Ok(0) => {}
                                    Ok(n) => log::info!("imported {n} audit entries from {p}"),
                                    Err(e) => log::warn!("audit import from {p} failed: {e}"),
                                }
                            }
                        }
                        t
                    }
                    Err(e) => {
                        log::warn!("Audit PostgreSQL unavailable ({e}); using in-memory trail");
                        Self::default()
                    }
                };
            }
            let path = std::env::var("ZORVIA_AUDIT_DB").ok().or_else(|| {
                std::env::var("ZORVIA_AUTH_DB").ok().map(|auth| {
                    let p = std::path::PathBuf::from(auth);
                    p.parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join("audit.db")
                        .to_string_lossy()
                        .to_string()
                })
            });
            if let Some(p) = path {
                return match Self::open_persistent(&p, 10000) {
                    Ok(t) => t,
                    Err(e) => {
                        log::warn!("Audit persistence unavailable ({e}); using in-memory trail");
                        Self::default()
                    }
                };
            }
        }
        Self::default()
    }

    #[cfg(feature = "web")]
    pub fn open_persistent(path: impl AsRef<Path>, max_entries: usize) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let db = Backend::sqlite(rusqlite::Connection::open(path)?);
        db.batch(
            "CREATE TABLE IF NOT EXISTS audit_entries (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                user_name TEXT NOT NULL,
                action TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                namespace TEXT NOT NULL,
                details TEXT NOT NULL,
                ip_address TEXT NOT NULL,
                success INTEGER NOT NULL,
                severity TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_entries(timestamp);",
        )?;
        let mut trail = Self {
            entries: Vec::new(),
            max_entries,
            db: Some(db),
            refreshed: None,
        };
        trail.reload_from_db()?;
        log::info!(
            "Audit trail: persistent store at {} ({} entries loaded)",
            path.display(),
            trail.entries.len()
        );
        Ok(trail)
    }

    #[cfg(feature = "web")]
    fn sqlite_path_from_env() -> Option<String> {
        let p = std::env::var("ZORVIA_AUDIT_DB").ok().or_else(|| {
            std::env::var("ZORVIA_AUTH_DB").ok().map(|auth| {
                std::path::PathBuf::from(auth)
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("audit.db")
                    .to_string_lossy()
                    .to_string()
            })
        })?;
        Path::new(&p).exists().then_some(p)
    }

    /// One-time copy of a SQLite audit database into an empty shared store. Entries
    /// already present (same id) are kept; returns how many were added.
    #[cfg(feature = "web")]
    pub fn import_from_sqlite(&mut self, path: &str) -> anyhow::Result<usize> {
        let Some(db) = &self.db else { return Ok(0) };
        let existing = db
            .query_one("SELECT COUNT(*) FROM audit_entries", &[])?
            .map(|r| r.int(0))
            .unwrap_or(0);
        if existing > 0 {
            return Ok(0);
        }
        let src = Backend::sqlite(rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?);
        let rows = src.query(
            "SELECT id, timestamp, user_name, action, resource_type, resource_name, namespace,
                    details, ip_address, success, severity FROM audit_entries",
            &[],
        )?;
        let mut n = 0;
        for r in &rows {
            n += db.exec(
                "INSERT INTO audit_entries
                 (id, timestamp, user_name, action, resource_type, resource_name, namespace,
                  details, ip_address, success, severity)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT (id) DO NOTHING",
                &(0..11)
                    .map(|i| match i {
                        9 => r.int(9).into(),
                        _ => r.opt_text(i).unwrap_or_default().into(),
                    })
                    .collect::<Vec<crate::store::Value>>(),
            )? as usize;
        }
        self.reload_from_db()?;
        Ok(n)
    }

    /// The trail in PostgreSQL, shared by every replica.
    #[cfg(feature = "web")]
    pub fn open_postgres(url: &str, max_entries: usize) -> anyhow::Result<Self> {
        let db = Backend::postgres(url)?;
        db.batch(
            "SELECT pg_advisory_xact_lock(727272003);
            CREATE TABLE IF NOT EXISTS audit_entries (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                user_name TEXT NOT NULL,
                action TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_name TEXT NOT NULL,
                namespace TEXT NOT NULL,
                details TEXT NOT NULL,
                ip_address TEXT NOT NULL,
                success BIGINT NOT NULL,
                severity TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_entries(timestamp);",
        )?;
        let mut trail = Self {
            entries: Vec::new(),
            max_entries,
            db: Some(db),
            refreshed: None,
        };
        trail.reload_from_db()?;
        log::info!(
            "Audit trail: PostgreSQL ({} entries loaded)",
            trail.entries.len()
        );
        Ok(trail)
    }

    /// True when the trail lives in a database other replicas also write to.
    #[cfg(feature = "web")]
    pub fn is_shared(&self) -> bool {
        self.db.as_ref().is_some_and(|d| d.is_postgres())
    }

    /// Re-read the newest entries from a shared database (at most once a second), so a
    /// replica shows what the others recorded. A no-op for SQLite and memory.
    #[cfg(feature = "web")]
    pub fn refresh_shared(&mut self) {
        if !self.is_shared() {
            return;
        }
        if self
            .refreshed
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        match self.reload_from_db() {
            Ok(()) => self.refreshed = Some(std::time::Instant::now()),
            Err(e) => log::warn!("audit: cannot refresh from the shared store: {e}"),
        }
    }

    #[cfg(feature = "web")]
    fn reload_from_db(&mut self) -> anyhow::Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };
        let rows = db.query(
            "SELECT id, timestamp, user_name, action, resource_type, resource_name, namespace,
                    details, ip_address, success, severity
             FROM audit_entries ORDER BY timestamp DESC, id DESC LIMIT ?1",
            &[(self.max_entries as i64).into()],
        )?;
        let mut entries: Vec<AuditEntry> = rows.iter().map(row_to_entry).collect();
        entries.reverse();
        self.entries = entries;
        Ok(())
    }

    pub fn record(&mut self, entry: AuditEntry) {
        #[cfg(feature = "web")]
        if let Some(db) = &self.db {
            if let Err(e) = db.exec(
                "INSERT INTO audit_entries
                 (id, timestamp, user_name, action, resource_type, resource_name, namespace,
                  details, ip_address, success, severity)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT (id) DO NOTHING",
                &[
                    entry.id.clone().into(),
                    entry.timestamp.to_rfc3339().into(),
                    entry.user.clone().into(),
                    action_to_str(&entry.action).into(),
                    entry.resource_type.clone().into(),
                    entry.resource_name.clone().into(),
                    entry.namespace.clone().into(),
                    entry.details.to_string().into(),
                    entry.ip_address.clone().into(),
                    entry.success.into(),
                    severity_to_str(&entry.severity).into(),
                ],
            ) {
                log::warn!("audit: cannot persist entry {}: {e}", entry.id);
            }
        }
        self.entries.push(entry.clone());
        #[cfg(feature = "web")]
        append_jsonl_sidecar(&entry);
        if self.entries.len() > self.max_entries {
            let excess = self.entries.len().saturating_sub(self.max_entries);
            if excess > 0 {
                self.entries.drain(0..excess);
            }
        }
    }

    /// Serialize matching entries as newline-delimited JSON (newest first).
    pub fn export_jsonl(
        &self,
        user: Option<&str>,
        resource_type: Option<&str>,
        limit: usize,
    ) -> String {
        let mut lines = Vec::new();
        for e in self.entries.iter().rev() {
            if user.is_some_and(|u| e.user != u) {
                continue;
            }
            if resource_type.is_some_and(|t| e.resource_type != t) {
                continue;
            }
            if let Ok(line) = serde_json::to_string(e) {
                lines.push(line);
            }
            if lines.len() >= limit {
                break;
            }
        }
        lines.join("\n")
    }

    pub fn log_action(
        &mut self,
        user: &str,
        action: AuditAction,
        resource_type: &str,
        resource_name: &str,
        namespace: &str,
        success: bool,
    ) {
        let severity = match &action {
            AuditAction::Delete | AuditAction::Migrate | AuditAction::Exec => AuditSeverity::High,
            AuditAction::Create | AuditAction::Update | AuditAction::ConfigChange => {
                AuditSeverity::Medium
            }
            AuditAction::Start | AuditAction::Stop | AuditAction::Restart => AuditSeverity::Low,
            _ => AuditSeverity::Info,
        };
        self.record(AuditEntry {
            id: generate_id("audit", resource_name),
            timestamp: Utc::now(),
            user: user.to_string(),
            action,
            resource_type: resource_type.to_string(),
            resource_name: resource_name.to_string(),
            namespace: namespace.to_string(),
            details: Value::Null,
            ip_address: "127.0.0.1".to_string(),
            success,
            severity,
        });
    }

    pub fn query(
        &self,
        user: Option<&str>,
        action: Option<&AuditAction>,
        namespace: Option<&str>,
        min_severity: Option<&AuditSeverity>,
    ) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| {
                user.is_none_or(|u| e.user == u)
                    && action.is_none_or(|a| e.action == *a)
                    && namespace.is_none_or(|ns| e.namespace == ns)
                    && min_severity.is_none_or(|s| e.severity >= *s)
            })
            .collect()
    }

    pub fn recent(&self, limit: usize) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn by_resource(&self, resource_name: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.resource_name == resource_name)
            .collect()
    }

    pub fn failed_actions(&self) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| !e.success).collect()
    }

    pub fn stats(&self) -> AuditStats {
        AuditStats {
            total: self.entries.len(),
            successful: self.entries.iter().filter(|e| e.success).count(),
            failed: self.entries.iter().filter(|e| !e.success).count(),
            critical: self
                .entries
                .iter()
                .filter(|e| e.severity == AuditSeverity::Critical)
                .count(),
            high: self
                .entries
                .iter()
                .filter(|e| e.severity == AuditSeverity::High)
                .count(),
        }
    }
}

/// Read guard on the shared trail, refreshed first when the trail lives in a database
/// other replicas write to.
#[cfg(feature = "web")]
pub async fn read_fresh(
    audit: &std::sync::Arc<tokio::sync::RwLock<AuditTrail>>,
) -> tokio::sync::RwLockReadGuard<'_, AuditTrail> {
    if audit.read().await.is_shared() {
        audit.write().await.refresh_shared();
    }
    audit.read().await
}

#[derive(Debug, Clone)]
pub struct AuditStats {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub critical: usize,
    pub high: usize,
}

impl Default for AuditTrail {
    fn default() -> Self {
        Self::new(10000)
    }
}

#[cfg(feature = "web")]
#[cfg(feature = "web")]
fn row_to_entry(row: &Row) -> AuditEntry {
    AuditEntry {
        id: row.opt_text(0).unwrap_or_default(),
        timestamp: row
            .opt_text(1)
            .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        user: row.opt_text(2).unwrap_or_default(),
        action: action_from_str(&row.opt_text(3).unwrap_or_default()),
        resource_type: row.opt_text(4).unwrap_or_default(),
        resource_name: row.opt_text(5).unwrap_or_default(),
        namespace: row.opt_text(6).unwrap_or_default(),
        details: row
            .opt_text(7)
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or(Value::Null),
        ip_address: row.opt_text(8).unwrap_or_default(),
        success: row.int(9) != 0,
        severity: severity_from_str(&row.opt_text(10).unwrap_or_default()),
    }
}

fn action_to_str(a: &AuditAction) -> &'static str {
    match a {
        AuditAction::Create => "Create",
        AuditAction::Update => "Update",
        AuditAction::Delete => "Delete",
        AuditAction::Start => "Start",
        AuditAction::Stop => "Stop",
        AuditAction::Restart => "Restart",
        AuditAction::Migrate => "Migrate",
        AuditAction::Clone => "Clone",
        AuditAction::Snapshot => "Snapshot",
        AuditAction::Restore => "Restore",
        AuditAction::ConfigChange => "ConfigChange",
        AuditAction::Login => "Login",
        AuditAction::Logout => "Logout",
        AuditAction::Export => "Export",
        AuditAction::Import => "Import",
        AuditAction::ScaleUp => "ScaleUp",
        AuditAction::ScaleDown => "ScaleDown",
        AuditAction::Approve => "Approve",
        AuditAction::Reject => "Reject",
        AuditAction::ScheduleChange => "ScheduleChange",
        AuditAction::Exec => "Exec",
        AuditAction::ViewLogs => "ViewLogs",
    }
}

#[cfg(feature = "web")]
fn action_from_str(s: &str) -> AuditAction {
    match s {
        "Create" => AuditAction::Create,
        "Update" => AuditAction::Update,
        "Delete" => AuditAction::Delete,
        "Start" => AuditAction::Start,
        "Stop" => AuditAction::Stop,
        "Restart" => AuditAction::Restart,
        "Migrate" => AuditAction::Migrate,
        "Clone" => AuditAction::Clone,
        "Snapshot" => AuditAction::Snapshot,
        "Restore" => AuditAction::Restore,
        "ConfigChange" => AuditAction::ConfigChange,
        "Login" => AuditAction::Login,
        "Logout" => AuditAction::Logout,
        "Export" => AuditAction::Export,
        "Import" => AuditAction::Import,
        "ScaleUp" => AuditAction::ScaleUp,
        "ScaleDown" => AuditAction::ScaleDown,
        "Approve" => AuditAction::Approve,
        "Reject" => AuditAction::Reject,
        "ScheduleChange" => AuditAction::ScheduleChange,
        "Exec" => AuditAction::Exec,
        "ViewLogs" => AuditAction::ViewLogs,
        _ => AuditAction::Update,
    }
}

#[cfg(feature = "web")]
fn severity_to_str(s: &AuditSeverity) -> &'static str {
    match s {
        AuditSeverity::Info => "Info",
        AuditSeverity::Low => "Low",
        AuditSeverity::Medium => "Medium",
        AuditSeverity::High => "High",
        AuditSeverity::Critical => "Critical",
    }
}

#[cfg(feature = "web")]
fn severity_from_str(s: &str) -> AuditSeverity {
    match s {
        "Low" => AuditSeverity::Low,
        "Medium" => AuditSeverity::Medium,
        "High" => AuditSeverity::High,
        "Critical" => AuditSeverity::Critical,
        _ => AuditSeverity::Info,
    }
}

/// Optional append-only JSONL sidecar (`ZORVIA_AUDIT_JSONL=/path/audit.jsonl`).
#[cfg(feature = "web")]
fn append_jsonl_sidecar(entry: &AuditEntry) {
    let Ok(path) = std::env::var("ZORVIA_AUDIT_JSONL") else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    let Ok(line) = serde_json::to_string(entry) else {
        return;
    };
    use std::io::Write;
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(all(test, feature = "web"))]
mod tests {
    use super::*;

    #[test]
    fn persistent_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let mut trail = AuditTrail::open_persistent(&path, 100).unwrap();
            trail.log_action("admin", AuditAction::Login, "auth", "login", "ns", true);
            assert_eq!(trail.entries.len(), 1);
        }
        let trail2 = AuditTrail::open_persistent(&path, 100).unwrap();
        assert_eq!(trail2.entries.len(), 1);
        assert_eq!(trail2.entries[0].user, "admin");
        assert_eq!(trail2.entries[0].action, AuditAction::Login);
    }

    #[test]
    fn export_jsonl_newest_first() {
        let mut trail = AuditTrail::new(100);
        trail.log_action("a", AuditAction::Login, "auth", "1", "ns", true);
        trail.log_action("b", AuditAction::Create, "vm", "vm1", "ns", true);
        let out = trail.export_jsonl(None, None, 10);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"user\":\"b\""));
        assert!(lines[1].contains("\"user\":\"a\""));
        let filtered = trail.export_jsonl(Some("b"), Some("vm"), 10);
        assert_eq!(filtered.lines().count(), 1);
    }

    /// Two replicas on one PostgreSQL see each other's entries after a refresh, and an
    /// old SQLite trail can be imported once. Needs `ZORVIA_TEST_POSTGRES_URL`.
    #[test]
    fn postgres_trail_is_shared_between_replicas() {
        let Ok(url) = std::env::var("ZORVIA_TEST_POSTGRES_URL") else {
            eprintln!("skipped: ZORVIA_TEST_POSTGRES_URL is not set");
            return;
        };
        let reset = || {
            let t = AuditTrail::open_postgres(&url, 100).unwrap();
            t.db.as_ref()
                .unwrap()
                .batch("DROP TABLE IF EXISTS audit_entries")
                .unwrap();
        };
        reset();
        let mut a = AuditTrail::open_postgres(&url, 100).unwrap();
        let mut b = AuditTrail::open_postgres(&url, 100).unwrap();
        assert!(a.is_shared());
        a.log_action("alice", AuditAction::Login, "auth", "login", "ns", true);
        b.log_action("bob", AuditAction::Delete, "vm", "vm1", "ns", false);
        assert_eq!(a.entries.len(), 1);
        a.refresh_shared();
        b.refresh_shared();
        for t in [&a, &b] {
            let users: Vec<_> = t.entries.iter().map(|e| e.user.as_str()).collect();
            assert_eq!(users, ["alice", "bob"]);
            assert!(t.entries[1].severity == AuditSeverity::High && !t.entries[1].success);
        }
        // The window is capped like the SQLite one.
        let mut small = AuditTrail::open_postgres(&url, 1).unwrap();
        assert_eq!(small.entries.len(), 1);
        assert_eq!(small.entries[0].user, "bob");
        small.refresh_shared();

        // Import from an existing SQLite trail, once.
        reset();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let mut old = AuditTrail::open_persistent(&path, 100).unwrap();
            old.log_action("old-admin", AuditAction::Create, "vm", "legacy", "ns", true);
        }
        let mut fresh = AuditTrail::open_postgres(&url, 100).unwrap();
        assert_eq!(fresh.import_from_sqlite(path.to_str().unwrap()).unwrap(), 1);
        assert_eq!(fresh.import_from_sqlite(path.to_str().unwrap()).unwrap(), 0);
        assert_eq!(fresh.entries[0].resource_name, "legacy");
        reset();
    }
}
