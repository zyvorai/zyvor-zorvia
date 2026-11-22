// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Guest visibility derived from KubeVirt VMI status.
//!
//! KubeVirt surfaces QEMU Guest Agent information through VMI status when the
//! agent is connected. Zorvia turns that low-level status into an operator-
//! friendly readiness report without shelling into the guest.

use crate::kube::{VirtualMachineInstanceStatus, VmiInterface};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuestAgentState {
    Connected,
    NotDetected,
    NotRunning,
    Unknown,
}

impl std::fmt::Display for GuestAgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Connected => "connected",
            Self::NotDetected => "not-detected",
            Self::NotRunning => "not-running",
            Self::Unknown => "unknown",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestInterfaceInsight {
    pub name: Option<String>,
    pub guest_name: Option<String>,
    pub mac: Option<String>,
    pub primary_ip: Option<String>,
    pub ip_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestInsightReport {
    pub vm: String,
    pub namespace: String,
    pub phase: String,
    pub node: Option<String>,
    pub agent_state: GuestAgentState,
    pub readiness_score: u8,
    pub os_name: Option<String>,
    pub os_id: Option<String>,
    pub os_version: Option<String>,
    pub kernel_release: Option<String>,
    pub interfaces: Vec<GuestInterfaceInsight>,
    pub recommendations: Vec<String>,
}

impl GuestInsightReport {
    pub fn from_status(
        vm: impl Into<String>,
        namespace: impl Into<String>,
        status: Option<&VirtualMachineInstanceStatus>,
    ) -> Self {
        let vm = vm.into();
        let namespace = namespace.into();

        let Some(status) = status else {
            return Self {
                vm,
                namespace,
                phase: "NotRunning".to_string(),
                node: None,
                agent_state: GuestAgentState::NotRunning,
                readiness_score: 0,
                os_name: None,
                os_id: None,
                os_version: None,
                kernel_release: None,
                interfaces: Vec::new(),
                recommendations: vec![
                    "Start the VM before evaluating QEMU Guest Agent connectivity".to_string(),
                ],
            };
        };

        let phase = status
            .phase
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let is_running = phase.eq_ignore_ascii_case("running");
        let guest = status.guest_os_info.as_ref();

        let agent_state = if !is_running {
            GuestAgentState::NotRunning
        } else if guest.is_some() {
            GuestAgentState::Connected
        } else {
            GuestAgentState::NotDetected
        };

        let interfaces = status
            .interfaces
            .iter()
            .map(interface_insight)
            .collect::<Vec<_>>();

        let mut score = 0u8;
        if is_running {
            score = score.saturating_add(20);
        }
        if status.node_name.is_some() {
            score = score.saturating_add(10);
        }
        if interfaces
            .iter()
            .any(|iface| iface.primary_ip.is_some() || !iface.ip_addresses.is_empty())
        {
            score = score.saturating_add(20);
        }
        if guest.is_some() {
            score = score.saturating_add(30);
        }
        if guest
            .and_then(|info| info.kernel_release.as_ref())
            .is_some()
        {
            score = score.saturating_add(20);
        }

        let mut recommendations = Vec::new();
        match agent_state {
            GuestAgentState::Connected => {
                if guest
                    .and_then(|info| info.kernel_release.as_ref())
                    .is_none()
                {
                    recommendations.push(
                        "Guest agent is connected but kernel metadata is incomplete; verify qemu-guest-agent is current"
                            .to_string(),
                    );
                }
                if interfaces.is_empty() {
                    recommendations.push(
                        "Guest agent is connected but no interfaces are reported; verify guest networking"
                            .to_string(),
                    );
                }
            }
            GuestAgentState::NotDetected => {
                recommendations
                    .push("Install and enable qemu-guest-agent inside the guest".to_string());
                recommendations.push(
                    "Verify the KubeVirt guest-agent channel is available and the service is running"
                        .to_string(),
                );
            }
            GuestAgentState::NotRunning => recommendations.push(
                "Bring the VMI to Running state before evaluating guest-agent health".to_string(),
            ),
            GuestAgentState::Unknown => recommendations.push(
                "Guest-agent state is unknown; inspect VMI status and KubeVirt conditions"
                    .to_string(),
            ),
        }

        Self {
            vm,
            namespace,
            phase,
            node: status.node_name.clone(),
            agent_state,
            readiness_score: score.min(100),
            os_name: guest.and_then(|info| info.name.clone()),
            os_id: guest.and_then(|info| info.id.clone()),
            os_version: guest.and_then(|info| info.version.clone()),
            kernel_release: guest.and_then(|info| info.kernel_release.clone()),
            interfaces,
            recommendations,
        }
    }

    pub fn agent_connected(&self) -> bool {
        self.agent_state == GuestAgentState::Connected
    }

    pub fn healthy(&self) -> bool {
        self.agent_connected() && self.readiness_score >= 70
    }
}

fn interface_insight(iface: &VmiInterface) -> GuestInterfaceInsight {
    GuestInterfaceInsight {
        name: iface.name.clone(),
        guest_name: iface.interface_name.clone(),
        mac: iface.mac.clone(),
        primary_ip: iface.ip_address.clone().filter(|value| !value.is_empty()),
        ip_addresses: iface
            .ip_addresses
            .iter()
            .filter(|value| !value.is_empty())
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kube::GuestOsInfo;

    fn running_status(with_guest: bool) -> VirtualMachineInstanceStatus {
        VirtualMachineInstanceStatus {
            conditions: None,
            phase: Some("Running".to_string()),
            node_name: Some("worker-1".to_string()),
            interfaces: vec![VmiInterface {
                name: Some("default".to_string()),
                ip_address: Some("10.0.0.10".to_string()),
                ip_addresses: vec!["10.0.0.10".to_string()],
                mac: Some("52:54:00:aa:bb:cc".to_string()),
                interface_name: Some("eth0".to_string()),
            }],
            guest_os_info: with_guest.then(|| GuestOsInfo {
                name: Some("Ubuntu".to_string()),
                id: Some("ubuntu".to_string()),
                version: Some("24.04".to_string()),
                kernel_release: Some("6.8.0".to_string()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn connected_agent_scores_full_readiness() {
        let status = running_status(true);
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert_eq!(report.agent_state, GuestAgentState::Connected);
        assert_eq!(report.readiness_score, 100);
        assert!(report.healthy());
    }

    #[test]
    fn missing_guest_info_is_not_detected() {
        let status = running_status(false);
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert_eq!(report.agent_state, GuestAgentState::NotDetected);
        assert!(!report.agent_connected());
        assert!(report
            .recommendations
            .iter()
            .any(|v| v.contains("qemu-guest-agent")));
    }

    #[test]
    fn absent_vmi_is_not_running() {
        let report = GuestInsightReport::from_status("vm", "default", None);
        assert_eq!(report.agent_state, GuestAgentState::NotRunning);
        assert_eq!(report.readiness_score, 0);
    }

    #[test]
    fn non_running_phase_is_not_running() {
        let mut status = running_status(true);
        status.phase = Some("Scheduling".to_string());
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert_eq!(report.agent_state, GuestAgentState::NotRunning);
    }

    #[test]
    fn interface_details_are_preserved() {
        let status = running_status(true);
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert_eq!(report.interfaces.len(), 1);
        assert_eq!(report.interfaces[0].guest_name.as_deref(), Some("eth0"));
        assert_eq!(
            report.interfaces[0].primary_ip.as_deref(),
            Some("10.0.0.10")
        );
    }

    #[test]
    fn empty_primary_ip_is_removed() {
        let mut status = running_status(true);
        status.interfaces[0].ip_address = Some(String::new());
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert!(report.interfaces[0].primary_ip.is_none());
    }

    #[test]
    fn partial_guest_info_recommends_agent_update() {
        let mut status = running_status(true);
        status.guest_os_info.as_mut().unwrap().kernel_release = None;
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        assert_eq!(report.readiness_score, 80);
        assert!(report
            .recommendations
            .iter()
            .any(|v| v.contains("kernel metadata")));
    }

    #[test]
    fn report_serializes_to_json() {
        let status = running_status(true);
        let report = GuestInsightReport::from_status("vm", "default", Some(&status));
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["agent_state"], "connected");
    }
}
