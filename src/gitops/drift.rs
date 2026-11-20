// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Semantic drift detection for KubeVirt `VirtualMachine` resources.
//!
//! This module turns the lightweight drift data model in `reconciliation` into
//! an operational engine suitable for local checks, GitOps pipelines and live
//! cluster comparisons. It deliberately ignores Kubernetes-managed noise by
//! default and compares named arrays (disks, interfaces, volumes, networks,
//! etc.) by name so ordering-only changes do not produce false positives.

use super::reconciliation::{DriftDifference, DriftStatus, DriftType, ResourceDrift};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Severity attached to a drift finding.
///
/// Ordering is intentional and is used by CI threshold checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriftSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl DriftSeverity {
    /// Parse a human-friendly severity value.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" | "crit" => Some(Self::Critical),
            _ => None,
        }
    }

    pub fn weight(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Low => 2,
            Self::Medium => 5,
            Self::High => 12,
            Self::Critical => 25,
        }
    }
}

impl std::fmt::Display for DriftSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        };
        f.write_str(value)
    }
}

/// A semantic difference enriched with operational risk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftFinding {
    /// RFC 6901-style JSON pointer.
    pub path: String,
    pub diff_type: DriftType,
    pub desired_value: Option<String>,
    pub actual_value: Option<String>,
    pub severity: DriftSeverity,
    pub reason: String,
}

/// Summary counts for a drift report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DriftSummary {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub info: usize,
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    pub critical: usize,
}

impl DriftSummary {
    pub fn total(&self) -> usize {
        self.added + self.removed + self.modified
    }
}

/// Complete drift assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    /// Existing Zorvia drift model, populated by the semantic engine.
    pub resource: ResourceDrift,
    /// Risk-aware findings corresponding to `resource.differences`.
    pub findings: Vec<DriftFinding>,
    /// Bounded 0-100 risk score.
    pub risk_score: u8,
    /// Highest finding severity. `None` means the resource is in sync.
    pub highest_severity: Option<DriftSeverity>,
    pub summary: DriftSummary,
}

impl DriftReport {
    pub fn is_drifted(&self) -> bool {
        self.resource.status == DriftStatus::Drifted
    }

    /// Return true when at least one finding meets or exceeds `threshold`.
    pub fn meets_threshold(&self, threshold: DriftSeverity) -> bool {
        self.findings.iter().any(|f| f.severity >= threshold)
    }
}

/// Comparison behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftOptions {
    /// JSON pointers to ignore. A pointer ending in `/*` ignores that subtree.
    pub ignore_paths: Vec<String>,
    /// Kubernetes status is ignored by default because it is controller-owned.
    pub include_status: bool,
    /// Remove standard server-managed metadata fields.
    pub normalize_kubernetes_metadata: bool,
}

impl Default for DriftOptions {
    fn default() -> Self {
        Self {
            ignore_paths: Vec::new(),
            include_status: false,
            normalize_kubernetes_metadata: true,
        }
    }
}

/// Semantic drift engine.
pub struct DriftEngine {
    options: DriftOptions,
}

impl DriftEngine {
    pub fn new(options: DriftOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &DriftOptions {
        &self.options
    }

    /// Parse either YAML or JSON into a JSON value.
    pub fn parse_manifest(content: &str) -> Result<Value> {
        serde_yaml::from_str::<Value>(content).context("failed to parse manifest as YAML/JSON")
    }

    /// Read and parse a manifest file.
    pub fn load_manifest(path: &Path) -> Result<Value> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest '{}'", path.display()))?;
        Self::parse_manifest(&content)
            .with_context(|| format!("failed to parse manifest '{}'", path.display()))
    }

    /// Compare desired and actual resources semantically.
    pub fn compare(&self, desired: &Value, actual: &Value) -> DriftReport {
        let actual_revision = string_pointer(actual, "/metadata/resourceVersion")
            .unwrap_or_else(|| "live".to_string());
        let mut desired = desired.clone();
        let mut actual = actual.clone();

        self.normalize(&mut desired);
        self.normalize(&mut actual);

        let name = string_pointer(&desired, "/metadata/name")
            .or_else(|| string_pointer(&actual, "/metadata/name"))
            .unwrap_or_else(|| "unknown".to_string());
        let kind = string_pointer(&desired, "/kind")
            .or_else(|| string_pointer(&actual, "/kind"))
            .unwrap_or_else(|| "Unknown".to_string());
        let namespace = string_pointer(&desired, "/metadata/namespace")
            .or_else(|| string_pointer(&actual, "/metadata/namespace"))
            .unwrap_or_else(|| "default".to_string());
        let mut resource = ResourceDrift::new(name, kind, namespace, "desired", actual_revision);
        let mut findings = Vec::new();

        self.diff_value("", &desired, &actual, &mut findings);

        for finding in &findings {
            let difference = match &finding.diff_type {
                DriftType::Added => DriftDifference::added(
                    finding.path.clone(),
                    finding.actual_value.clone().unwrap_or_default(),
                ),
                DriftType::Removed => DriftDifference::removed(
                    finding.path.clone(),
                    finding.desired_value.clone().unwrap_or_default(),
                ),
                DriftType::Modified => DriftDifference::modified(
                    finding.path.clone(),
                    finding.desired_value.clone().unwrap_or_default(),
                    finding.actual_value.clone().unwrap_or_default(),
                ),
            };
            resource.add_difference(difference);
        }

        let mut summary = DriftSummary::default();
        let mut score: u16 = 0;
        let mut highest = None;

        for finding in &findings {
            match &finding.diff_type {
                DriftType::Added => summary.added += 1,
                DriftType::Removed => summary.removed += 1,
                DriftType::Modified => summary.modified += 1,
            }
            match finding.severity {
                DriftSeverity::Info => summary.info += 1,
                DriftSeverity::Low => summary.low += 1,
                DriftSeverity::Medium => summary.medium += 1,
                DriftSeverity::High => summary.high += 1,
                DriftSeverity::Critical => summary.critical += 1,
            }
            score += u16::from(finding.severity.weight());
            highest = Some(highest.map_or(finding.severity, |current: DriftSeverity| {
                current.max(finding.severity)
            }));
        }

        DriftReport {
            resource,
            findings,
            risk_score: score.min(100) as u8,
            highest_severity: highest,
            summary,
        }
    }

    fn normalize(&self, value: &mut Value) {
        if self.options.normalize_kubernetes_metadata {
            for pointer in [
                "/metadata/creationTimestamp",
                "/metadata/deletionGracePeriodSeconds",
                "/metadata/deletionTimestamp",
                "/metadata/generation",
                "/metadata/managedFields",
                "/metadata/resourceVersion",
                "/metadata/selfLink",
                "/metadata/uid",
                "/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration",
                "/metadata/annotations/kubevirt.io~1latest-observed-api-version",
                "/metadata/annotations/kubevirt.io~1storage-observed-api-version",
            ] {
                remove_pointer(value, pointer);
            }
        }

        if !self.options.include_status {
            remove_pointer(value, "/status");
        }

        sort_named_arrays(value);
    }

    fn diff_value(
        &self,
        path: &str,
        desired: &Value,
        actual: &Value,
        findings: &mut Vec<DriftFinding>,
    ) {
        if self.is_ignored(path) {
            return;
        }

        match (desired, actual) {
            (Value::Object(desired_map), Value::Object(actual_map)) => {
                let keys: BTreeSet<_> = desired_map.keys().chain(actual_map.keys()).collect();
                for key in keys {
                    let child = push_pointer(path, key);
                    if self.is_ignored(&child) {
                        continue;
                    }
                    match (desired_map.get(key), actual_map.get(key)) {
                        (Some(d), Some(a)) => self.diff_value(&child, d, a, findings),
                        (Some(d), None) => {
                            findings.push(self.finding(child, DriftType::Removed, Some(d), None))
                        }
                        (None, Some(a)) => {
                            findings.push(self.finding(child, DriftType::Added, None, Some(a)))
                        }
                        (None, None) => {}
                    }
                }
            }
            (Value::Array(desired_items), Value::Array(actual_items)) => {
                let max_len = desired_items.len().max(actual_items.len());
                for index in 0..max_len {
                    let child = push_pointer(path, &index.to_string());
                    if self.is_ignored(&child) {
                        continue;
                    }
                    match (desired_items.get(index), actual_items.get(index)) {
                        (Some(d), Some(a)) => self.diff_value(&child, d, a, findings),
                        (Some(d), None) => {
                            findings.push(self.finding(child, DriftType::Removed, Some(d), None))
                        }
                        (None, Some(a)) => {
                            findings.push(self.finding(child, DriftType::Added, None, Some(a)))
                        }
                        (None, None) => {}
                    }
                }
            }
            _ if desired == actual => {}
            _ => findings.push(self.finding(
                if path.is_empty() {
                    "/".to_string()
                } else {
                    path.to_string()
                },
                DriftType::Modified,
                Some(desired),
                Some(actual),
            )),
        }
    }

    fn finding(
        &self,
        path: String,
        diff_type: DriftType,
        desired: Option<&Value>,
        actual: Option<&Value>,
    ) -> DriftFinding {
        let (severity, reason) = classify(&path, &diff_type);
        DriftFinding {
            path,
            diff_type,
            desired_value: desired.map(value_to_string),
            actual_value: actual.map(value_to_string),
            severity,
            reason: reason.to_string(),
        }
    }

    fn is_ignored(&self, path: &str) -> bool {
        self.options.ignore_paths.iter().any(|raw| {
            let ignore = normalize_pointer(raw);
            if ignore.ends_with("/*") {
                let prefix = ignore.trim_end_matches("/*");
                path == prefix || path.starts_with(&format!("{}/", prefix))
            } else {
                path == ignore
            }
        })
    }
}

fn classify(path: &str, diff_type: &DriftType) -> (DriftSeverity, &'static str) {
    let p = path.to_ascii_lowercase();

    if p.contains("/spec/template/spec/volumes")
        || p.contains("/spec/template/spec/domain/devices/disks")
        || p.contains("/datavolumetemplates")
    {
        let severity = if matches!(diff_type, DriftType::Removed | DriftType::Modified) {
            DriftSeverity::Critical
        } else {
            DriftSeverity::High
        };
        return (severity, "storage topology or disk source changed");
    }

    if p.contains("/spec/template/spec/domain/firmware")
        || p.contains("/spec/template/spec/domain/machine")
        || p.contains("/spec/template/spec/domain/devices/interfaces")
        || p.contains("/spec/template/spec/networks")
        || p.contains("/spec/template/spec/nodeselector")
        || p.contains("/spec/template/spec/affinity")
        || p.contains("/spec/template/spec/evictionstrategy")
    {
        return (
            DriftSeverity::High,
            "placement, firmware or network topology changed",
        );
    }

    if p.contains("/spec/template/spec/domain/cpu")
        || p.contains("/spec/template/spec/domain/memory")
        || p.contains("/spec/template/spec/domain/resources")
        || p.ends_with("/spec/running")
        || p.ends_with("/spec/runstrategy")
        || p.contains("/terminationgraceperiodseconds")
    {
        return (
            DriftSeverity::Medium,
            "runtime resources or lifecycle behavior changed",
        );
    }

    if p.starts_with("/metadata/labels") || p.starts_with("/metadata/annotations") {
        return (DriftSeverity::Low, "metadata changed");
    }

    (DriftSeverity::Info, "declarative field changed")
}

fn string_pointer(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn normalize_pointer(pointer: &str) -> String {
    let trimmed = pointer.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return trimmed.to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.trim_end_matches('/').to_string()
    } else {
        format!("/{}", trimmed.trim_end_matches('/'))
    }
}

fn push_pointer(parent: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    if parent.is_empty() {
        format!("/{}", escaped)
    } else {
        format!("{}/{}", parent, escaped)
    }
}

fn decode_pointer_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

fn remove_pointer(root: &mut Value, pointer: &str) -> bool {
    let pointer = normalize_pointer(pointer);
    if pointer.is_empty() || pointer == "/" {
        return false;
    }

    let mut segments = pointer
        .trim_start_matches('/')
        .split('/')
        .map(decode_pointer_segment)
        .collect::<Vec<_>>();
    let Some(last) = segments.pop() else {
        return false;
    };

    let mut current = root;
    for segment in segments {
        match current {
            Value::Object(map) => {
                let Some(next) = map.get_mut(&segment) else {
                    return false;
                };
                current = next;
            }
            Value::Array(items) => {
                let Ok(index) = segment.parse::<usize>() else {
                    return false;
                };
                let Some(next) = items.get_mut(index) else {
                    return false;
                };
                current = next;
            }
            _ => return false,
        }
    }

    match current {
        Value::Object(map) => map.remove(&last).is_some(),
        Value::Array(items) => last
            .parse::<usize>()
            .ok()
            .filter(|index| *index < items.len())
            .map(|index| {
                items.remove(index);
                true
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn sort_named_arrays(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                sort_named_arrays(child);
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                sort_named_arrays(child);
            }

            if !items.is_empty()
                && items.iter().all(|item| {
                    item.as_object()
                        .and_then(|obj| obj.get("name"))
                        .and_then(Value::as_str)
                        .is_some()
                })
            {
                items.sort_by(|a, b| {
                    let a_name = a
                        .as_object()
                        .and_then(|obj| obj.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let b_name = b
                        .as_object()
                        .and_then(|obj| obj.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    a_name.cmp(b_name)
                });
            }
        }
        _ => {}
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine() -> DriftEngine {
        DriftEngine::new(DriftOptions::default())
    }

    #[test]
    fn identical_resources_are_in_sync() {
        let manifest = json!({
            "apiVersion": "kubevirt.io/v1",
            "kind": "VirtualMachine",
            "metadata": {"name": "vm1", "namespace": "default"},
            "spec": {"running": true}
        });

        let report = engine().compare(&manifest, &manifest);
        assert!(!report.is_drifted());
        assert_eq!(report.summary.total(), 0);
        assert_eq!(report.risk_score, 0);
        assert_eq!(report.highest_severity, None);
    }

    #[test]
    fn kubernetes_managed_metadata_does_not_create_drift() {
        let desired = json!({
            "kind": "VirtualMachine",
            "metadata": {"name": "vm1"},
            "spec": {"running": true}
        });
        let actual = json!({
            "kind": "VirtualMachine",
            "metadata": {
                "name": "vm1",
                "uid": "generated",
                "resourceVersion": "42",
                "generation": 7,
                "creationTimestamp": "2026-09-09T00:00:00Z",
                "managedFields": [{"manager": "kube-controller"}]
            },
            "spec": {"running": true},
            "status": {"ready": true}
        });

        let report = engine().compare(&desired, &actual);
        assert!(!report.is_drifted());
    }

    #[test]
    fn status_can_be_included_explicitly() {
        let desired =
            json!({"kind":"VirtualMachine","metadata":{"name":"vm1"},"status":{"ready":false}});
        let actual =
            json!({"kind":"VirtualMachine","metadata":{"name":"vm1"},"status":{"ready":true}});
        let options = DriftOptions {
            include_status: true,
            ..DriftOptions::default()
        };
        let report = DriftEngine::new(options).compare(&desired, &actual);
        assert!(report.is_drifted());
        assert_eq!(report.summary.modified, 1);
    }

    #[test]
    fn cpu_change_is_medium_risk() {
        let desired = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"domain":{"cpu":{"cores":2}}}}}
        });
        let actual = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"domain":{"cpu":{"cores":4}}}}}
        });

        let report = engine().compare(&desired, &actual);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, DriftSeverity::Medium);
        assert!(report.meets_threshold(DriftSeverity::Medium));
        assert!(!report.meets_threshold(DriftSeverity::High));
    }

    #[test]
    fn disk_change_is_critical_risk() {
        let desired = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"domain":{"devices":{"disks":[{"name":"root","disk":{"bus":"virtio"}}]}}}}}
        });
        let actual = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"domain":{"devices":{"disks":[{"name":"root","disk":{"bus":"sata"}}]}}}}}
        });

        let report = engine().compare(&desired, &actual);
        assert_eq!(report.highest_severity, Some(DriftSeverity::Critical));
        assert!(report.risk_score >= DriftSeverity::Critical.weight());
    }

    #[test]
    fn named_array_order_is_normalized() {
        let desired = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"volumes":[{"name":"root"},{"name":"cloudinit"}]}}}
        });
        let actual = json!({
            "kind":"VirtualMachine","metadata":{"name":"vm1"},
            "spec":{"template":{"spec":{"volumes":[{"name":"cloudinit"},{"name":"root"}]}}}
        });

        let report = engine().compare(&desired, &actual);
        assert!(!report.is_drifted());
    }

    #[test]
    fn ignore_exact_pointer_suppresses_finding() {
        let desired = json!({"metadata":{"name":"vm1","labels":{"build":"1"}}});
        let actual = json!({"metadata":{"name":"vm1","labels":{"build":"2"}}});
        let options = DriftOptions {
            ignore_paths: vec!["/metadata/labels/build".to_string()],
            ..DriftOptions::default()
        };

        let report = DriftEngine::new(options).compare(&desired, &actual);
        assert!(!report.is_drifted());
    }

    #[test]
    fn ignore_subtree_suppresses_all_children() {
        let desired = json!({"metadata":{"name":"vm1","annotations":{"a":"1","b":"2"}}});
        let actual = json!({"metadata":{"name":"vm1","annotations":{"a":"3","b":"4"}}});
        let options = DriftOptions {
            ignore_paths: vec!["/metadata/annotations/*".to_string()],
            ..DriftOptions::default()
        };

        let report = DriftEngine::new(options).compare(&desired, &actual);
        assert!(!report.is_drifted());
    }

    #[test]
    fn added_and_removed_fields_are_reported() {
        let desired = json!({"metadata":{"name":"vm1"},"spec":{"running":true,"alpha":1}});
        let actual = json!({"metadata":{"name":"vm1"},"spec":{"running":true,"beta":2}});

        let report = engine().compare(&desired, &actual);
        assert_eq!(report.summary.added, 1);
        assert_eq!(report.summary.removed, 1);
        assert_eq!(report.summary.total(), 2);
    }

    #[test]
    fn parses_yaml_and_json() {
        let yaml = "kind: VirtualMachine\nmetadata:\n  name: vm1\n";
        let json = r#"{"kind":"VirtualMachine","metadata":{"name":"vm1"}}"#;

        assert_eq!(
            DriftEngine::parse_manifest(yaml).unwrap()["metadata"]["name"],
            "vm1"
        );
        assert_eq!(
            DriftEngine::parse_manifest(json).unwrap()["metadata"]["name"],
            "vm1"
        );
    }

    #[test]
    fn pointer_escaping_handles_slashes_and_tildes() {
        assert_eq!(
            push_pointer("/metadata/labels", "app.kubernetes.io/name"),
            "/metadata/labels/app.kubernetes.io~1name"
        );
        assert_eq!(push_pointer("", "a~b"), "/a~0b");
    }

    #[test]
    fn remove_pointer_decodes_json_pointer_segments() {
        let mut value = json!({"metadata":{"annotations":{"a/b":"x"}}});
        assert!(remove_pointer(&mut value, "/metadata/annotations/a~1b"));
        assert!(value["metadata"]["annotations"].get("a/b").is_none());
    }
}
