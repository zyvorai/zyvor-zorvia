// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Real compliance scanning, backed by `crate::security::compliance::ComplianceChecker`
//! -- pure functions that inspect an actual fetched `VirtualMachine` spec
//! for PCI-DSS/HIPAA/SOC2 controls (resource limits, eviction strategy,
//! TPM, dedicated CPU, etc). These had zero callers anywhere in the
//! codebase; every check here runs against real, live VM specs, computed
//! fresh on each request (there's no stored scan history to go stale).

use super::*;
use crate::security::compliance::{CheckStatus, ComplianceChecker};
use serde_json::json;

fn status_str(s: &CheckStatus) -> &'static str {
    match s {
        CheckStatus::Passed => "pass",
        CheckStatus::Failed => "fail",
        CheckStatus::ManualReview | CheckStatus::NotApplicable => "warning",
    }
}

pub async fn compliance_dashboard_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let kube_client = s.kube_client.clone();
    drop(s);

    let vms = match kube_client.list_vms(&namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "COMPLIANCE_SCAN_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let mut checks = Vec::new();
    let mut categories = std::collections::BTreeSet::new();
    let (mut passed, mut warnings, mut failed) = (0u32, 0u32, 0u32);

    for vm in &vms {
        let vm_name = vm.metadata.name.clone().unwrap_or_default();
        for report in ComplianceChecker::check_all(&vm_name, vm) {
            let category = report.framework.to_string();
            categories.insert(category.clone());
            for check in &report.check_results {
                match check.status {
                    CheckStatus::Passed => passed += 1,
                    CheckStatus::Failed => failed += 1,
                    CheckStatus::ManualReview | CheckStatus::NotApplicable => warnings += 1,
                }
                checks.push(json!({
                    "id": format!("{}/{}", vm_name, check.check_id),
                    "category": category,
                    "name": format!("{} ({vm_name})", check.title),
                    "status": status_str(&check.status),
                    "description": check.message,
                    "remediation": check.evidence,
                }));
            }
        }
    }

    let total = passed + warnings + failed;
    let score = if total > 0 {
        (passed as f64 / total as f64 * 100.0).round()
    } else {
        100.0
    };

    Json(json!({
        "score": score,
        "total": total,
        "passed": passed,
        "warnings": warnings,
        "failed": failed,
        "categories": categories.into_iter().collect::<Vec<_>>(),
        "checks": checks,
        "last_scan": chrono::Utc::now().to_rfc3339(),
    }))
    .into_response()
}
