// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Durable DR replication pairs on top of the off-cluster S3 backup/restore code.
//!
//! A pair says "keep a verified off-cluster backup of VM X no older than RPO
//! seconds, so it can be restored as `X-dr`". The scheduler loop calls [`tick`]
//! for every pair: it polls the in-flight backup operation, records last-success
//! and lag, and queues the next verified backup when the RPO interval has elapsed.
//! Failover is a restore of the last verified backup as the DR VM; it never
//! overwrites an existing VM and refuses while the source VM may be running.
//!
//! Everything touching the cluster goes through the [`BackupBackend`] and
//! [`RestoreBackend`] traits so the logic is tested with fakes. Pairs persist via
//! `crate::state_docs` (a file, or PostgreSQL when `ZORVIA_DATABASE_URL` is set).
//!
//! This is a same-cluster object-store replication: data moves through the
//! configured S3 target, not between sites, and nothing here repoints traffic.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MIN_RPO_SECS: u64 = 60;
pub const MAX_RPO_SECS: u64 = 7 * 24 * 3600;
const ALERT_RETENTION_SECS: u64 = 7 * 24 * 3600;

/// Env flag required (together with `ZORVIA_EXPERIMENTAL=1`) to use this feature.
pub const FLAG: &str = "ZORVIA_FEATURE_DR_REPLICATION";

pub fn enabled() -> bool {
    crate::features::enterprise_flag(FLAG)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Replicating,
    FailedOver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    #[default]
    Ok,
    Breached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverRecord {
    pub requested_at: DateTime<Utc>,
    pub backup_op: String,
    pub restore_op: String,
    pub dr_vm: String,
    pub dr_namespace: String,
    /// `pending`, `succeeded` or `failed`.
    pub state: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrPair {
    pub name: String,
    pub vm: String,
    pub namespace: String,
    pub dr_namespace: String,
    pub dr_vm: String,
    pub rpo_seconds: u64,
    pub rto_seconds: u64,
    pub enabled: bool,
    pub phase: Phase,
    pub created: DateTime<Utc>,
    /// Lag is measured from here until the first verified backup (reset on resume).
    pub epoch: DateTime<Utc>,
    #[serde(default)]
    pub in_flight_op: Option<String>,
    #[serde(default)]
    pub last_attempt: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_success: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_success_op: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub alert_state: AlertState,
    #[serde(default)]
    pub failover: Option<FailoverRecord>,
}

fn label_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && crate::backup::restore::dns_label(s)
}

impl DrPair {
    pub fn new(
        name: &str,
        vm: &str,
        namespace: &str,
        dr_namespace: &str,
        rpo_seconds: u64,
        rto_seconds: u64,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        for (what, v) in [
            ("name", name),
            ("vm", vm),
            ("namespace", namespace),
            ("dr_namespace", dr_namespace),
        ] {
            if !label_ok(v) {
                return Err(format!(
                    "{what} must be a DNS label (lowercase letters, digits, '-', max 63)"
                ));
            }
        }
        let dr_vm = format!("{vm}-dr");
        if !label_ok(&dr_vm) {
            return Err("vm name too long: '<vm>-dr' must fit in 63 characters".into());
        }
        if !(MIN_RPO_SECS..=MAX_RPO_SECS).contains(&rpo_seconds) {
            return Err(format!(
                "rpo_seconds must be between {MIN_RPO_SECS} and {MAX_RPO_SECS}"
            ));
        }
        if rto_seconds == 0 || rto_seconds > 30 * 24 * 3600 {
            return Err("rto_seconds must be between 1 and 2592000".into());
        }
        Ok(Self {
            name: name.into(),
            vm: vm.into(),
            namespace: namespace.into(),
            dr_namespace: dr_namespace.into(),
            dr_vm,
            rpo_seconds,
            rto_seconds,
            enabled: true,
            phase: Phase::Replicating,
            created: now,
            epoch: now,
            in_flight_op: None,
            last_attempt: None,
            last_success: None,
            last_success_op: None,
            last_error: None,
            consecutive_failures: 0,
            alert_state: AlertState::Ok,
            failover: None,
        })
    }

    /// When the next backup attempt is due (first attempt: immediately).
    pub fn due_at(&self) -> DateTime<Utc> {
        match self.last_attempt {
            Some(t) => t + Duration::seconds(self.rpo_seconds as i64),
            None => self.epoch,
        }
    }
}

// ---------------------------------------------------------------- store

/// All pairs, persisted as one JSON document (file or PostgreSQL via `state_docs`).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PairStore {
    pub pairs: Vec<DrPair>,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl PairStore {
    pub fn default_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("zorvia")
            .join("dr_pairs.json")
    }

    pub fn load_from(path: &Path) -> Self {
        if let Some(c) = crate::state_docs::read(path) {
            match serde_json::from_str(&c) {
                Ok(s) => return s,
                Err(e) => log::warn!("Failed to parse DR pairs: {e}"),
            }
        }
        Self::default()
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        crate::state_docs::write(path, &serde_json::to_string_pretty(self)?)
    }

    /// Load, mutate and save under a process-wide lock so the scheduler loop and
    /// API handlers do not lose each other's updates within one replica.
    pub fn update<T>(
        path: &Path,
        f: impl FnOnce(&mut PairStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Self::load_from(path);
        let out = f(&mut s)?;
        s.save_to(path)
            .map_err(|e| format!("persist failed: {e}"))?;
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Option<&DrPair> {
        self.pairs.iter().find(|p| p.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut DrPair> {
        self.pairs.iter_mut().find(|p| p.name == name)
    }

    pub fn add(&mut self, pair: DrPair) -> Result<(), String> {
        if self.get(&pair.name).is_some() {
            return Err(format!("pair '{}' already exists", pair.name));
        }
        if self
            .pairs
            .iter()
            .any(|p| p.vm == pair.vm && p.namespace == pair.namespace)
        {
            return Err(format!(
                "VM {}/{} already has a replication pair",
                pair.namespace, pair.vm
            ));
        }
        if self
            .pairs
            .iter()
            .any(|p| p.dr_vm == pair.dr_vm && p.dr_namespace == pair.dr_namespace)
        {
            return Err(format!(
                "DR target {}/{} is already used by another pair",
                pair.dr_namespace, pair.dr_vm
            ));
        }
        self.pairs.push(pair);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.pairs.len();
        self.pairs.retain(|p| p.name != name);
        self.pairs.len() < n
    }
}

// ------------------------------------------------------------- backends

#[derive(Debug, Clone, PartialEq)]
pub enum OpProgress {
    Pending,
    Succeeded { completed: Option<DateTime<Utc>> },
    Failed(String),
}

/// Takes verified off-cluster backups. `backup_progress` reports `Succeeded` only
/// for a backup that was uploaded, read back and is restorable.
pub trait BackupBackend {
    fn ready(&self) -> Result<(), String>;
    fn enqueue_backup(&self, namespace: &str, vm: &str, key: &str) -> Result<String, String>;
    fn backup_progress(&self, op_id: &str) -> Result<OpProgress, String>;
}

pub trait RestoreBackend {
    fn enqueue_restore(
        &self,
        backup_op: &str,
        namespace: &str,
        new_vm: &str,
        start: bool,
    ) -> Result<String, String>;
    fn restore_progress(&self, op_id: &str) -> Result<OpProgress, String>;
}

/// The real backend: the durable operations queue and the off-cluster restore code.
pub struct OpsBackend {
    pub target: Result<(), String>,
}

fn parse_ts(s: &Option<String>) -> Option<DateTime<Utc>> {
    s.as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn op_progress(op: &crate::operations::Operation) -> OpProgress {
    use crate::operations::OpState;
    match op.state {
        OpState::Queued | OpState::Running => OpProgress::Pending,
        OpState::Succeeded => OpProgress::Succeeded {
            completed: parse_ts(&op.completed),
        },
        OpState::Failed | OpState::Cancelled => OpProgress::Failed(
            op.error
                .clone()
                .unwrap_or_else(|| format!("operation {}", op.state.as_str())),
        ),
    }
}

impl BackupBackend for OpsBackend {
    fn ready(&self) -> Result<(), String> {
        self.target.clone()
    }

    fn enqueue_backup(&self, namespace: &str, vm: &str, key: &str) -> Result<String, String> {
        crate::backup::op::enqueue(
            namespace,
            vm,
            "full",
            None,
            Some("dr replication".into()),
            key,
        )
        .map(|(op, _)| op.id)
        .map_err(|e| e.to_string())
    }

    fn backup_progress(&self, op_id: &str) -> Result<OpProgress, String> {
        let op = crate::operations::OperationsDb::global()
            .get(op_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("backup operation {op_id} not found"))?;
        let p = op_progress(&op);
        if matches!(p, OpProgress::Succeeded { .. }) {
            // Must be an off-cluster backup that can actually be restored.
            if let Err(e) = crate::backup::restore::source_from_op(&op) {
                return Ok(OpProgress::Failed(format!("backup is not restorable: {e}")));
            }
        }
        Ok(p)
    }
}

impl RestoreBackend for OpsBackend {
    fn enqueue_restore(
        &self,
        backup_op: &str,
        namespace: &str,
        new_vm: &str,
        start: bool,
    ) -> Result<String, String> {
        let db = crate::operations::OperationsDb::global();
        let op = db
            .get(backup_op)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("backup operation {backup_op} not found"))?;
        let src = crate::backup::restore::source_from_op(&op).map_err(|e| e.to_string())?;
        crate::backup::restore::enqueue_restore_in(db, &src, namespace, new_vm, None, start, None)
            .map(|o| o.id)
            .map_err(|e| e.to_string())
    }

    fn restore_progress(&self, op_id: &str) -> Result<OpProgress, String> {
        let op = crate::operations::OperationsDb::global()
            .get(op_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("restore operation {op_id} not found"))?;
        Ok(op_progress(&op))
    }
}

// --------------------------------------------------------------- alerts

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

pub trait AlertSink {
    fn alert(&self, severity: Severity, pair: &str, message: &str);
}

static ALERTS: Mutex<Option<crate::notifications::NotificationManager>> = Mutex::new(None);

/// Alert hook backed by the notifications module (in-memory, per replica, also logged).
pub struct NotificationSink;

impl AlertSink for NotificationSink {
    fn alert(&self, severity: Severity, pair: &str, message: &str) {
        use crate::notifications::{Notification, NotificationType};
        let (t, text) = match severity {
            Severity::Info => (
                NotificationType::Success,
                format!("DR pair {pair}: {message}"),
            ),
            Severity::Warning => (
                NotificationType::Warning,
                format!("DR pair {pair}: {message}"),
            ),
            Severity::Error => (
                NotificationType::Error,
                format!("DR pair {pair}: {message}"),
            ),
        };
        match severity {
            Severity::Error => log::error!("{text}"),
            Severity::Warning => log::warn!("{text}"),
            Severity::Info => log::info!("{text}"),
        }
        let n = Notification::new(&text, t)
            .with_source("dr-replication")
            .with_duration(ALERT_RETENTION_SECS);
        let mut g = ALERTS.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(crate::notifications::NotificationManager::default)
            .notify(n);
    }
}

/// Recent DR alerts, newest last.
pub fn recent_alerts() -> Vec<crate::notifications::Notification> {
    let g = ALERTS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()
        .map(|m| m.active().into_iter().cloned().collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------ lag / tick

#[derive(Debug, Clone, Serialize)]
pub struct Lag {
    pub seconds: u64,
    pub since_last_success: bool,
    pub rpo_met: bool,
}

/// Replication lag = time since the last verified backup (or since the pair
/// started, until the first one lands). Never negative.
pub fn lag(pair: &DrPair, now: DateTime<Utc>) -> Lag {
    let from = pair.last_success.unwrap_or(pair.epoch);
    let seconds = (now - from).num_seconds().max(0) as u64;
    Lag {
        seconds,
        since_last_success: pair.last_success.is_some(),
        rpo_met: seconds <= pair.rpo_seconds,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    Skipped,
    Waiting,
    Started(String),
    Succeeded(String),
    Failed(String),
}

fn record_failure(pair: &mut DrPair, now: DateTime<Utc>, msg: String, sink: &dyn AlertSink) {
    pair.in_flight_op = None;
    pair.last_attempt = Some(now);
    pair.consecutive_failures += 1;
    sink.alert(
        Severity::Warning,
        &pair.name,
        &format!(
            "backup failed ({} in a row): {msg}",
            pair.consecutive_failures
        ),
    );
    pair.last_error = Some(msg);
}

/// Edge-triggered RPO alerting: one alert when the RPO is first breached, one
/// when it recovers.
pub fn evaluate_alerts(pair: &mut DrPair, now: DateTime<Utc>, sink: &dyn AlertSink) {
    let l = lag(pair, now);
    match (pair.alert_state, l.rpo_met) {
        (AlertState::Ok, false) => {
            pair.alert_state = AlertState::Breached;
            sink.alert(
                Severity::Error,
                &pair.name,
                &format!(
                    "RPO breached: lag {}s exceeds target {}s",
                    l.seconds, pair.rpo_seconds
                ),
            );
        }
        (AlertState::Breached, true) => {
            pair.alert_state = AlertState::Ok;
            sink.alert(Severity::Info, &pair.name, "RPO recovered");
        }
        _ => {}
    }
}

/// One scheduler step for one pair. Pure apart from the backend calls; `now` is injected.
pub fn tick(
    pair: &mut DrPair,
    now: DateTime<Utc>,
    backup: &dyn BackupBackend,
    sink: &dyn AlertSink,
) -> TickOutcome {
    if !pair.enabled || pair.phase != Phase::Replicating {
        return TickOutcome::Skipped;
    }
    let outcome = step(pair, now, backup, sink);
    evaluate_alerts(pair, now, sink);
    outcome
}

fn step(
    pair: &mut DrPair,
    now: DateTime<Utc>,
    backup: &dyn BackupBackend,
    sink: &dyn AlertSink,
) -> TickOutcome {
    if let Some(op) = pair.in_flight_op.clone() {
        return match backup.backup_progress(&op) {
            Ok(OpProgress::Pending) => TickOutcome::Waiting,
            Ok(OpProgress::Succeeded { completed }) => {
                // Lag counts from the backup's own completion time when known.
                pair.last_success = Some(completed.unwrap_or(now).min(now));
                pair.last_success_op = Some(op.clone());
                pair.in_flight_op = None;
                pair.last_error = None;
                pair.consecutive_failures = 0;
                TickOutcome::Succeeded(op)
            }
            Ok(OpProgress::Failed(e)) => {
                record_failure(pair, now, e.clone(), sink);
                TickOutcome::Failed(e)
            }
            // The operation lookup itself failed: keep waiting, try again next tick.
            Err(_) => TickOutcome::Waiting,
        };
    }
    if now < pair.due_at() {
        return TickOutcome::Skipped;
    }
    if let Err(e) = backup.ready() {
        record_failure(pair, now, e.clone(), sink);
        return TickOutcome::Failed(e);
    }
    // Deterministic per due slot: a crash before the save re-uses the same operation.
    let key = format!("dr:{}:{}", pair.name, pair.due_at().timestamp());
    match backup.enqueue_backup(&pair.namespace, &pair.vm, &key) {
        Ok(op) => {
            pair.in_flight_op = Some(op.clone());
            pair.last_attempt = Some(now);
            TickOutcome::Started(op)
        }
        Err(e) => {
            record_failure(pair, now, e.clone(), sink);
            TickOutcome::Failed(e)
        }
    }
}

// ----------------------------------------------------------- RPO / RTO status

#[derive(Debug, Clone, Serialize)]
pub struct PairStatus {
    pub name: String,
    pub vm: String,
    pub namespace: String,
    pub dr_vm: String,
    pub dr_namespace: String,
    pub phase: Phase,
    pub enabled: bool,
    /// `healthy`, `rpo_breached`, `no_backup_yet`, `failing`, `paused`, `failed_over`.
    pub health: &'static str,
    pub rpo_seconds: u64,
    pub lag_seconds: u64,
    pub rpo_met: bool,
    pub last_success: Option<DateTime<Utc>>,
    pub last_attempt: Option<DateTime<Utc>>,
    pub next_due: Option<DateTime<Utc>>,
    pub in_flight_op: Option<String>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub rto_target_seconds: u64,
    /// Measured from the last failover request to restore completion, if one finished.
    pub rto_measured_seconds: Option<u64>,
    pub rto_met: Option<bool>,
}

pub fn status(pair: &DrPair, now: DateTime<Utc>) -> PairStatus {
    let l = lag(pair, now);
    let health = if pair.phase == Phase::FailedOver {
        "failed_over"
    } else if !pair.enabled {
        "paused"
    } else if !l.rpo_met {
        "rpo_breached"
    } else if pair.consecutive_failures > 0 {
        "failing"
    } else if pair.last_success.is_none() {
        "no_backup_yet"
    } else {
        "healthy"
    };
    let rto_measured = pair.failover.as_ref().and_then(|f| {
        f.completed_at
            .filter(|_| f.state == "succeeded")
            .map(|c| (c - f.requested_at).num_seconds().max(0) as u64)
    });
    PairStatus {
        name: pair.name.clone(),
        vm: pair.vm.clone(),
        namespace: pair.namespace.clone(),
        dr_vm: pair.dr_vm.clone(),
        dr_namespace: pair.dr_namespace.clone(),
        phase: pair.phase,
        enabled: pair.enabled,
        health,
        rpo_seconds: pair.rpo_seconds,
        lag_seconds: l.seconds,
        rpo_met: l.rpo_met,
        last_success: pair.last_success,
        last_attempt: pair.last_attempt,
        next_due: (pair.enabled && pair.phase == Phase::Replicating && pair.in_flight_op.is_none())
            .then(|| pair.due_at()),
        in_flight_op: pair.in_flight_op.clone(),
        last_error: pair.last_error.clone(),
        consecutive_failures: pair.consecutive_failures,
        rto_target_seconds: pair.rto_seconds,
        rto_measured_seconds: rto_measured,
        rto_met: rto_measured.map(|m| m <= pair.rto_seconds),
    }
}

// --------------------------------------------------------------- failover

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum SourceState {
    Running,
    Stopped,
    Absent,
    /// The cluster could not be asked; treated as unsafe.
    Unknown(String),
}

/// Facts gathered from the cluster by the caller (kept out of the pure logic).
#[derive(Debug, Clone)]
pub struct Facts {
    pub source: SourceState,
    /// Does a VM already exist at the DR target? `None` = could not tell.
    pub dr_vm_exists: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailoverPlan {
    pub pair: String,
    pub source: String,
    pub source_state: SourceState,
    pub dr_target: String,
    pub backup_op: Option<String>,
    pub backup_age_seconds: Option<u64>,
    /// Data written since the last verified backup is lost on failover.
    pub potential_data_loss_seconds: u64,
    pub rpo_met: bool,
    pub rto_target_seconds: u64,
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub allowed: bool,
    pub confirm_phrase: String,
}

pub fn failover_phrase(pair: &DrPair) -> String {
    format!("failover {}", pair.name)
}

pub fn resume_phrase(pair: &DrPair) -> String {
    format!("resume {}", pair.name)
}

pub fn plan_failover(pair: &DrPair, facts: &Facts, now: DateTime<Utc>) -> FailoverPlan {
    let l = lag(pair, now);
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    if pair.phase == Phase::FailedOver {
        blockers.push("pair has already failed over; see the failback plan".to_string());
    }
    match &facts.source {
        SourceState::Running => blockers.push(format!(
            "source VM {}/{} is running; stop it first (Zorvia never fails over a live source)",
            pair.namespace, pair.vm
        )),
        SourceState::Unknown(e) => blockers.push(format!(
            "could not determine whether the source VM is running ({e}); refusing"
        )),
        SourceState::Stopped => warnings.push(
            "source is stopped: take a final off-cluster backup now to shrink data loss, then re-plan"
                .to_string(),
        ),
        SourceState::Absent => warnings.push(
            "source VM no longer exists; restoring from the last verified backup".to_string(),
        ),
    }
    match facts.dr_vm_exists {
        Some(true) => blockers.push(format!(
            "a VM named {}/{} already exists; Zorvia will not overwrite it",
            pair.dr_namespace, pair.dr_vm
        )),
        None => blockers.push("could not determine whether the DR VM already exists".to_string()),
        Some(false) => {}
    }
    if pair.last_success_op.is_none() {
        blockers.push("no verified backup exists yet".to_string());
    }
    if pair.last_success_op.is_some() && !l.rpo_met {
        warnings.push(format!(
            "RPO breached: the newest backup is {}s old (target {}s)",
            l.seconds, pair.rpo_seconds
        ));
    }
    if pair.in_flight_op.is_some() {
        warnings.push("a backup is in flight and is not part of this failover".to_string());
    }
    let allowed = blockers.is_empty();
    FailoverPlan {
        pair: pair.name.clone(),
        source: format!("{}/{}", pair.namespace, pair.vm),
        source_state: facts.source.clone(),
        dr_target: format!("{}/{}", pair.dr_namespace, pair.dr_vm),
        backup_op: pair.last_success_op.clone(),
        backup_age_seconds: pair.last_success.map(|_| l.seconds),
        potential_data_loss_seconds: l.seconds,
        rpo_met: l.rpo_met,
        rto_target_seconds: pair.rto_seconds,
        steps: vec![
            "verify the source VM is not running".into(),
            format!(
                "restore the last verified backup as {}/{}",
                pair.dr_namespace, pair.dr_vm
            ),
            "optionally start the DR VM".into(),
            "repoint clients/DNS yourself: Zorvia does not move traffic".into(),
        ],
        warnings,
        blockers,
        allowed,
        confirm_phrase: failover_phrase(pair),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FailoverRequest {
    /// Defaults to true: nothing happens unless the caller opts out explicitly.
    #[serde(default = "yes")]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: Option<String>,
    /// Start the DR VM after restore (default true).
    #[serde(default = "yes")]
    pub start: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailoverError {
    Refused(Vec<String>),
    ConfirmationRequired(String),
    Backend(String),
}

impl std::fmt::Display for FailoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(b) => write!(f, "failover refused: {}", b.join("; ")),
            Self::ConfirmationRequired(p) => {
                write!(
                    f,
                    "confirmation required: send confirm = \"{p}\" with dry_run = false"
                )
            }
            Self::Backend(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FailoverOutcome {
    DryRun(FailoverPlan),
    Started {
        plan: FailoverPlan,
        restore_op: String,
    },
}

pub fn execute_failover(
    pair: &mut DrPair,
    facts: &Facts,
    req: &FailoverRequest,
    restore: &dyn RestoreBackend,
    now: DateTime<Utc>,
) -> Result<FailoverOutcome, FailoverError> {
    let plan = plan_failover(pair, facts, now);
    if req.dry_run {
        return Ok(FailoverOutcome::DryRun(plan));
    }
    if !plan.allowed {
        return Err(FailoverError::Refused(plan.blockers));
    }
    if req.confirm.as_deref() != Some(plan.confirm_phrase.as_str()) {
        return Err(FailoverError::ConfirmationRequired(plan.confirm_phrase));
    }
    let backup_op = pair
        .last_success_op
        .clone()
        .ok_or_else(|| FailoverError::Refused(vec!["no verified backup exists yet".to_string()]))?;
    let restore_op = restore
        .enqueue_restore(&backup_op, &pair.dr_namespace, &pair.dr_vm, req.start)
        .map_err(FailoverError::Backend)?;
    pair.phase = Phase::FailedOver;
    pair.failover = Some(FailoverRecord {
        requested_at: now,
        backup_op,
        restore_op: restore_op.clone(),
        dr_vm: pair.dr_vm.clone(),
        dr_namespace: pair.dr_namespace.clone(),
        state: "pending".into(),
        completed_at: None,
        error: None,
    });
    Ok(FailoverOutcome::Started { plan, restore_op })
}

/// Refresh the failover record from the restore operation (feeds measured RTO).
pub fn refresh_failover(pair: &mut DrPair, restore: &dyn RestoreBackend, now: DateTime<Utc>) {
    let Some(f) = pair.failover.as_mut() else {
        return;
    };
    if f.state != "pending" {
        return;
    }
    match restore.restore_progress(&f.restore_op) {
        Ok(OpProgress::Succeeded { completed }) => {
            f.state = "succeeded".into();
            f.completed_at = Some(completed.unwrap_or(now));
        }
        Ok(OpProgress::Failed(e)) => {
            f.state = "failed".into();
            f.error = Some(e);
            f.completed_at = Some(now);
        }
        _ => {}
    }
}

// -------------------------------------------------------------- failback

#[derive(Debug, Clone, Serialize)]
pub struct FailbackPlan {
    pub pair: String,
    pub allowed: bool,
    pub blockers: Vec<String>,
    pub steps: Vec<String>,
    pub resume_phrase: String,
}

/// Failback is a plan plus a confirmed `resume`: Zorvia does not automate the data
/// return, because it must never overwrite a source VM.
pub fn failback_plan(pair: &DrPair, facts: &Facts) -> FailbackPlan {
    let mut blockers = Vec::new();
    if pair.phase != Phase::FailedOver {
        blockers.push("pair has not failed over".to_string());
    }
    match &facts.source {
        SourceState::Absent => {}
        SourceState::Unknown(e) => {
            blockers.push(format!("could not inspect the original source VM ({e})"))
        }
        _ => blockers.push(format!(
            "the original VM {}/{} still exists; delete or rename it (Zorvia will not overwrite it)",
            pair.namespace, pair.vm
        )),
    }
    let steps = vec![
        format!(
            "stop {}/{} (the DR VM) so its disks are quiescent",
            pair.dr_namespace, pair.dr_vm
        ),
        format!(
            "POST /api/backups/offcluster for {}/{} and wait for it to succeed",
            pair.dr_namespace, pair.dr_vm
        ),
        format!(
            "remove the old source {}/{} once you no longer need it",
            pair.namespace, pair.vm
        ),
        format!(
            "restore that backup as {}/{} via the restore API",
            pair.namespace, pair.vm
        ),
        "repoint clients back, then resume replication for this pair".to_string(),
    ];
    FailbackPlan {
        pair: pair.name.clone(),
        allowed: blockers.is_empty(),
        blockers,
        steps,
        resume_phrase: resume_phrase(pair),
    }
}

/// Return a failed-over pair to replication after the operator finished failback.
pub fn resume(
    pair: &mut DrPair,
    facts: &Facts,
    confirm: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), FailoverError> {
    let plan = failback_plan(pair, facts);
    if pair.phase != Phase::FailedOver {
        return Err(FailoverError::Refused(plan.blockers));
    }
    // The source VM must be back (not Absent): replication needs something to back up.
    if matches!(facts.source, SourceState::Absent | SourceState::Unknown(_)) {
        return Err(FailoverError::Refused(vec![format!(
            "source VM {}/{} must exist again before replication resumes",
            pair.namespace, pair.vm
        )]));
    }
    if confirm != Some(plan.resume_phrase.as_str()) {
        return Err(FailoverError::ConfirmationRequired(plan.resume_phrase));
    }
    pair.phase = Phase::Replicating;
    pair.failover = None;
    pair.in_flight_op = None;
    pair.last_attempt = None;
    pair.last_success = None;
    pair.last_success_op = None;
    pair.last_error = None;
    pair.consecutive_failures = 0;
    pair.alert_state = AlertState::Ok;
    pair.epoch = now;
    Ok(())
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn pair() -> DrPair {
        DrPair::new("web", "web1", "prod", "dr", 300, 900, t0()).unwrap()
    }

    #[derive(Default)]
    struct FakeBackup {
        ready_err: RefCell<Option<String>>,
        enqueued: RefCell<Vec<(String, String)>>,
        progress: RefCell<Option<OpProgress>>,
    }

    impl BackupBackend for FakeBackup {
        fn ready(&self) -> Result<(), String> {
            self.ready_err.borrow().clone().map_or(Ok(()), Err)
        }
        fn enqueue_backup(&self, _ns: &str, vm: &str, key: &str) -> Result<String, String> {
            let id = format!("op-{}", self.enqueued.borrow().len() + 1);
            self.enqueued.borrow_mut().push((vm.into(), key.into()));
            Ok(id)
        }
        fn backup_progress(&self, _op: &str) -> Result<OpProgress, String> {
            Ok(self
                .progress
                .borrow()
                .clone()
                .unwrap_or(OpProgress::Pending))
        }
    }

    #[derive(Default)]
    struct FakeRestore {
        calls: RefCell<Vec<(String, String, String, bool)>>,
        progress: RefCell<Option<OpProgress>>,
        fail: bool,
    }

    impl RestoreBackend for FakeRestore {
        fn enqueue_restore(
            &self,
            b: &str,
            ns: &str,
            vm: &str,
            start: bool,
        ) -> Result<String, String> {
            if self.fail {
                return Err("boom".into());
            }
            self.calls
                .borrow_mut()
                .push((b.into(), ns.into(), vm.into(), start));
            Ok("rop-1".into())
        }
        fn restore_progress(&self, _op: &str) -> Result<OpProgress, String> {
            Ok(self
                .progress
                .borrow()
                .clone()
                .unwrap_or(OpProgress::Pending))
        }
    }

    #[derive(Default)]
    struct Sink(RefCell<Vec<(Severity, String)>>);
    impl AlertSink for Sink {
        fn alert(&self, s: Severity, _p: &str, m: &str) {
            self.0.borrow_mut().push((s, m.into()));
        }
    }

    fn facts(source: SourceState) -> Facts {
        Facts {
            source,
            dr_vm_exists: Some(false),
        }
    }

    fn replicated_pair() -> DrPair {
        let mut p = pair();
        p.last_success = Some(t0());
        p.last_success_op = Some("op-9".into());
        p
    }

    #[test]
    fn validation_rejects_bad_input() {
        assert!(DrPair::new("Web", "a", "b", "c", 300, 900, t0()).is_err());
        assert!(DrPair::new("w", "a", "b", "c", 10, 900, t0()).is_err());
        assert!(DrPair::new("w", "a", "b", "c", 300, 0, t0()).is_err());
        assert!(DrPair::new("w", &"a".repeat(62), "b", "c", 300, 900, t0()).is_err());
        assert!(DrPair::new("w", "a", "b", "c", 300, 900, t0()).is_ok());
        assert_eq!(pair().dr_vm, "web1-dr");
    }

    #[test]
    fn pairs_persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dr_pairs_test.json");
        PairStore::update(&path, |s| s.add(pair())).unwrap();
        let s = PairStore::load_from(&path);
        assert_eq!(s.pairs.len(), 1);
        assert_eq!(s.get("web").unwrap().dr_vm, "web1-dr");
        // Mutations persist too.
        PairStore::update(&path, |s| {
            s.get_mut("web").unwrap().last_success_op = Some("op-1".into());
            Ok(())
        })
        .unwrap();
        let s = PairStore::load_from(&path);
        assert_eq!(
            s.get("web").unwrap().last_success_op.as_deref(),
            Some("op-1")
        );
        PairStore::update(&path, |s| {
            assert!(s.remove("web"));
            Ok(())
        })
        .unwrap();
        assert!(PairStore::load_from(&path).pairs.is_empty());
    }

    #[test]
    fn store_rejects_duplicates() {
        let mut s = PairStore::default();
        s.add(pair()).unwrap();
        assert!(s.add(pair()).is_err());
        let other = DrPair::new("other", "web1", "prod", "dr2", 300, 900, t0()).unwrap();
        assert!(s.add(other).is_err());
        let clash = DrPair::new("o2", "x", "prod", "dr", 300, 900, t0()).unwrap();
        assert!(s.add(clash).is_ok());
        let clash_dr = DrPair::new("o3", "web1", "stage", "dr", 300, 900, t0()).unwrap();
        assert!(s.add(clash_dr).is_err());
    }

    #[test]
    fn first_tick_enqueues_then_success_records_lag() {
        let mut p = pair();
        let b = FakeBackup::default();
        let sink = Sink::default();
        let out = tick(&mut p, t0(), &b, &sink);
        assert_eq!(out, TickOutcome::Started("op-1".into()));
        assert_eq!(b.enqueued.borrow()[0].0, "web1");
        // Still pending a minute later.
        assert_eq!(
            tick(&mut p, t0() + Duration::seconds(60), &b, &sink),
            TickOutcome::Waiting
        );
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded {
            completed: Some(t0() + Duration::seconds(90)),
        });
        let now = t0() + Duration::seconds(120);
        assert_eq!(
            tick(&mut p, now, &b, &sink),
            TickOutcome::Succeeded("op-1".into())
        );
        assert_eq!(p.last_success_op.as_deref(), Some("op-1"));
        let l = lag(&p, now);
        assert_eq!(l.seconds, 30);
        assert!(l.rpo_met && l.since_last_success);
        assert_eq!(status(&p, now).health, "healthy");
    }

    #[test]
    fn schedule_respects_interval() {
        let mut p = pair();
        let b = FakeBackup::default();
        let sink = Sink::default();
        tick(&mut p, t0(), &b, &sink);
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        tick(&mut p, t0() + Duration::seconds(10), &b, &sink);
        assert!(p.in_flight_op.is_none());
        assert_eq!(p.due_at(), t0() + Duration::seconds(300));
        // Too early: nothing new.
        assert_eq!(
            tick(&mut p, t0() + Duration::seconds(200), &b, &sink),
            TickOutcome::Skipped
        );
        assert_eq!(b.enqueued.borrow().len(), 1);
        // Due: a second backup with a different key.
        assert!(matches!(
            tick(&mut p, t0() + Duration::seconds(300), &b, &sink),
            TickOutcome::Started(_)
        ));
        let keys: Vec<_> = b.enqueued.borrow().iter().map(|e| e.1.clone()).collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn failure_is_recorded_and_retried_on_interval_not_hot() {
        let mut p = pair();
        let b = FakeBackup::default();
        let sink = Sink::default();
        tick(&mut p, t0(), &b, &sink);
        *b.progress.borrow_mut() = Some(OpProgress::Failed("disk error".into()));
        let now = t0() + Duration::seconds(30);
        assert_eq!(
            tick(&mut p, now, &b, &sink),
            TickOutcome::Failed("disk error".into())
        );
        assert_eq!(p.consecutive_failures, 1);
        assert_eq!(p.last_error.as_deref(), Some("disk error"));
        assert_eq!(status(&p, now).health, "failing");
        assert_eq!(
            tick(&mut p, now + Duration::seconds(1), &b, &sink),
            TickOutcome::Skipped
        );
        assert!(sink.0.borrow().iter().any(|(s, _)| *s == Severity::Warning));
    }

    #[test]
    fn missing_target_is_a_recorded_failure() {
        let mut p = pair();
        let b = FakeBackup::default();
        *b.ready_err.borrow_mut() = Some("no off-cluster target".into());
        let sink = Sink::default();
        assert!(matches!(
            tick(&mut p, t0(), &b, &sink),
            TickOutcome::Failed(_)
        ));
        assert!(b.enqueued.borrow().is_empty());
        assert_eq!(p.last_error.as_deref(), Some("no off-cluster target"));
    }

    #[test]
    fn rpo_alert_is_edge_triggered_and_recovers() {
        let mut p = pair();
        let b = FakeBackup::default();
        let sink = Sink::default();
        tick(&mut p, t0(), &b, &sink); // in flight, never completes
        tick(&mut p, t0() + Duration::seconds(299), &b, &sink);
        assert!(sink.0.borrow().is_empty());
        tick(&mut p, t0() + Duration::seconds(400), &b, &sink);
        tick(&mut p, t0() + Duration::seconds(500), &b, &sink);
        let errs = sink
            .0
            .borrow()
            .iter()
            .filter(|(s, _)| *s == Severity::Error)
            .count();
        assert_eq!(errs, 1);
        assert_eq!(
            status(&p, t0() + Duration::seconds(500)).health,
            "rpo_breached"
        );
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded {
            completed: Some(t0() + Duration::seconds(600)),
        });
        tick(&mut p, t0() + Duration::seconds(610), &b, &sink);
        assert_eq!(p.alert_state, AlertState::Ok);
        assert!(sink.0.borrow().iter().any(|(s, _)| *s == Severity::Info));
    }

    #[test]
    fn lag_never_negative_and_paused_pairs_skip() {
        let mut p = replicated_pair();
        assert_eq!(lag(&p, t0() - Duration::seconds(50)).seconds, 0);
        p.enabled = false;
        let b = FakeBackup::default();
        assert_eq!(
            tick(&mut p, t0() + Duration::seconds(9999), &b, &Sink::default()),
            TickOutcome::Skipped
        );
        assert_eq!(status(&p, t0()).health, "paused");
    }

    #[test]
    fn failover_refuses_when_source_running() {
        let p = replicated_pair();
        let plan = plan_failover(&p, &facts(SourceState::Running), t0());
        assert!(!plan.allowed);
        assert!(plan.blockers.iter().any(|b| b.contains("is running")));
        let mut p = p;
        let r = FakeRestore::default();
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: true,
        };
        let e = execute_failover(&mut p, &facts(SourceState::Running), &req, &r, t0()).unwrap_err();
        assert!(matches!(e, FailoverError::Refused(_)));
        assert!(r.calls.borrow().is_empty());
        assert_eq!(p.phase, Phase::Replicating);
    }

    #[test]
    fn failover_refuses_when_source_state_unknown_or_dr_exists_or_no_backup() {
        let p = replicated_pair();
        assert!(!plan_failover(&p, &facts(SourceState::Unknown("x".into())), t0()).allowed);
        let f = Facts {
            source: SourceState::Stopped,
            dr_vm_exists: Some(true),
        };
        assert!(!plan_failover(&p, &f, t0()).allowed);
        let f = Facts {
            source: SourceState::Stopped,
            dr_vm_exists: None,
        };
        assert!(!plan_failover(&p, &f, t0()).allowed);
        assert!(!plan_failover(&pair(), &facts(SourceState::Stopped), t0()).allowed);
    }

    #[test]
    fn failover_refuses_without_confirmation() {
        let mut p = replicated_pair();
        let r = FakeRestore::default();
        for confirm in [
            None,
            Some("yes".to_string()),
            Some("failover other".to_string()),
        ] {
            let req = FailoverRequest {
                dry_run: false,
                confirm,
                start: true,
            };
            let e =
                execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap_err();
            assert!(matches!(e, FailoverError::ConfirmationRequired(_)));
        }
        assert!(r.calls.borrow().is_empty());
        assert_eq!(p.phase, Phase::Replicating);
    }

    #[test]
    fn dry_run_is_default_and_changes_nothing() {
        let req: FailoverRequest = serde_json::from_str("{}").unwrap();
        assert!(req.dry_run);
        let mut p = replicated_pair();
        let r = FakeRestore::default();
        let out = execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap();
        assert!(matches!(out, FailoverOutcome::DryRun(ref pl) if pl.allowed));
        assert!(r.calls.borrow().is_empty());
        assert_eq!(p.phase, Phase::Replicating);
    }

    #[test]
    fn confirmed_failover_restores_as_dr_vm_and_measures_rto() {
        let mut p = replicated_pair();
        let r = FakeRestore::default();
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: true,
        };
        let now = t0() + Duration::seconds(100);
        let out = execute_failover(&mut p, &facts(SourceState::Absent), &req, &r, now).unwrap();
        assert!(matches!(out, FailoverOutcome::Started { .. }));
        assert_eq!(
            r.calls.borrow()[0],
            ("op-9".into(), "dr".into(), "web1-dr".into(), true)
        );
        assert_eq!(p.phase, Phase::FailedOver);
        // Failed-over pairs stop replicating and cannot fail over twice.
        assert_eq!(
            tick(&mut p, now, &FakeBackup::default(), &Sink::default()),
            TickOutcome::Skipped
        );
        assert!(!plan_failover(&p, &facts(SourceState::Absent), now).allowed);
        *r.progress.borrow_mut() = Some(OpProgress::Succeeded {
            completed: Some(now + Duration::seconds(240)),
        });
        refresh_failover(&mut p, &r, now + Duration::seconds(300));
        let s = status(&p, now + Duration::seconds(300));
        assert_eq!(s.rto_measured_seconds, Some(240));
        assert_eq!(s.rto_met, Some(true));
        assert_eq!(s.health, "failed_over");
    }

    #[test]
    fn restore_backend_error_leaves_pair_untouched() {
        let mut p = replicated_pair();
        let r = FakeRestore {
            fail: true,
            ..Default::default()
        };
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: false,
        };
        let e = execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap_err();
        assert!(matches!(e, FailoverError::Backend(_)));
        assert_eq!(p.phase, Phase::Replicating);
        assert!(p.failover.is_none());
    }

    #[test]
    fn rpo_breach_is_a_warning_not_a_blocker() {
        let p = replicated_pair();
        let plan = plan_failover(
            &p,
            &facts(SourceState::Stopped),
            t0() + Duration::seconds(1000),
        );
        assert!(plan.allowed);
        assert!(!plan.rpo_met);
        assert_eq!(plan.potential_data_loss_seconds, 1000);
        assert!(plan.warnings.iter().any(|w| w.contains("RPO breached")));
    }

    #[test]
    fn failback_plan_and_resume_rules() {
        let mut p = replicated_pair();
        assert!(!failback_plan(&p, &facts(SourceState::Absent)).allowed);
        let r = FakeRestore::default();
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: true,
        };
        execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap();
        // Original still present: blocked.
        assert!(!failback_plan(&p, &facts(SourceState::Stopped)).allowed);
        assert!(failback_plan(&p, &facts(SourceState::Absent)).allowed);
        // Resume needs the source back and the phrase.
        assert!(matches!(
            resume(
                &mut p,
                &facts(SourceState::Absent),
                Some("resume web"),
                t0()
            ),
            Err(FailoverError::Refused(_))
        ));
        assert!(matches!(
            resume(&mut p, &facts(SourceState::Stopped), None, t0()),
            Err(FailoverError::ConfirmationRequired(_))
        ));
        let later = t0() + Duration::seconds(5000);
        resume(
            &mut p,
            &facts(SourceState::Stopped),
            Some("resume web"),
            later,
        )
        .unwrap();
        assert_eq!(p.phase, Phase::Replicating);
        assert!(p.last_success_op.is_none() && p.failover.is_none());
        assert_eq!(p.epoch, later);
    }

    #[test]
    fn notification_sink_records_alerts() {
        NotificationSink.alert(Severity::Error, "zz-test", "RPO breached");
        assert!(recent_alerts()
            .iter()
            .any(|n| n.source == "dr-replication" && n.message.contains("zz-test")));
    }
}
