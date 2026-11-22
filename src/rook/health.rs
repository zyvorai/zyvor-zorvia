// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Parses `CephCluster.status` into a client-friendly health summary. Pure
//! functions over already-fetched JSON — no cluster access here.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum CephHealthState {
    /// CephCluster not found / not yet created.
    NotProvisioned,
    /// CephCluster exists but hasn't reached Ready/Connected yet.
    Provisioning,
    /// Ceph reports HEALTH_OK.
    Healthy,
    /// Ceph reports HEALTH_WARN.
    Warning,
    /// Ceph reports HEALTH_ERR, or the cluster phase is a failure state.
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CephHealthSummary {
    pub state: CephHealthState,
    pub phase: Option<String>,
    pub ceph_health: Option<String>,
    pub message: Option<String>,
}

/// Classify a CephCluster object's `status` block (`state`/`phase`,
/// `ceph.health`, `ceph.details`/`message`) into a summary.
pub fn summarize_ceph_cluster_status(obj: Option<&Value>) -> CephHealthSummary {
    let Some(obj) = obj else {
        return CephHealthSummary {
            state: CephHealthState::NotProvisioned,
            phase: None,
            ceph_health: None,
            message: None,
        };
    };
    let status = obj.get("status");
    let phase = status
        .and_then(|s| s.get("phase").or_else(|| s.get("state")))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    let ceph_health = status
        .and_then(|s| s.get("ceph"))
        .and_then(|c| c.get("health"))
        .and_then(|h| h.as_str())
        .map(|s| s.to_string());
    let message = status
        .and_then(|s| s.get("ceph"))
        .and_then(|c| c.get("details"))
        .and_then(|d| d.as_object())
        .and_then(|obj| obj.values().next())
        .and_then(|v| v.get("message"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());

    let state = match ceph_health.as_deref() {
        Some("HEALTH_OK") => CephHealthState::Healthy,
        Some("HEALTH_WARN") => CephHealthState::Warning,
        Some("HEALTH_ERR") => CephHealthState::Error,
        _ => match phase.as_deref() {
            Some("Ready") | Some("Connected") => CephHealthState::Healthy,
            Some("Failure") => CephHealthState::Error,
            Some(_) => CephHealthState::Provisioning,
            None => CephHealthState::Unknown,
        },
    };

    CephHealthSummary {
        state,
        phase,
        ceph_health,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_provisioned_when_absent() {
        let s = summarize_ceph_cluster_status(None);
        assert_eq!(s.state, CephHealthState::NotProvisioned);
    }

    #[test]
    fn healthy_from_ceph_health_ok() {
        let obj = json!({ "status": { "phase": "Ready", "ceph": { "health": "HEALTH_OK" } } });
        let s = summarize_ceph_cluster_status(Some(&obj));
        assert_eq!(s.state, CephHealthState::Healthy);
        assert_eq!(s.phase.as_deref(), Some("Ready"));
    }

    #[test]
    fn warning_from_ceph_health_warn() {
        let obj = json!({ "status": { "phase": "Ready", "ceph": { "health": "HEALTH_WARN" } } });
        let s = summarize_ceph_cluster_status(Some(&obj));
        assert_eq!(s.state, CephHealthState::Warning);
    }

    #[test]
    fn provisioning_when_no_ceph_health_yet() {
        let obj = json!({ "status": { "phase": "Progressing" } });
        let s = summarize_ceph_cluster_status(Some(&obj));
        assert_eq!(s.state, CephHealthState::Provisioning);
    }

    #[test]
    fn error_from_failure_phase() {
        let obj = json!({ "status": { "phase": "Failure" } });
        let s = summarize_ceph_cluster_status(Some(&obj));
        assert_eq!(s.state, CephHealthState::Error);
    }

    #[test]
    fn extracts_detail_message() {
        let obj = json!({
            "status": {
                "phase": "Ready",
                "ceph": {
                    "health": "HEALTH_WARN",
                    "details": { "MON_CLOCK_SKEW": { "message": "clock skew detected" } }
                }
            }
        });
        let s = summarize_ceph_cluster_status(Some(&obj));
        assert_eq!(s.message.as_deref(), Some("clock skew detected"));
    }
}
