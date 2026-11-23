// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use crate::config::VMConfig;
use crate::gitops::drift::{DriftEngine, DriftOptions, DriftReport, DriftSeverity};
use crate::gitops::lint::{lint_vm, LintReport, LintSeverity};
use crate::kube::{vm_config_to_kubevirt, KubeClient};
use crate::tui::colors::cli as color;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::Path;

/// Compare a desired VM manifest with either a second file or the live cluster.
#[allow(clippy::too_many_arguments)]
pub async fn handle_drift(
    desired: String,
    actual: Option<String>,
    vm: Option<String>,
    ignore: Vec<String>,
    include_status: bool,
    fail_on: String,
    output: String,
    cli_namespace: &str,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let desired_value = load_vm_manifest(Path::new(&desired))?;

    let (actual_value, source_label) = if let Some(actual_path) = actual {
        (load_vm_manifest(Path::new(&actual_path))?, actual_path)
    } else {
        let name = vm
            .or_else(|| manifest_name(&desired_value))
            .ok_or_else(|| {
                anyhow!(
                    "unable to determine VM name; set metadata.name in the manifest or pass --vm"
                )
            })?;
        let namespace = manifest_namespace(&desired_value)
            .filter(|ns| !ns.trim().is_empty())
            .unwrap_or_else(|| cli_namespace.to_string());

        let client = match kubeconfig {
            Some(path) => KubeClient::with_kubeconfig(path)
                .await
                .with_context(|| format!("failed to load kubeconfig '{}'", path))?,
            None => KubeClient::new()
                .await
                .context("failed to create Kubernetes client")?,
        };
        let live = client
            .get_vm(&namespace, &name)
            .await
            .with_context(|| format!("failed to get VirtualMachine {}/{}", namespace, name))?;
        (
            serde_json::to_value(live).context("failed to serialize live VirtualMachine")?,
            format!("cluster:{}/{}", namespace, name),
        )
    };

    let options = DriftOptions {
        ignore_paths: ignore,
        include_status,
        normalize_kubernetes_metadata: true,
    };
    let engine = DriftEngine::new(options);
    let report = engine.compare(&desired_value, &actual_value);

    render_report(&report, &desired, &source_label, &output)?;

    if let Some(threshold) = parse_fail_threshold(&fail_on)? {
        if report.meets_threshold(threshold) {
            let highest = report
                .highest_severity
                .map(|s| s.to_string())
                .unwrap_or_else(|| "none".to_string());
            return Err(anyhow!(
                "drift gate failed: highest severity '{}' meets --fail-on {}",
                highest,
                threshold
            ));
        }
    }

    Ok(())
}

fn load_vm_manifest(path: &Path) -> Result<Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest '{}'", path.display()))?;
    let raw = DriftEngine::parse_manifest(&content)
        .with_context(|| format!("failed to parse manifest '{}'", path.display()))?;

    // Native KubeVirt VirtualMachine manifests can be compared directly.
    if raw.get("kind").and_then(Value::as_str) == Some("VirtualMachine") {
        return Ok(raw);
    }

    // Zorvia VMConfig is also accepted. Converting both input forms to the
    // KubeVirt representation gives the drift engine one canonical shape.
    let config: VMConfig = serde_yaml::from_str(&content).with_context(|| {
        format!(
            "'{}' is neither a KubeVirt VirtualMachine nor a valid Zorvia VMConfig",
            path.display()
        )
    })?;
    let vm = vm_config_to_kubevirt(&config)
        .with_context(|| format!("failed to convert '{}' to KubeVirt", path.display()))?;
    serde_json::to_value(vm).context("failed to serialize converted VirtualMachine")
}

fn manifest_name(value: &Value) -> Option<String> {
    value
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn manifest_namespace(value: &Value) -> Option<String> {
    value
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_fail_threshold(value: &str) -> Result<Option<DriftSeverity>> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "never" => Ok(None),
        other => DriftSeverity::parse(other).map(Some).ok_or_else(|| {
            anyhow!(
                "invalid --fail-on '{}'; expected one of: none, info, low, medium, high, critical",
                value
            )
        }),
    }
}

fn render_report(
    report: &DriftReport,
    desired_source: &str,
    actual_source: &str,
    output: &str,
) -> Result<()> {
    match output.to_ascii_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(report)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(report)?),
        "table" | "text" => render_table(report, desired_source, actual_source),
        other => {
            return Err(anyhow!(
                "unsupported drift output '{}'; expected table, json, or yaml",
                other
            ))
        }
    }
    Ok(())
}

fn render_table(report: &DriftReport, desired_source: &str, actual_source: &str) {
    println!("{}", color::header("Zorvia Drift Guard"));
    println!();
    println!(
        "  Resource:   {}",
        color::value(&report.resource.full_name())
    );
    println!("  Desired:    {}", desired_source);
    println!("  Actual:     {}", actual_source);
    println!(
        "  State:      {}",
        if report.is_drifted() {
            color::warning("DRIFTED")
        } else {
            color::success("IN SYNC")
        }
    );
    println!("  Risk score: {}/100", report.risk_score);
    println!(
        "  Highest:    {}",
        report
            .highest_severity
            .map(|severity| severity.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!();

    if report.findings.is_empty() {
        println!("{}", color::success("✓ No semantic drift detected"));
        return;
    }

    println!("  {:<10} {:<10} {:<52} REASON", "SEVERITY", "TYPE", "PATH");
    println!("  {}", "-".repeat(105));
    for finding in &report.findings {
        let severity = finding.severity.to_string();
        let rendered_severity = match finding.severity {
            DriftSeverity::Critical | DriftSeverity::High => color::error(&severity),
            DriftSeverity::Medium => color::warning(&severity),
            DriftSeverity::Low => color::info(&severity),
            DriftSeverity::Info => color::muted(&severity),
        };
        println!(
            "  {:<10} {:<10} {:<52} {}",
            rendered_severity,
            format!("{:?}", finding.diff_type).to_ascii_lowercase(),
            truncate(&finding.path, 52),
            finding.reason
        );
        if finding.desired_value != finding.actual_value {
            println!(
                "    desired: {}",
                truncate(finding.desired_value.as_deref().unwrap_or("<missing>"), 90)
            );
            println!(
                "    actual:  {}",
                truncate(finding.actual_value.as_deref().unwrap_or("<missing>"), 90)
            );
        }
    }

    println!();
    println!(
        "{}",
        color::info(&format!(
            "Summary: {} changed ({} added, {} removed, {} modified) | risk {}/100",
            report.summary.total(),
            report.summary.added,
            report.summary.removed,
            report.summary.modified,
            report.risk_score
        ))
    );
}

fn truncate(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push('…');
    truncated
}

/// Build an operational execution plan from the same semantic comparison used by Drift Guard.
#[allow(clippy::too_many_arguments)]
pub async fn handle_change_plan(
    desired: String,
    actual: Option<String>,
    vm: Option<String>,
    ignore: Vec<String>,
    include_status: bool,
    fail_on_downtime: bool,
    fail_on_recreate: bool,
    output: String,
    cli_namespace: &str,
    kubeconfig: Option<&str>,
) -> Result<()> {
    use crate::change_plan::{ChangeDisposition, ChangePlanner};

    let desired_value = load_vm_manifest(Path::new(&desired))?;

    let (actual_value, source_label) = if let Some(actual_path) = actual {
        (load_vm_manifest(Path::new(&actual_path))?, actual_path)
    } else {
        let name = vm
            .or_else(|| manifest_name(&desired_value))
            .ok_or_else(|| {
                anyhow!(
                    "unable to determine VM name; set metadata.name in the manifest or pass --vm"
                )
            })?;
        let namespace = manifest_namespace(&desired_value)
            .filter(|ns| !ns.trim().is_empty())
            .unwrap_or_else(|| cli_namespace.to_string());

        let client = match kubeconfig {
            Some(path) => KubeClient::with_kubeconfig(path)
                .await
                .with_context(|| format!("failed to load kubeconfig '{}'", path))?,
            None => KubeClient::new()
                .await
                .context("failed to create Kubernetes client")?,
        };
        let live = client
            .get_vm(&namespace, &name)
            .await
            .with_context(|| format!("failed to get VirtualMachine {}/{}", namespace, name))?;
        (
            serde_json::to_value(live).context("failed to serialize live VirtualMachine")?,
            format!("cluster:{}/{}", namespace, name),
        )
    };

    let report = DriftEngine::new(DriftOptions {
        ignore_paths: ignore,
        include_status,
        normalize_kubernetes_metadata: true,
    })
    .compare(&desired_value, &actual_value);
    let plan = ChangePlanner::plan(&report);

    render_change_plan(&plan, &desired, &source_label, &output)?;

    if fail_on_recreate && plan.requires_recreation {
        return Err(anyhow!(
            "change-plan gate failed: reconciliation requires VM/VMI recreation"
        ));
    }
    if fail_on_downtime && plan.requires_downtime {
        return Err(anyhow!(
            "change-plan gate failed: reconciliation requires planned downtime"
        ));
    }

    // Manual review is deliberately visible but is not an automatic failure:
    // callers can use JSON/YAML output to implement organization-specific policy.
    if plan.overall_disposition == ChangeDisposition::ManualReview {
        log::warn!("change plan contains operations that require manual review");
    }

    Ok(())
}

fn render_change_plan(
    plan: &crate::change_plan::ChangePlan,
    desired_source: &str,
    actual_source: &str,
    output: &str,
) -> Result<()> {
    match output.to_ascii_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(plan)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(plan)?),
        "table" | "text" => render_change_plan_table(plan, desired_source, actual_source),
        other => {
            return Err(anyhow!(
                "unsupported change-plan output '{}'; expected table, json, or yaml",
                other
            ))
        }
    }
    Ok(())
}

fn render_change_plan_table(
    plan: &crate::change_plan::ChangePlan,
    desired_source: &str,
    actual_source: &str,
) {
    println!("{}", color::header("Zorvia Change Planner"));
    println!();
    println!("  Resource:     {}", color::value(&plan.resource));
    println!("  Desired:      {}", desired_source);
    println!("  Actual:       {}", actual_source);
    println!("  Risk score:   {}/100", plan.risk_score);
    println!("  Disposition:  {}", plan.overall_disposition);
    println!(
        "  Downtime:     {}",
        if plan.requires_downtime {
            "required"
        } else {
            "not expected"
        }
    );
    println!(
        "  Recreation:   {}",
        if plan.requires_recreation {
            "required"
        } else {
            "no"
        }
    );
    println!();

    if plan.is_noop() {
        println!("{}", color::success("✓ No change required"));
        return;
    }

    println!(
        "  {:<18} {:<10} {:<50} ACTION",
        "IMPACT", "SEVERITY", "PATH"
    );
    println!("  {}", "-".repeat(110));
    for change in &plan.changes {
        println!(
            "  {:<18} {:<10} {:<50} {}",
            change.disposition,
            change.severity,
            truncate(&change.path, 50),
            change.action
        );
    }

    print_plan_section("Preflight", &plan.preflight_checks);
    print_plan_section("Execution", &plan.execution_steps);
    print_plan_section("Postflight", &plan.postflight_checks);
}

fn print_plan_section(title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!();
    println!("{}", color::label(&format!("{}:", title)));
    for item in items {
        println!("  - {}", item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_threshold_accepts_none_and_levels() {
        assert!(parse_fail_threshold("none").unwrap().is_none());
        assert_eq!(
            parse_fail_threshold("high").unwrap(),
            Some(DriftSeverity::High)
        );
        assert!(parse_fail_threshold("banana").is_err());
    }

    #[test]
    fn truncate_is_unicode_safe() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("αβγδε", 4), "αβγ…");
    }
}

/// Largest manifest the linter will read from disk.
const MAX_LINT_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Lint one or more VM manifests offline (no cluster access).
pub fn handle_lint(files: Vec<String>, fail_on: String, output: String) -> Result<()> {
    let threshold = match fail_on.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "never" => None,
        other => Some(LintSeverity::parse(other).ok_or_else(|| {
            anyhow!(
                "invalid --fail-on '{}'; expected one of: none, info, low, medium, high",
                fail_on
            )
        })?),
    };
    let output = output.to_ascii_lowercase();
    if !matches!(output.as_str(), "table" | "text" | "json" | "yaml" | "yml") {
        return Err(anyhow!(
            "unsupported lint output '{}'; expected table, json, or yaml",
            output
        ));
    }

    let mut reports: Vec<(String, LintReport)> = Vec::new();
    for file in &files {
        let path = Path::new(file);
        let len = std::fs::metadata(path)
            .with_context(|| format!("failed to read manifest '{file}'"))?
            .len();
        if len > MAX_LINT_FILE_BYTES {
            return Err(anyhow!(
                "manifest '{file}' is {len} bytes; the lint limit is {MAX_LINT_FILE_BYTES}"
            ));
        }
        let value = load_vm_manifest(path)?;
        reports.push((file.clone(), lint_vm(&value)));
    }

    match output.as_str() {
        "json" => {
            let v: Vec<Value> = reports
                .iter()
                .map(|(f, r)| serde_json::json!({ "file": f, "report": r }))
                .collect();
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        "yaml" | "yml" => {
            let v: Vec<Value> = reports
                .iter()
                .map(|(f, r)| serde_json::json!({ "file": f, "report": r }))
                .collect();
            println!("{}", serde_yaml::to_string(&v)?);
        }
        _ => {
            for (file, r) in &reports {
                render_lint_table(file, r);
            }
        }
    }

    if let Some(t) = threshold {
        if let Some((file, r)) = reports.iter().find(|(_, r)| r.meets_threshold(t)) {
            return Err(anyhow!(
                "lint gate failed: '{}' has a finding of severity '{}' (>= --fail-on {})",
                file,
                r.highest_severity
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                t
            ));
        }
    }
    Ok(())
}

fn render_lint_table(file: &str, r: &LintReport) {
    println!("{}", color::header("Zorvia VM lint"));
    println!("  File:       {file}");
    println!(
        "  VM:         {}/{}",
        r.namespace.as_deref().unwrap_or("-"),
        r.name.as_deref().unwrap_or("-")
    );
    if r.live_migration_possible {
        println!("  Migration:  {}", color::success("no blockers found"));
    } else {
        println!(
            "  Migration:  {} ({})",
            color::warning("BLOCKED"),
            r.live_migration_blockers.join(", ")
        );
    }
    println!();
    if r.findings.is_empty() {
        println!("{}", color::success("✓ No findings"));
        println!();
        return;
    }
    println!("  {:<7} {:<9} {:<52} MESSAGE", "RULE", "SEVERITY", "PATH");
    println!("  {}", "-".repeat(110));
    for f in &r.findings {
        let sev = f.severity.to_string();
        let shown = match f.severity {
            LintSeverity::High => color::error(&sev),
            LintSeverity::Medium => color::warning(&sev),
            LintSeverity::Low => color::info(&sev),
            LintSeverity::Info => color::muted(&sev),
        };
        println!(
            "  {:<7} {:<9} {:<52} {}",
            f.rule, shown, f.pointer, f.message
        );
        println!("  {:<7} {:<9} {:<52} fix: {}", "", "", "", f.remedy);
    }
    if r.truncated {
        println!(
            "  (list truncated at {} findings)",
            crate::gitops::lint::MAX_FINDINGS
        );
    }
    println!();
}

#[cfg(test)]
mod lint_tests {
    use super::*;

    fn tmp(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zorvia-lint-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("vm.yaml");
        std::fs::write(&p, body).unwrap();
        p
    }

    const BAD: &str = "kind: VirtualMachine\nmetadata: {name: a, namespace: n}\nspec:\n  template:\n    spec:\n      domain:\n        devices:\n          interfaces: [{name: d, bridge: {}}]\n      networks: [{name: d, pod: {}}]\n";

    #[test]
    fn gate_fails_on_high_and_passes_with_none() {
        let p = tmp("gate", BAD);
        let f = p.to_string_lossy().to_string();
        let e = handle_lint(vec![f.clone()], "high".into(), "json".into()).unwrap_err();
        assert!(e.to_string().contains("lint gate failed"), "{e}");
        handle_lint(vec![f], "none".into(), "json".into()).unwrap();
    }

    #[test]
    fn bad_flags_and_missing_files_error() {
        let p = tmp("flags", BAD).to_string_lossy().to_string();
        assert!(handle_lint(vec![p.clone()], "bogus".into(), "json".into()).is_err());
        assert!(handle_lint(vec![p], "none".into(), "xml".into()).is_err());
        assert!(handle_lint(
            vec!["/nonexistent/vm.yaml".into()],
            "none".into(),
            "json".into()
        )
        .is_err());
    }

    #[test]
    fn garbage_and_oversize_files_error_without_panic() {
        let p = tmp("garbage", "\u{0}\u{1}: [unterminated")
            .to_string_lossy()
            .to_string();
        assert!(handle_lint(vec![p], "none".into(), "json".into()).is_err());
        let big = tmp("big", &"a".repeat(MAX_LINT_FILE_BYTES as usize + 1))
            .to_string_lossy()
            .to_string();
        let e = handle_lint(vec![big], "none".into(), "json".into()).unwrap_err();
        assert!(e.to_string().contains("lint limit"));
    }
}
