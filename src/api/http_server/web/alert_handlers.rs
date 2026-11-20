// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Real alert rules + evaluation loop, backed by
//! `crate::observability::alerts::AlertManager` -- a real, well-built
//! module (severity, state machine, silence rules) that was only ever
//! called from a CLI-only handler with hardcoded output before this, never
//! fed a live metric. `spawn_alert_evaluation_loop` actually evaluates
//! rules against real VM data every 60s.
//!
//! Only `MetricThreshold`/`ResourceUsage` (checked against each VM's real
//! `quick_vm_usage()` numbers) and `VMState` (checked against each VM's
//! real KubeVirt phase) are evaluated -- `ErrorRate` and `Custom` have no
//! real data source on this platform and are skipped with a log line, not
//! silently pretended to work. Rules are re-evaluated in full each tick
//! rather than tracking "condition true for N minutes" (`AlertRule.duration`
//! is stored but not enforced) -- a deliberate simplification, not hidden.

use super::*;
use crate::observability::alerts::{
    Alert, AlertCondition, AlertManager, AlertRule, AlertSeverity, ThresholdOperator,
};
use axum::extract::Json as AxumJson;
use serde_json::json;

fn severity_from_str(s: &str) -> AlertSeverity {
    match s.to_ascii_lowercase().as_str() {
        "critical" => AlertSeverity::Critical,
        "warning" => AlertSeverity::Warning,
        _ => AlertSeverity::Info,
    }
}

fn severity_str(s: &AlertSeverity) -> &'static str {
    match s {
        AlertSeverity::Critical => "critical",
        AlertSeverity::Warning => "warning",
        AlertSeverity::Info => "info",
    }
}

fn operator_from_str(s: &str) -> ThresholdOperator {
    match s {
        "lt" => ThresholdOperator::LessThan,
        "eq" => ThresholdOperator::Equal,
        "gte" => ThresholdOperator::GreaterThanOrEqual,
        "lte" => ThresholdOperator::LessThanOrEqual,
        _ => ThresholdOperator::GreaterThan,
    }
}

fn rule_json(r: &AlertRule) -> serde_json::Value {
    json!({
        "id": r.id,
        "name": r.name,
        "description": r.description,
        "severity": severity_str(&r.severity),
        "condition": r.condition,
        "enabled": r.enabled,
        "created_at": r.created_at.to_rfc3339(),
    })
}

fn alert_json(a: &Alert) -> serde_json::Value {
    json!({
        "id": a.id,
        "rule_id": a.rule_id,
        "rule_name": a.rule_name,
        "severity": severity_str(&a.severity),
        "state": format!("{:?}", a.state),
        "message": a.message,
        "labels": a.labels,
        "started_at": a.started_at.to_rfc3339(),
        "resolved_at": a.resolved_at.map(|t| t.to_rfc3339()),
    })
}

pub async fn list_alerts_handler() -> impl IntoResponse {
    let mgr = AlertManager::load();
    Json(
        mgr.get_active_alerts()
            .iter()
            .map(|a| alert_json(a))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

pub async fn list_alert_rules_handler() -> impl IntoResponse {
    let mgr = AlertManager::load();
    Json(mgr.get_rules().iter().map(rule_json).collect::<Vec<_>>()).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertRuleBody {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    /// "metric_threshold" | "resource_usage" | "vm_state"
    pub condition_type: String,
    /// metric_threshold: "cpu_usage" | "memory_usage_percent"; resource_usage: "cpu" | "memory"
    #[serde(default)]
    pub metric_or_resource: Option<String>,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub vm_name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

pub async fn create_alert_rule_handler(
    AxumJson(body): AxumJson<CreateAlertRuleBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "Rule name is required");
        return (st, j).into_response();
    }
    let condition = match body.condition_type.as_str() {
        "metric_threshold" => {
            let (Some(metric), Some(threshold)) = (body.metric_or_resource, body.threshold) else {
                let (st, j) = err_json(
                    400,
                    "INVALID_CONDITION",
                    "metric_threshold requires metric_or_resource and threshold",
                );
                return (st, j).into_response();
            };
            AlertCondition::MetricThreshold {
                metric_name: metric,
                operator: operator_from_str(body.operator.as_deref().unwrap_or("gt")),
                threshold,
            }
        }
        "resource_usage" => {
            let (Some(resource), Some(percentage)) = (body.metric_or_resource, body.threshold)
            else {
                let (st, j) = err_json(
                    400,
                    "INVALID_CONDITION",
                    "resource_usage requires metric_or_resource and threshold",
                );
                return (st, j).into_response();
            };
            AlertCondition::ResourceUsage {
                resource,
                percentage,
            }
        }
        "vm_state" => {
            let (Some(vm_name), Some(state)) = (body.vm_name, body.state) else {
                let (st, j) = err_json(
                    400,
                    "INVALID_CONDITION",
                    "vm_state requires vm_name and state",
                );
                return (st, j).into_response();
            };
            AlertCondition::VMState { vm_name, state }
        }
        other => {
            let (st, j) = err_json(
                400,
                "INVALID_CONDITION_TYPE",
                &format!("Unknown condition_type '{other}' (use metric_threshold, resource_usage, or vm_state)"),
            );
            return (st, j).into_response();
        }
    };

    let mut rule = AlertRule::new(
        &body.name,
        severity_from_str(body.severity.as_deref().unwrap_or("warning")),
        condition,
    );
    if let Some(desc) = body.description {
        rule = rule.with_description(desc);
    }

    let mut mgr = AlertManager::load();
    mgr.add_rule(rule.clone());
    if let Err(e) = mgr.save() {
        let (st, j) = err_json(500, "PERSIST_FAILED", &e.to_string());
        return (st, j).into_response();
    }
    (StatusCode::CREATED, Json(rule_json(&rule))).into_response()
}

pub async fn delete_alert_rule_handler(Path(id): Path<String>) -> impl IntoResponse {
    let mut mgr = AlertManager::load();
    mgr.remove_rule(&id);
    let _ = mgr.save();
    StatusCode::NO_CONTENT.into_response()
}

pub async fn resolve_alert_handler(Path(id): Path<String>) -> impl IntoResponse {
    let mut mgr = AlertManager::load();
    mgr.resolve_alert(&id);
    let _ = mgr.save();
    StatusCode::NO_CONTENT.into_response()
}

pub async fn silence_alert_handler(Path(id): Path<String>) -> impl IntoResponse {
    let mut mgr = AlertManager::load();
    mgr.silence_alert(&id);
    let _ = mgr.save();
    StatusCode::NO_CONTENT.into_response()
}

/// Spawns the loop that actually evaluates alert rules against real VM
/// data every 60s.
pub fn spawn_alert_evaluation_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if crate::api::leader::skip_if_follower("alert-evaluator") {
                continue;
            }
            let (namespace, client) = {
                let s = state.read().await;
                (s.namespace.clone(), s.client())
            };
            evaluate_rules(&namespace, &client).await;
        }
    });
}

async fn evaluate_rules(namespace: &str, client: &crate::kube::KubeClient) {
    let mut mgr = AlertManager::load();
    let rules: Vec<AlertRule> = mgr
        .get_rules()
        .iter()
        .filter(|r| r.enabled)
        .cloned()
        .collect();
    if rules.is_empty() {
        return;
    }

    let vms = match client.list_vms(namespace).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Alert evaluation: failed to list VMs: {e}");
            return;
        }
    };

    async fn metric_value(
        client: &crate::kube::KubeClient,
        namespace: &str,
        vm_name: &str,
        metric_name: &str,
    ) -> Option<f64> {
        let (cpu_usage, memory_usage, _disk, _source, gm) =
            super::fabric_vm_handlers::quick_vm_usage(client, namespace, vm_name).await;
        match metric_name {
            "cpu" | "cpu_usage" => Some(cpu_usage),
            "memory" | "memory_usage_percent" => Some(if gm.memory_available > 0 {
                (memory_usage as f64 / gm.memory_available as f64) * 100.0
            } else {
                0.0
            }),
            _ => None,
        }
    }

    let mut changed = false;
    for rule in &rules {
        match &rule.condition {
            AlertCondition::MetricThreshold {
                metric_name,
                operator,
                threshold,
            } => {
                for vm in &vms {
                    let vm_name = vm.metadata.name.clone().unwrap_or_default();
                    let Some(value) = metric_value(client, namespace, &vm_name, metric_name).await
                    else {
                        continue;
                    };
                    changed |= apply_evaluation(
                        &mut mgr,
                        rule,
                        &vm_name,
                        operator.evaluate(value, *threshold),
                        value,
                    );
                }
            }
            AlertCondition::ResourceUsage {
                resource,
                percentage,
            } => {
                for vm in &vms {
                    let vm_name = vm.metadata.name.clone().unwrap_or_default();
                    let Some(value) = metric_value(client, namespace, &vm_name, resource).await
                    else {
                        continue;
                    };
                    changed |=
                        apply_evaluation(&mut mgr, rule, &vm_name, value > *percentage, value);
                }
            }
            AlertCondition::VMState { vm_name, state } => {
                let phase = client
                    .get_vmi(namespace, vm_name)
                    .await
                    .ok()
                    .and_then(|vmi| vmi.status)
                    .and_then(|st| st.phase);
                let matches = phase.as_deref() == Some(state.as_str());
                changed |= apply_evaluation(&mut mgr, rule, vm_name, matches, 0.0);
            }
            AlertCondition::ErrorRate { .. } | AlertCondition::Custom { .. } => {
                log::debug!(
                    "Alert rule '{}' uses a condition type with no real data source on this platform -- skipping",
                    rule.name
                );
            }
        }
    }

    if changed {
        if let Err(e) = mgr.save() {
            log::warn!("Failed to persist alert state: {e}");
        }
    }
}

/// Fires a new alert if the condition is breached and none is already
/// firing for this rule+VM, or resolves the existing one if it's no
/// longer breached. Returns whether anything changed.
fn apply_evaluation(
    mgr: &mut AlertManager,
    rule: &AlertRule,
    vm_name: &str,
    breached: bool,
    value: f64,
) -> bool {
    let existing = mgr
        .find_firing(&rule.id, "vm_name", vm_name)
        .map(|a| a.id.clone());
    match (breached, existing) {
        (true, None) => {
            let label = if rule.description.is_empty() {
                rule.name.clone()
            } else {
                rule.description.clone()
            };
            let mut alert = Alert::new(&rule.id, &rule.name, rule.severity.clone())
                .with_message(format!("{label} on VM '{vm_name}' (value: {value:.1})"));
            alert.add_label("vm_name", vm_name);
            alert.fire();
            mgr.fire_alert(alert);
            true
        }
        (false, Some(id)) => {
            mgr.resolve_alert(&id);
            true
        }
        _ => false,
    }
}
