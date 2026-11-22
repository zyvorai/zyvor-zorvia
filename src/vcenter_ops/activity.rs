// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivitySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityKind {
    Task,
    Event,
    Alarm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityRecord {
    pub timestamp: String,
    pub namespace: String,
    pub kind: ActivityKind,
    pub severity: ActivitySeverity,
    pub reason: String,
    pub resource_kind: String,
    pub resource_name: String,
    pub message: String,
    pub count: i32,
}

impl ActivityRecord {
    pub fn classify(
        event_type: Option<&str>,
        reason: Option<&str>,
    ) -> (ActivityKind, ActivitySeverity) {
        let typ = event_type.unwrap_or_default().to_ascii_lowercase();
        let reason = reason.unwrap_or_default().to_ascii_lowercase();
        let severity = if typ == "warning" || reason.contains("failed") || reason.contains("error")
        {
            ActivitySeverity::Error
        } else if reason.contains("backoff")
            || reason.contains("unhealthy")
            || reason.contains("warning")
        {
            ActivitySeverity::Warning
        } else {
            ActivitySeverity::Info
        };
        let kind = if severity >= ActivitySeverity::Warning {
            ActivityKind::Alarm
        } else if [
            "create",
            "created",
            "start",
            "started",
            "stop",
            "stopped",
            "migrate",
            "migrated",
            "delete",
            "deleted",
            "scheduled",
            "successfulcreate",
        ]
        .iter()
        .any(|v| reason.contains(v))
        {
            ActivityKind::Task
        } else {
            ActivityKind::Event
        };
        (kind, severity)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub target: Option<String>,
    pub min_severity: Option<ActivitySeverity>,
    pub kind: Option<ActivityKind>,
}

pub fn filter_and_limit(
    mut rows: Vec<ActivityRecord>,
    filter: &ActivityFilter,
    limit: usize,
) -> Vec<ActivityRecord> {
    rows.retain(|r| {
        filter.target.as_ref().is_none_or(|t| &r.resource_name == t)
            && filter.min_severity.is_none_or(|s| r.severity >= s)
            && filter.kind.is_none_or(|k| r.kind == k)
    });
    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(name: &str, ts: &str, sev: ActivitySeverity) -> ActivityRecord {
        ActivityRecord {
            timestamp: ts.into(),
            namespace: "default".into(),
            kind: ActivityKind::Event,
            severity: sev,
            reason: "Seen".into(),
            resource_kind: "VirtualMachine".into(),
            resource_name: name.into(),
            message: "m".into(),
            count: 1,
        }
    }
    #[test]
    fn warning_event_becomes_alarm() {
        let (k, s) = ActivityRecord::classify(Some("Warning"), Some("FailedScheduling"));
        assert_eq!(k, ActivityKind::Alarm);
        assert_eq!(s, ActivitySeverity::Error);
    }
    #[test]
    fn migration_becomes_task() {
        let (k, s) = ActivityRecord::classify(Some("Normal"), Some("Migrated"));
        assert_eq!(k, ActivityKind::Task);
        assert_eq!(s, ActivitySeverity::Info);
    }
    #[test]
    fn newest_first_and_limit() {
        let out = filter_and_limit(
            vec![
                r("a", "2026-01-01", ActivitySeverity::Info),
                r("b", "2026-02-01", ActivitySeverity::Info),
            ],
            &ActivityFilter::default(),
            1,
        );
        assert_eq!(out[0].resource_name, "b");
    }
    #[test]
    fn severity_filter() {
        let f = ActivityFilter {
            min_severity: Some(ActivitySeverity::Warning),
            ..Default::default()
        };
        assert_eq!(
            filter_and_limit(
                vec![
                    r("a", "1", ActivitySeverity::Info),
                    r("b", "2", ActivitySeverity::Error)
                ],
                &f,
                10
            )
            .len(),
            1
        );
    }
}
