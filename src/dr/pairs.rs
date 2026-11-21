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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailbackState {
    /// Off-cluster backup of the DR VM is running.
    BackingUp,
    /// The DR VM backup is verified; waiting for the original name to be free.
    BackedUp,
    /// The restore to the original VM name is running.
    Restoring,
    Succeeded,
    Failed,
    Cancelled,
}

impl FailbackState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::BackingUp | Self::BackedUp | Self::Restoring)
    }
}

/// Progress and result of a real failback, persisted in the pair so a crash or
/// restart resumes from the recorded stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailbackRecord {
    pub requested_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: FailbackState,
    pub dr_backup_op: Option<String>,
    pub restore_op: Option<String>,
    /// Start the restored original VM after the restore (default false).
    pub start: bool,
    /// Keep the DR VM after success until the operator deletes it (default true).
    pub keep_dr_vm: bool,
    #[serde(default)]
    pub dr_vm_deleted: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Non-terminal status text, e.g. why a restore is still waiting.
    pub note: Option<String>,
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
    #[serde(default)]
    pub failback: Option<FailbackRecord>,
    /// Counts failback starts; part of the backup idempotency key so a crash
    /// before the save re-uses the same backup operation.
    #[serde(default)]
    pub failback_seq: u32,
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
            failback: None,
            failback_seq: 0,
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
    /// Ask the queue to stop a backup that is no longer wanted (best effort).
    fn cancel_backup(&self, _op_id: &str) -> Result<(), String> {
        Ok(())
    }
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
    /// An existing non-failed restore of `backup_op` to `namespace/new_vm`, so a
    /// resume after a crash adopts it instead of queueing a second one.
    fn find_restore(&self, _backup_op: &str, _namespace: &str, _new_vm: &str) -> Option<String> {
        None
    }
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

    fn cancel_backup(&self, op_id: &str) -> Result<(), String> {
        crate::operations::OperationsDb::global()
            .request_cancel(op_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
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

    fn find_restore(&self, backup_op: &str, namespace: &str, new_vm: &str) -> Option<String> {
        let ops = crate::operations::OperationsDb::global()
            .list(Some(crate::backup::restore::RESTORE_KIND), 200)
            .ok()?;
        ops.into_iter()
            .filter(|o| {
                o.namespace == namespace
                    && o.params.get("new_vm_name").and_then(|v| v.as_str()) == Some(new_vm)
                    && o.params.get("source_op").and_then(|v| v.as_str()) == Some(backup_op)
                    && !matches!(
                        o.state,
                        crate::operations::OpState::Failed | crate::operations::OpState::Cancelled
                    )
            })
            .map(|o| o.id)
            .next()
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

const MAX_ALERTS: usize = 200;
static ALERT_LOCK: Mutex<()> = Mutex::new(());

/// The alert list, persisted as one JSON document through `state_docs` (a file, or
/// PostgreSQL when `ZORVIA_DATABASE_URL` is set) so alerts survive restarts and
/// are shared by replicas. Concurrent appends from two replicas are
/// last-writer-wins, like the pairs document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct AlertDoc {
    alerts: Vec<crate::notifications::Notification>,
}

pub fn alerts_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zorvia")
        .join("dr_alerts.json")
}

fn load_alerts(path: &Path) -> AlertDoc {
    match crate::state_docs::read(path) {
        Some(c) => serde_json::from_str(&c).unwrap_or_else(|e| {
            log::warn!("Failed to parse DR alerts: {e}");
            AlertDoc::default()
        }),
        None => AlertDoc::default(),
    }
}

/// Append one alert (expired ones and anything past the cap are dropped).
pub fn record_alert(path: &Path, n: crate::notifications::Notification) {
    let _g = ALERT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = load_alerts(path);
    doc.alerts.retain(|a| !a.is_expired());
    doc.alerts.push(n);
    if doc.alerts.len() > MAX_ALERTS {
        let excess = doc.alerts.len() - MAX_ALERTS;
        doc.alerts.drain(0..excess);
    }
    let saved = serde_json::to_string_pretty(&doc)
        .map_err(anyhow::Error::from)
        .and_then(|j| crate::state_docs::write(path, &j));
    if let Err(e) = saved {
        log::error!("Failed to persist DR alert: {e}");
    }
}

/// Recent alerts stored at `path`, oldest first.
pub fn recent_alerts_from(path: &Path) -> Vec<crate::notifications::Notification> {
    let _g = ALERT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_alerts(path)
        .alerts
        .into_iter()
        .filter(|a| !a.is_expired())
        .collect()
}

/// Recent DR alerts, newest last.
pub fn recent_alerts() -> Vec<crate::notifications::Notification> {
    recent_alerts_from(&alerts_path())
}

fn emit(path: &Path, severity: Severity, pair: &str, message: &str) {
    use crate::notifications::{Notification, NotificationType};
    let t = match severity {
        Severity::Info => NotificationType::Success,
        Severity::Warning => NotificationType::Warning,
        Severity::Error => NotificationType::Error,
    };
    let text = format!("DR pair {pair}: {message}");
    match severity {
        Severity::Error => log::error!("{text}"),
        Severity::Warning => log::warn!("{text}"),
        Severity::Info => log::info!("{text}"),
    }
    record_alert(
        path,
        Notification::new(&text, t)
            .with_source("dr-replication")
            .with_duration(ALERT_RETENTION_SECS),
    );
}

/// Alert hook backed by the notifications module: logged and persisted durably.
pub struct NotificationSink;

impl AlertSink for NotificationSink {
    fn alert(&self, severity: Severity, pair: &str, message: &str) {
        emit(&alerts_path(), severity, pair, message);
    }
}

/// Same, writing to an explicit path (tests).
pub struct PathSink(pub PathBuf);

impl AlertSink for PathSink {
    fn alert(&self, severity: Severity, pair: &str, message: &str) {
        emit(&self.0, severity, pair, message);
    }
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
    pub warnings: Vec<String>,
    pub steps: Vec<String>,
    pub confirm_phrase: String,
    pub resume_phrase: String,
    /// The current or last failback, if any.
    pub failback: Option<FailbackRecord>,
}

pub fn failback_phrase(pair: &DrPair) -> String {
    format!("failback {}", pair.name)
}

/// Plan for returning to the original VM. The data return is automated by
/// [`execute_failback`]: a verified off-cluster backup of the DR VM restored to
/// the original name, only when that name is free and its VM is not running.
pub fn failback_plan(pair: &DrPair, facts: &Facts) -> FailbackPlan {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    if pair.phase != Phase::FailedOver {
        blockers.push("pair has not failed over".to_string());
    } else if pair.failover.as_ref().map(|f| f.state.as_str()) != Some("succeeded") {
        blockers.push("the failover has not completed successfully".to_string());
    }
    if let Some(fb) = &pair.failback {
        if fb.state.is_active() {
            blockers.push("a failback is already in progress".to_string());
        } else if fb.state == FailbackState::Succeeded {
            blockers.push("failback already completed; resume replication".to_string());
        }
    }
    match &facts.source {
        SourceState::Running => blockers.push(format!(
            "the original VM {}/{} is running; Zorvia never restores over a running VM",
            pair.namespace, pair.vm
        )),
        SourceState::Unknown(e) => blockers.push(format!(
            "could not determine the state of the original VM ({e}); refusing"
        )),
        SourceState::Stopped => warnings.push(format!(
            "the original VM {}/{} exists (stopped): the restore waits until you delete or rename it, because a restore never overwrites",
            pair.namespace, pair.vm
        )),
        SourceState::Absent => {}
    }
    match facts.dr_vm_exists {
        Some(true) => {}
        Some(false) => blockers.push(format!(
            "the DR VM {}/{} does not exist; there is nothing to back up",
            pair.dr_namespace, pair.dr_vm
        )),
        None => blockers.push("could not determine whether the DR VM exists".to_string()),
    }
    warnings.push(
        "stop the DR VM first for a clean copy: the backup captures its disks as they are"
            .to_string(),
    );
    let steps = vec![
        format!(
            "take a verified off-cluster backup of {}/{}",
            pair.dr_namespace, pair.dr_vm
        ),
        format!(
            "restore it as {}/{} once the original is stopped or absent",
            pair.namespace, pair.vm
        ),
        "keep the DR VM until you delete it (unless keep_dr_vm=false)".to_string(),
        "repoint clients back yourself, then POST resume to restart replication".to_string(),
    ];
    FailbackPlan {
        pair: pair.name.clone(),
        allowed: blockers.is_empty(),
        blockers,
        warnings,
        steps,
        confirm_phrase: failback_phrase(pair),
        resume_phrase: resume_phrase(pair),
        failback: pair.failback.clone(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FailbackRequest {
    /// Defaults to true: nothing happens unless the caller opts out explicitly.
    #[serde(default = "yes")]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: Option<String>,
    /// Start the restored original VM (default false; start it after repointing).
    #[serde(default)]
    pub start: bool,
    /// Keep the DR VM after success until the operator deletes it (default true).
    #[serde(default = "yes")]
    pub keep_dr_vm: bool,
}

#[derive(Debug, Clone)]
pub enum FailbackOutcome {
    DryRun(FailbackPlan),
    Started {
        plan: FailbackPlan,
        backup_op: String,
    },
}

/// Start a failback: dry-run by default; a real run needs the confirmation phrase
/// and an allowed plan, then queues the verified DR VM backup.
pub fn execute_failback(
    pair: &mut DrPair,
    facts: &Facts,
    req: &FailbackRequest,
    backup: &dyn BackupBackend,
    sink: &dyn AlertSink,
    now: DateTime<Utc>,
) -> Result<FailbackOutcome, FailoverError> {
    let plan = failback_plan(pair, facts);
    if req.dry_run {
        return Ok(FailbackOutcome::DryRun(plan));
    }
    if !plan.allowed {
        return Err(FailoverError::Refused(plan.blockers));
    }
    if req.confirm.as_deref() != Some(plan.confirm_phrase.as_str()) {
        return Err(FailoverError::ConfirmationRequired(plan.confirm_phrase));
    }
    backup.ready().map_err(FailoverError::Backend)?;
    let failover_ts = pair
        .failover
        .as_ref()
        .map_or(0, |f| f.requested_at.timestamp());
    let key = format!(
        "dr-failback:{}:{}:{}",
        pair.name, failover_ts, pair.failback_seq
    );
    let op = backup
        .enqueue_backup(&pair.dr_namespace, &pair.dr_vm, &key)
        .map_err(FailoverError::Backend)?;
    pair.failback_seq += 1;
    pair.failback = Some(FailbackRecord {
        requested_at: now,
        updated_at: now,
        state: FailbackState::BackingUp,
        dr_backup_op: Some(op.clone()),
        restore_op: None,
        start: req.start,
        keep_dr_vm: req.keep_dr_vm,
        dr_vm_deleted: false,
        completed_at: None,
        error: None,
        note: None,
    });
    sink.alert(
        Severity::Warning,
        &pair.name,
        &format!(
            "failback started: backing up {}/{}",
            pair.dr_namespace, pair.dr_vm
        ),
    );
    Ok(FailbackOutcome::Started {
        plan,
        backup_op: op,
    })
}

fn fail_failback(pair: &mut DrPair, msg: String, sink: &dyn AlertSink, now: DateTime<Utc>) {
    sink.alert(
        Severity::Error,
        &pair.name,
        &format!("failback failed: {msg}"),
    );
    if let Some(fb) = pair.failback.as_mut() {
        fb.state = FailbackState::Failed;
        fb.error = Some(msg);
        fb.note = None;
        fb.completed_at = Some(now);
        fb.updated_at = now;
    }
}

/// Move an active failback forward one step. Safe to call repeatedly and after a
/// restart: it only reads the persisted stage. The source VM state is re-checked
/// right before the restore is queued, because time has passed since the plan.
pub fn advance_failback(
    pair: &mut DrPair,
    facts: &Facts,
    backup: &dyn BackupBackend,
    restore: &dyn RestoreBackend,
    sink: &dyn AlertSink,
    now: DateTime<Utc>,
) {
    let Some(fb) = pair.failback.clone() else {
        return;
    };
    if !fb.state.is_active() {
        return;
    }
    let mut state = fb.state;
    if state == FailbackState::BackingUp {
        let Some(op) = fb.dr_backup_op.clone() else {
            return fail_failback(pair, "no backup operation recorded".into(), sink, now);
        };
        match backup.backup_progress(&op) {
            Ok(OpProgress::Succeeded { .. }) => {
                state = FailbackState::BackedUp;
                set_failback(pair, state, None, now);
            }
            Ok(OpProgress::Failed(e)) => {
                return fail_failback(pair, format!("DR VM backup failed: {e}"), sink, now)
            }
            // Pending, or the lookup itself failed: try again next tick.
            _ => return,
        }
    }
    if state == FailbackState::BackedUp {
        let Some(bop) = fb.dr_backup_op.clone() else {
            return;
        };
        // A crash after queueing the restore but before saving: adopt it.
        let adopted = restore.find_restore(&bop, &pair.namespace, &pair.vm);
        if adopted.is_none() {
            match &facts.source {
                SourceState::Running => {
                    return fail_failback(
                        pair,
                        format!(
                            "original VM {}/{} is running; refusing to restore over it",
                            pair.namespace, pair.vm
                        ),
                        sink,
                        now,
                    )
                }
                SourceState::Unknown(e) => {
                    return set_failback_note(
                        pair,
                        format!("waiting: cannot read the original VM state ({e})"),
                        now,
                    )
                }
                SourceState::Stopped => {
                    return set_failback_note(
                        pair,
                        format!(
                            "waiting: original VM {}/{} still exists (stopped); delete or rename it, a restore never overwrites",
                            pair.namespace, pair.vm
                        ),
                        now,
                    )
                }
                SourceState::Absent => {}
            }
        }
        let op = match adopted {
            Some(op) => op,
            None => match restore.enqueue_restore(&bop, &pair.namespace, &pair.vm, fb.start) {
                Ok(op) => op,
                Err(e) => {
                    return set_failback_note(pair, format!("restore not queued yet: {e}"), now)
                }
            },
        };
        if let Some(f) = pair.failback.as_mut() {
            f.restore_op = Some(op);
        }
        set_failback(pair, FailbackState::Restoring, None, now);
        return;
    }
    if state == FailbackState::Restoring {
        let Some(op) = fb.restore_op.clone() else {
            return fail_failback(pair, "no restore operation recorded".into(), sink, now);
        };
        match restore.restore_progress(&op) {
            Ok(OpProgress::Succeeded { completed }) => {
                set_failback(pair, FailbackState::Succeeded, None, now);
                if let Some(f) = pair.failback.as_mut() {
                    f.completed_at = Some(completed.unwrap_or(now).min(now));
                }
                sink.alert(
                    Severity::Info,
                    &pair.name,
                    &format!(
                        "failback complete: {}/{} restored; repoint clients, then resume replication",
                        pair.namespace, pair.vm
                    ),
                );
            }
            Ok(OpProgress::Failed(e)) => {
                fail_failback(pair, format!("restore failed: {e}"), sink, now)
            }
            _ => {}
        }
    }
}

fn set_failback(pair: &mut DrPair, state: FailbackState, note: Option<String>, now: DateTime<Utc>) {
    if let Some(f) = pair.failback.as_mut() {
        f.state = state;
        f.note = note;
        f.updated_at = now;
    }
}

fn set_failback_note(pair: &mut DrPair, note: String, now: DateTime<Utc>) {
    if let Some(f) = pair.failback.as_mut() {
        f.note = Some(note);
        f.updated_at = now;
    }
}

/// Cancel a failback that has not started restoring. A restore already in
/// flight cannot be stopped safely and is refused.
pub fn cancel_failback(
    pair: &mut DrPair,
    backup: &dyn BackupBackend,
    sink: &dyn AlertSink,
    now: DateTime<Utc>,
) -> Result<(), FailoverError> {
    let Some(fb) = pair.failback.clone() else {
        return Err(FailoverError::Refused(vec![
            "no failback to cancel".to_string()
        ]));
    };
    match fb.state {
        FailbackState::BackingUp | FailbackState::BackedUp => {}
        FailbackState::Restoring => {
            return Err(FailoverError::Refused(vec![
                "the restore is already running and cannot be cancelled safely".to_string(),
            ]))
        }
        _ => {
            return Err(FailoverError::Refused(vec![
                "the failback is not active".to_string()
            ]))
        }
    }
    if let Some(op) = fb.dr_backup_op.as_deref() {
        // Best effort: the backup is harmless if it finishes anyway.
        if let Err(e) = backup.cancel_backup(op) {
            log::warn!("failback cancel: backup {op}: {e}");
        }
    }
    set_failback(pair, FailbackState::Cancelled, None, now);
    if let Some(f) = pair.failback.as_mut() {
        f.completed_at = Some(now);
    }
    sink.alert(Severity::Warning, &pair.name, "failback cancelled");
    Ok(())
}

/// True when the DR VM should be deleted now: failback succeeded, the operator
/// asked not to keep it, and it has not been deleted yet.
pub fn dr_vm_cleanup_due(pair: &DrPair) -> bool {
    pair.failback
        .as_ref()
        .is_some_and(|f| f.state == FailbackState::Succeeded && !f.keep_dr_vm && !f.dr_vm_deleted)
}

pub fn mark_dr_vm_deleted(pair: &mut DrPair) {
    if let Some(f) = pair.failback.as_mut() {
        f.dr_vm_deleted = true;
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
    if pair.failback.as_ref().is_some_and(|f| f.state.is_active()) {
        return Err(FailoverError::Refused(vec![
            "a failback is in progress; wait for it or cancel it".to_string(),
        ]));
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
    pair.failback = None;
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
        cancelled: RefCell<Vec<String>>,
    }

    impl BackupBackend for FakeBackup {
        fn cancel_backup(&self, op: &str) -> Result<(), String> {
            self.cancelled.borrow_mut().push(op.into());
            Ok(())
        }
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
        found: RefCell<Option<String>>,
    }

    impl RestoreBackend for FakeRestore {
        fn find_restore(&self, _b: &str, _ns: &str, _vm: &str) -> Option<String> {
            self.found.borrow().clone()
        }
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

    fn facts_dr(source: SourceState) -> Facts {
        Facts {
            source,
            dr_vm_exists: Some(true),
        }
    }

    fn failed_over_pair() -> DrPair {
        let mut p = replicated_pair();
        let r = FakeRestore::default();
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: true,
        };
        execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap();
        p.failover.as_mut().unwrap().state = "succeeded".into();
        p
    }

    fn fb_req(dry: bool, confirm: Option<&str>) -> FailbackRequest {
        FailbackRequest {
            dry_run: dry,
            confirm: confirm.map(Into::into),
            start: false,
            keep_dr_vm: true,
        }
    }

    fn started_failback(p: &mut DrPair, b: &FakeBackup, s: &Sink) {
        execute_failback(
            p,
            &facts_dr(SourceState::Stopped),
            &fb_req(false, Some("failback web")),
            b,
            s,
            t0(),
        )
        .unwrap();
    }

    #[test]
    fn failback_dry_run_and_confirmation() {
        let mut p = failed_over_pair();
        let (b, s) = (FakeBackup::default(), Sink::default());
        let f = facts_dr(SourceState::Absent);
        // Dry run (the default) changes nothing.
        let out = execute_failback(&mut p, &f, &fb_req(true, None), &b, &s, t0()).unwrap();
        assert!(matches!(out, FailbackOutcome::DryRun(pl) if pl.allowed));
        assert!(p.failback.is_none() && b.enqueued.borrow().is_empty());
        // Real run without the phrase is refused.
        assert!(matches!(
            execute_failback(&mut p, &f, &fb_req(false, Some("nope")), &b, &s, t0()),
            Err(FailoverError::ConfirmationRequired(_))
        ));
        assert!(b.enqueued.borrow().is_empty());
        // With the phrase the DR VM (not the original) is backed up.
        started_failback(&mut p, &b, &s);
        assert_eq!(b.enqueued.borrow()[0].0, "web1-dr");
        assert_eq!(p.failback.as_ref().unwrap().state, FailbackState::BackingUp);
        assert!(s
            .0
            .borrow()
            .iter()
            .any(|(_, m)| m.contains("failback started")));
        // A second start while active is refused.
        assert!(matches!(
            execute_failback(
                &mut p,
                &f,
                &fb_req(false, Some("failback web")),
                &b,
                &s,
                t0()
            ),
            Err(FailoverError::Refused(_))
        ));
    }

    #[test]
    fn failback_refuses_running_unknown_or_unconfirmed_failover() {
        let (b, s) = (FakeBackup::default(), Sink::default());
        for src in [SourceState::Running, SourceState::Unknown("x".into())] {
            let mut p = failed_over_pair();
            let r = execute_failback(
                &mut p,
                &facts_dr(src),
                &fb_req(false, Some("failback web")),
                &b,
                &s,
                t0(),
            );
            assert!(matches!(r, Err(FailoverError::Refused(_))));
            assert!(p.failback.is_none());
        }
        // DR VM missing / unknown.
        let mut p = failed_over_pair();
        for dr in [Some(false), None] {
            let f = Facts {
                source: SourceState::Absent,
                dr_vm_exists: dr,
            };
            assert!(!failback_plan(&p, &f).allowed);
        }
        // Failover still pending, or never happened.
        p.failover.as_mut().unwrap().state = "pending".into();
        assert!(!failback_plan(&p, &facts_dr(SourceState::Absent)).allowed);
        assert!(!failback_plan(&replicated_pair(), &facts_dr(SourceState::Absent)).allowed);
        assert!(b.enqueued.borrow().is_empty());
    }

    #[test]
    fn failback_full_flow_restores_to_original_name() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        started_failback(&mut p, &b, &s);
        let f = facts_dr(SourceState::Absent);
        // Backup still running: nothing restored.
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        assert!(r.calls.borrow().is_empty());
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        {
            let calls = r.calls.borrow();
            assert_eq!(calls.len(), 1);
            // backup op, original namespace and name, not started.
            assert_eq!(
                calls[0],
                ("op-1".into(), "prod".into(), "web1".into(), false)
            );
        }
        assert_eq!(p.failback.as_ref().unwrap().state, FailbackState::Restoring);
        *r.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        let done = t0() + Duration::seconds(60);
        advance_failback(&mut p, &f, &b, &r, &s, done);
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.state, FailbackState::Succeeded);
        assert_eq!(fb.completed_at, Some(done));
        assert!(!dr_vm_cleanup_due(&p), "DR VM is kept by default");
        assert!(s
            .0
            .borrow()
            .iter()
            .any(|(_, m)| m.contains("failback complete")));
        // Resume now works (source exists again) and clears the record.
        resume(
            &mut p,
            &facts(SourceState::Stopped),
            Some("resume web"),
            done,
        )
        .unwrap();
        assert!(p.failback.is_none());
    }

    #[test]
    fn failback_waits_for_stopped_original_and_fails_if_it_runs() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        started_failback(&mut p, &b, &s);
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        // Original exists but is stopped: never overwritten, waits.
        advance_failback(&mut p, &facts_dr(SourceState::Stopped), &b, &r, &s, t0());
        assert!(r.calls.borrow().is_empty());
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.state, FailbackState::BackedUp);
        assert!(fb.note.as_deref().unwrap().contains("never overwrites"));
        // State unreadable: still waits.
        advance_failback(
            &mut p,
            &facts_dr(SourceState::Unknown("net".into())),
            &b,
            &r,
            &s,
            t0(),
        );
        assert!(r.calls.borrow().is_empty());
        assert_eq!(p.failback.as_ref().unwrap().state, FailbackState::BackedUp);
        // Someone started the original: refuse for good.
        advance_failback(&mut p, &facts_dr(SourceState::Running), &b, &r, &s, t0());
        assert!(r.calls.borrow().is_empty());
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.state, FailbackState::Failed);
        assert!(fb.error.as_deref().unwrap().contains("running"));
        assert!(s.0.borrow().iter().any(|(sv, _)| *sv == Severity::Error));
        // A failed failback can be started again with a fresh backup key.
        started_failback(&mut p, &b, &s);
        let keys: Vec<_> = b.enqueued.borrow().iter().map(|e| e.1.clone()).collect();
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn failback_backup_and_restore_failures_are_recorded() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        started_failback(&mut p, &b, &s);
        *b.progress.borrow_mut() = Some(OpProgress::Failed("disk".into()));
        advance_failback(&mut p, &facts_dr(SourceState::Absent), &b, &r, &s, t0());
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.state, FailbackState::Failed);
        assert!(fb.error.as_deref().unwrap().contains("disk"));
        // Restore failure.
        started_failback(&mut p, &b, &s);
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        *r.progress.borrow_mut() = Some(OpProgress::Failed("exists".into()));
        let f = facts_dr(SourceState::Absent);
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.state, FailbackState::Failed);
        assert!(fb.error.as_deref().unwrap().contains("restore failed"));
    }

    #[test]
    fn failback_cancel_rules() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        assert!(cancel_failback(&mut p, &b, &s, t0()).is_err());
        started_failback(&mut p, &b, &s);
        cancel_failback(&mut p, &b, &s, t0()).unwrap();
        assert_eq!(p.failback.as_ref().unwrap().state, FailbackState::Cancelled);
        assert_eq!(b.cancelled.borrow().as_slice(), ["op-1"]);
        // Cancelled failbacks never advance.
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        advance_failback(&mut p, &facts_dr(SourceState::Absent), &b, &r, &s, t0());
        assert!(r.calls.borrow().is_empty());
        assert!(cancel_failback(&mut p, &b, &s, t0()).is_err());
        // Once restoring, cancel is refused.
        started_failback(&mut p, &b, &s);
        advance_failback(&mut p, &facts_dr(SourceState::Absent), &b, &r, &s, t0());
        assert_eq!(p.failback.as_ref().unwrap().state, FailbackState::Restoring);
        assert!(matches!(
            cancel_failback(&mut p, &b, &s, t0()),
            Err(FailoverError::Refused(_))
        ));
        // Resume is refused while a failback is active.
        assert!(matches!(
            resume(
                &mut p,
                &facts(SourceState::Stopped),
                Some("resume web"),
                t0()
            ),
            Err(FailoverError::Refused(_))
        ));
    }

    #[test]
    fn failback_crash_resume_from_persisted_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dr_pairs_fb_test.json");
        PairStore::update(&path, |st| st.add(failed_over_pair())).unwrap();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        PairStore::update(&path, |st| {
            started_failback(st.get_mut("web").unwrap(), &b, &s);
            Ok(())
        })
        .unwrap();
        // "Crash": everything in memory is dropped; only the document remains.
        let (b2, r2) = (FakeBackup::default(), FakeRestore::default());
        *b2.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        let f = facts_dr(SourceState::Absent);
        PairStore::update(&path, |st| {
            advance_failback(st.get_mut("web").unwrap(), &f, &b2, &r2, &s, t0());
            Ok(())
        })
        .unwrap();
        // Crash again, now mid-restore.
        let loaded = PairStore::load_from(&path);
        let fb = loaded.get("web").unwrap().failback.clone().unwrap();
        assert_eq!(fb.state, FailbackState::Restoring);
        assert_eq!(fb.restore_op.as_deref(), Some("rop-1"));
        assert_eq!(fb.dr_backup_op.as_deref(), Some("op-1"));
        *r2.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        PairStore::update(&path, |st| {
            advance_failback(st.get_mut("web").unwrap(), &f, &b2, &r2, &s, t0());
            Ok(())
        })
        .unwrap();
        // Exactly one restore was ever queued across the restarts.
        assert_eq!(r2.calls.borrow().len(), 1);
        assert!(r.calls.borrow().is_empty());
        let done = PairStore::load_from(&path);
        assert_eq!(
            done.get("web").unwrap().failback.as_ref().unwrap().state,
            FailbackState::Succeeded
        );
    }

    #[test]
    fn failback_adopts_restore_queued_before_a_crash() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        started_failback(&mut p, &b, &s);
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        // The restore was queued (and already created a stopped VM) before the
        // state was saved: adopt it, do not wait on the now-existing original.
        *r.found.borrow_mut() = Some("rop-7".into());
        advance_failback(&mut p, &facts_dr(SourceState::Stopped), &b, &r, &s, t0());
        let fb = p.failback.as_ref().unwrap();
        assert_eq!(fb.restore_op.as_deref(), Some("rop-7"));
        assert!(r.calls.borrow().is_empty());
    }

    #[test]
    fn failback_dr_vm_cleanup_only_when_opted_out() {
        let mut p = failed_over_pair();
        let (b, r, s) = (
            FakeBackup::default(),
            FakeRestore::default(),
            Sink::default(),
        );
        let req = FailbackRequest {
            keep_dr_vm: false,
            ..fb_req(false, Some("failback web"))
        };
        execute_failback(&mut p, &facts_dr(SourceState::Absent), &req, &b, &s, t0()).unwrap();
        assert!(!dr_vm_cleanup_due(&p), "not before success");
        *b.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        *r.progress.borrow_mut() = Some(OpProgress::Succeeded { completed: None });
        let f = facts_dr(SourceState::Absent);
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        advance_failback(&mut p, &f, &b, &r, &s, t0());
        assert!(dr_vm_cleanup_due(&p));
        mark_dr_vm_deleted(&mut p);
        assert!(!dr_vm_cleanup_due(&p));
    }

    #[test]
    fn failback_plan_and_resume_rules() {
        let mut p = replicated_pair();
        assert!(!failback_plan(&p, &facts_dr(SourceState::Absent)).allowed);
        let r = FakeRestore::default();
        let req = FailoverRequest {
            dry_run: false,
            confirm: Some("failover web".into()),
            start: true,
        };
        execute_failover(&mut p, &facts(SourceState::Stopped), &req, &r, t0()).unwrap();
        // Failover not yet confirmed complete: blocked.
        assert!(!failback_plan(&p, &facts_dr(SourceState::Absent)).allowed);
        p.failover.as_mut().unwrap().state = "succeeded".into();
        // A stopped original is allowed (the restore waits); a running one is not.
        assert!(failback_plan(&p, &facts_dr(SourceState::Stopped)).allowed);
        assert!(failback_plan(&p, &facts_dr(SourceState::Absent)).allowed);
        assert!(!failback_plan(&p, &facts_dr(SourceState::Running)).allowed);
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
    fn alerts_are_durable_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dr_alerts_test.json");
        let sink = PathSink(path.clone());
        sink.alert(Severity::Error, "zz-test", "RPO breached");
        sink.alert(Severity::Info, "zz-test", "RPO recovered");
        // A fresh read (as after a restart, or from another replica) sees both.
        let alerts = recent_alerts_from(&path);
        assert_eq!(alerts.len(), 2);
        assert!(alerts
            .iter()
            .all(|n| n.source == "dr-replication" && n.message.contains("zz-test")));
        assert!(alerts[0].message.contains("breached"));
    }

    #[test]
    fn alert_store_caps_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dr_alerts_cap_test.json");
        for i in 0..(MAX_ALERTS + 5) {
            PathSink(path.clone()).alert(Severity::Info, "p", &format!("m{i}"));
        }
        let alerts = recent_alerts_from(&path);
        assert_eq!(alerts.len(), MAX_ALERTS);
        assert!(alerts
            .last()
            .unwrap()
            .message
            .ends_with(&format!("m{}", MAX_ALERTS + 4)));
    }
}
