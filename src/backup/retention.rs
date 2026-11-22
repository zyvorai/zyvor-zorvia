// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Retention enforcement for cluster-local backup snapshots.
//!
//! `retention_days` used to be stored as a label and never acted on. The
//! sweep deletes backup snapshots older than their retention, but always
//! keeps the newest snapshot of each VM so a VM is never left with no backup,
//! and skips snapshots a restore is still using.
//!
//! Off-cluster objects are not deleted from here: their lifetime is the
//! Object Lock retention set at upload time, and expiry is a bucket lifecycle
//! rule (Zorvia holds no delete rights on the store).

use crate::snapshots::{SnapshotInfo, SnapshotManager};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

const LABEL_BACKUP: &str = "zorvia.io/backup";
const LABEL_RETENTION_DAYS: &str = "zorvia.io/retention-days";

/// Names of snapshots that are past their retention at `now`.
pub fn select_expired(snapshots: &[SnapshotInfo], now: DateTime<Utc>) -> Vec<String> {
    let mut newest: HashMap<&str, DateTime<Utc>> = HashMap::new();
    for s in snapshots.iter().filter(|s| is_backup(s)) {
        if let Some(t) = s.created_at {
            let e = newest.entry(s.vm_name.as_str()).or_insert(t);
            if t > *e {
                *e = t;
            }
        }
    }
    snapshots
        .iter()
        .filter(|s| is_backup(s))
        .filter_map(|s| {
            let created = s.created_at?;
            let days: i64 = s.labels.get(LABEL_RETENTION_DAYS)?.parse().ok()?;
            if days < 1 || created + Duration::days(days) > now {
                return None;
            }
            // Never delete a VM's newest backup.
            if newest.get(s.vm_name.as_str()) == Some(&created) {
                return None;
            }
            Some(s.name.clone())
        })
        .collect()
}

fn is_backup(s: &SnapshotInfo) -> bool {
    s.labels.get(LABEL_BACKUP).map(String::as_str) == Some("true")
}

/// Delete expired backup snapshots in `namespace`. Returns how many.
pub async fn sweep_local(namespace: &str) -> anyhow::Result<usize> {
    let manager = SnapshotManager::new(namespace).await?;
    let all = manager.list_all_snapshots().await?;
    let mut deleted = 0;
    for name in select_expired(&all, Utc::now()) {
        match manager.is_snapshot_in_use(&name).await {
            Ok(false) => {}
            Ok(true) => continue,
            Err(e) => {
                log::warn!("retention: cannot check {name}: {e}");
                continue;
            }
        }
        match manager.delete_snapshot(&name).await {
            Ok(()) => {
                log::info!("retention: deleted expired backup snapshot {name}");
                deleted += 1;
            }
            Err(e) => log::warn!("retention: cannot delete {name}: {e}"),
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots::SnapshotStatus;

    fn snap(
        name: &str,
        vm: &str,
        age_days: i64,
        retention: Option<&str>,
        backup: bool,
    ) -> SnapshotInfo {
        let mut labels = HashMap::new();
        if backup {
            labels.insert(LABEL_BACKUP.to_string(), "true".to_string());
        }
        if let Some(r) = retention {
            labels.insert(LABEL_RETENTION_DAYS.to_string(), r.to_string());
        }
        SnapshotInfo {
            name: name.into(),
            vm_name: vm.into(),
            namespace: "default".into(),
            status: SnapshotStatus::Succeeded,
            created_at: Some(Utc::now() - Duration::days(age_days)),
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

    #[test]
    fn expires_only_old_backups_with_a_retention() {
        let snaps = vec![
            snap("old", "web", 40, Some("30"), true),
            snap("recent", "web", 5, Some("30"), true),
            snap("newest", "web", 1, Some("30"), true),
        ];
        assert_eq!(select_expired(&snaps, Utc::now()), vec!["old".to_string()]);
    }

    #[test]
    fn never_deletes_the_newest_backup_of_a_vm() {
        let snaps = vec![
            snap("a", "web", 90, Some("30"), true),
            snap("b", "db", 60, Some("7"), true),
            snap("c", "db", 80, Some("7"), true),
        ];
        // web's only backup is kept; db keeps its newest ("b"), drops "c".
        assert_eq!(select_expired(&snaps, Utc::now()), vec!["c".to_string()]);
    }

    #[test]
    fn ignores_snapshots_that_are_not_backups_or_have_no_retention() {
        let snaps = vec![
            snap("manual", "web", 400, Some("1"), false),
            snap("forever", "web", 400, None, true),
            snap("bad-label", "web", 400, Some("soon"), true),
            snap("zero", "web", 400, Some("0"), true),
            snap("keep", "web", 1, Some("30"), true),
        ];
        assert!(select_expired(&snaps, Utc::now()).is_empty());
    }
}
