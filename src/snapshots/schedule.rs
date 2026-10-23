// Scheduled VM snapshots -- cron-like recurring snapshots with per-schedule retention.
//
// Reuses the backup scheduler's `ScheduleType` / `VMSelector` and the shared next-run
// math, so a schedule fires exactly when an equivalent backup schedule would. Snapshots
// a schedule creates are labelled `zorvia.io/scheduled-by=<schedule>`; retention only
// ever touches those, never snapshots a person took by hand.

use super::{RetentionPolicy, SnapshotInfo};
use crate::backup::schedule::{ScheduleType, VMSelector};
use chrono::{DateTime, TimeDelta as Duration, Utc};
use serde::{Deserialize, Serialize};

/// Label stamped on every snapshot a schedule creates.
pub const SCHEDULED_BY_LABEL: &str = "zorvia.io/scheduled-by";

/// Longest schedule name accepted. Snapshot names embed it, and the label value limit is 63.
pub const MAX_SCHEDULE_NAME_LEN: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotSchedule {
    pub name: String,
    pub vm_selector: VMSelector,
    pub schedule_type: ScheduleType,
    pub enabled: bool,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
    /// Why the most recent run failed for at least one VM; cleared by a clean run.
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retention: RetentionPolicy,
}

/// A schedule name must be usable inside a Kubernetes object name and a label value.
pub fn validate_schedule_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_SCHEDULE_NAME_LEN {
        return Err(format!(
            "Schedule name must be 1-{MAX_SCHEDULE_NAME_LEN} characters"
        ));
    }
    let ok_edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let first = name.chars().next().unwrap_or('-');
    let last = name.chars().last().unwrap_or('-');
    if !ok_edge(first) || !ok_edge(last) || !name.chars().all(|c| ok_edge(c) || c == '-') {
        return Err(
            "Schedule name may only use lowercase letters, digits and '-', and must start and end with a letter or digit"
                .to_string(),
        );
    }
    Ok(())
}

impl SnapshotSchedule {
    pub fn new(name: impl Into<String>, schedule_type: ScheduleType) -> Self {
        Self {
            name: name.into(),
            vm_selector: VMSelector::All,
            schedule_type,
            enabled: true,
            next_run: None,
            last_run: None,
            last_error: None,
            retention: RetentionPolicy::default(),
        }
    }

    pub fn calculate_next_run(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        self.schedule_type.next_run(from)
    }

    /// Whether this schedule applies to `vm`. Label selectors are not wired to real VM
    /// labels (same as the backup scheduler), so they match nothing.
    pub fn matches_vm(&self, vm: &str) -> bool {
        match &self.vm_selector {
            VMSelector::All => true,
            VMSelector::ByName(names) => names.iter().any(|n| n == vm),
            VMSelector::ByLabel { .. } => false,
        }
    }

    /// Deterministic snapshot name for one VM in one due slot. Re-running the same slot
    /// (for example after a restart) produces the same name, so the create is idempotent.
    pub fn snapshot_name(&self, vm: &str, slot: DateTime<Utc>) -> String {
        snapshot_name(&self.name, vm, slot)
    }
}

pub fn snapshot_name(schedule: &str, vm: &str, slot: DateTime<Utc>) -> String {
    format!("{vm}-sched-{schedule}-{}", slot.format("%Y%m%d%H%M"))
}

/// Which of `snapshots` this schedule's retention should delete, oldest first.
///
/// Only snapshots labelled as created by `schedule` are considered. The newest
/// `keep_last_n` are always kept; beyond that, a snapshot goes when it is past
/// `max_snapshots` (counting newest first) or older than `max_age_days`.
pub fn snapshots_to_prune(
    snapshots: &[SnapshotInfo],
    schedule: &str,
    policy: &RetentionPolicy,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut mine: Vec<&SnapshotInfo> = snapshots
        .iter()
        .filter(|s| s.labels.get(SCHEDULED_BY_LABEL).map(String::as_str) == Some(schedule))
        .collect();
    mine.sort_by_key(|s| std::cmp::Reverse(s.created_at));

    let keep = policy.keep_last_n.unwrap_or(0) as usize;
    let mut doomed: Vec<String> = mine
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            if *i < keep {
                return false;
            }
            let over_count = policy.max_snapshots.is_some_and(|max| *i >= max as usize);
            let too_old = policy.max_age_days.is_some_and(|days| {
                s.created_at
                    .is_some_and(|t| now - t > Duration::days(days as i64))
            });
            over_count || too_old
        })
        .map(|(_, s)| s.name.clone())
        .collect();
    doomed.reverse();
    doomed
}

#[derive(Serialize, Deserialize, Default)]
pub struct SnapshotScheduleManager {
    schedules: Vec<SnapshotSchedule>,
}

impl SnapshotScheduleManager {
    fn persistence_path() -> std::path::PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("zorvia")
            .join("snapshot_schedules.json")
    }

    pub fn load() -> Self {
        let path = Self::persistence_path();
        if let Some(content) = crate::state_docs::read(&path) {
            match serde_json::from_str(&content) {
                Ok(manager) => return manager,
                Err(e) => log::warn!("Failed to parse snapshot schedules: {}", e),
            }
        }
        Self::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::persistence_path();
        let content = serde_json::to_string_pretty(self)?;
        crate::state_docs::write(&path, &content)
    }

    fn persist(&self) {
        if let Err(e) = self.save() {
            log::warn!("Failed to persist snapshot schedules: {}", e);
        }
    }

    pub fn add_schedule(&mut self, schedule: SnapshotSchedule) {
        self.schedules.push(schedule);
        self.persist();
    }

    pub fn remove_schedule(&mut self, name: &str) -> bool {
        let before = self.schedules.len();
        self.schedules.retain(|s| s.name != name);
        let removed = self.schedules.len() != before;
        if removed {
            self.persist();
        }
        removed
    }

    pub fn get_schedule(&self, name: &str) -> Option<&SnapshotSchedule> {
        self.schedules.iter().find(|s| s.name == name)
    }

    /// Replace the schedule of the same name in place (keeps list order).
    pub fn update_schedule(&mut self, updated: SnapshotSchedule) -> bool {
        match self.schedules.iter_mut().find(|s| s.name == updated.name) {
            Some(slot) => {
                *slot = updated;
                self.persist();
                true
            }
            None => false,
        }
    }

    pub fn all(&self) -> &[SnapshotSchedule] {
        &self.schedules
    }

    pub fn schedules_for_vm(&self, vm: &str) -> Vec<&SnapshotSchedule> {
        self.schedules.iter().filter(|s| s.matches_vm(vm)).collect()
    }

    pub fn get_due_schedules(&self, now: DateTime<Utc>) -> Vec<&SnapshotSchedule> {
        self.schedules
            .iter()
            .filter(|s| s.enabled && s.next_run.is_some_and(|next| next <= now))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots::SnapshotStatus;
    use chrono::TimeZone;
    use std::collections::HashMap;

    fn snap(name: &str, by: Option<&str>, age_days: i64, now: DateTime<Utc>) -> SnapshotInfo {
        let mut labels = HashMap::new();
        if let Some(by) = by {
            labels.insert(SCHEDULED_BY_LABEL.to_string(), by.to_string());
        }
        SnapshotInfo {
            name: name.to_string(),
            vm_name: "vm1".to_string(),
            namespace: "default".to_string(),
            status: SnapshotStatus::Succeeded,
            created_at: Some(now - Duration::days(age_days)),
            completed_at: None,
            description: None,
            size: None,
            labels,
            ready_to_use: true,
            error: None,
            excluded_volumes: vec![],
            total_volumes: 1,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    #[test]
    fn name_validation() {
        assert!(validate_schedule_name("nightly").is_ok());
        assert!(validate_schedule_name("db-6h").is_ok());
        assert!(validate_schedule_name("").is_err());
        assert!(validate_schedule_name("Nightly").is_err());
        assert!(validate_schedule_name("-x").is_err());
        assert!(validate_schedule_name("x-").is_err());
        assert!(validate_schedule_name("a b").is_err());
        assert!(validate_schedule_name(&"a".repeat(41)).is_err());
    }

    #[test]
    fn snapshot_name_is_deterministic_per_slot() {
        let s = SnapshotSchedule::new("nightly", ScheduleType::daily(2, 0));
        let slot = Utc.with_ymd_and_hms(2026, 10, 24, 2, 0, 0).unwrap();
        assert_eq!(
            s.snapshot_name("web1", slot),
            "web1-sched-nightly-202610240200"
        );
        assert_eq!(s.snapshot_name("web1", slot), s.snapshot_name("web1", slot));
        let next = Utc.with_ymd_and_hms(2026, 10, 25, 2, 0, 0).unwrap();
        assert_ne!(s.snapshot_name("web1", slot), s.snapshot_name("web1", next));
    }

    #[test]
    fn selector_matching() {
        let mut s = SnapshotSchedule::new("n", ScheduleType::daily(2, 0));
        assert!(s.matches_vm("anything"));
        s.vm_selector = VMSelector::ByName(vec!["a".into(), "b".into()]);
        assert!(s.matches_vm("a"));
        assert!(!s.matches_vm("c"));
        s.vm_selector = VMSelector::ByLabel {
            key: "k".into(),
            value: "v".into(),
        };
        assert!(!s.matches_vm("a"));
    }

    #[test]
    fn disabled_schedule_has_no_next_run_and_is_never_due() {
        let mut s = SnapshotSchedule::new("n", ScheduleType::hourly(0));
        s.enabled = false;
        assert!(s.calculate_next_run(now()).is_none());
        let mut m = SnapshotScheduleManager::default();
        s.next_run = Some(now() - Duration::hours(1));
        m.schedules.push(s);
        assert!(m.get_due_schedules(now()).is_empty());
    }

    #[test]
    fn due_when_next_run_has_passed() {
        let mut s = SnapshotSchedule::new("n", ScheduleType::hourly(0));
        s.next_run = Some(now() - Duration::minutes(1));
        let mut later = SnapshotSchedule::new("later", ScheduleType::hourly(0));
        later.next_run = Some(now() + Duration::minutes(1));
        let mut m = SnapshotScheduleManager::default();
        m.schedules.push(s);
        m.schedules.push(later);
        let due = m.get_due_schedules(now());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "n");
    }

    #[test]
    fn schedules_for_vm_filters_by_selector() {
        let mut a = SnapshotSchedule::new("a", ScheduleType::daily(1, 0));
        a.vm_selector = VMSelector::ByName(vec!["web1".into()]);
        let b = SnapshotSchedule::new("b", ScheduleType::daily(2, 0));
        let mut m = SnapshotScheduleManager::default();
        m.schedules.push(a);
        m.schedules.push(b);
        assert_eq!(m.schedules_for_vm("web1").len(), 2);
        assert_eq!(m.schedules_for_vm("db1").len(), 1);
    }

    #[test]
    fn retention_never_touches_manual_or_other_schedule_snapshots() {
        let n = now();
        let snaps = vec![
            snap("manual-old", None, 400, n),
            snap("other-old", Some("other"), 400, n),
            snap("mine-new", Some("nightly"), 1, n),
        ];
        let policy = RetentionPolicy {
            max_snapshots: Some(1),
            max_age_days: Some(7),
            keep_last_n: Some(0),
        };
        assert!(snapshots_to_prune(&snaps, "nightly", &policy, n).is_empty());
    }

    #[test]
    fn retention_by_count_keeps_newest_and_returns_oldest_first() {
        let n = now();
        let snaps: Vec<_> = (0..5)
            .map(|i| snap(&format!("s{i}"), Some("nightly"), i, n))
            .collect();
        let policy = RetentionPolicy {
            max_snapshots: Some(3),
            max_age_days: None,
            keep_last_n: Some(1),
        };
        // s0 newest .. s4 oldest; keep newest 3 (s0,s1,s2); prune s4 then s3 is index 3 -> pruned
        assert_eq!(
            snapshots_to_prune(&snaps, "nightly", &policy, n),
            vec!["s4", "s3"]
        );
    }

    #[test]
    fn retention_by_age_respects_keep_last_n() {
        let n = now();
        let snaps = vec![
            snap("a", Some("nightly"), 40, n),
            snap("b", Some("nightly"), 50, n),
            snap("c", Some("nightly"), 60, n),
        ];
        let policy = RetentionPolicy {
            max_snapshots: None,
            max_age_days: Some(30),
            keep_last_n: Some(2),
        };
        // all three are past 30 days, but the newest two are protected
        assert_eq!(snapshots_to_prune(&snaps, "nightly", &policy, n), vec!["c"]);
    }

    #[test]
    fn retention_with_no_limits_prunes_nothing() {
        let n = now();
        let snaps = vec![snap("a", Some("nightly"), 999, n)];
        let policy = RetentionPolicy {
            max_snapshots: None,
            max_age_days: None,
            keep_last_n: None,
        };
        assert!(snapshots_to_prune(&snaps, "nightly", &policy, n).is_empty());
    }

    #[test]
    fn schedule_round_trips_through_json() {
        let mut s = SnapshotSchedule::new("nightly", ScheduleType::daily(2, 30));
        s.vm_selector = VMSelector::ByName(vec!["web1".into()]);
        s.last_error = Some("boom".into());
        let json = serde_json::to_string(&s).unwrap();
        let back: SnapshotSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "nightly");
        assert_eq!(back.last_error.as_deref(), Some("boom"));
        assert!(back.matches_vm("web1"));
    }
}
