// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Guest-ready signals after start / cloud-init.
//!
//! KubeVirt does not expose cloud-init's status file directly. Operators wait
//! for: VMI Running, Ready=True, a guest IP, and (when present) qemu-ga.

use crate::kube::types::VirtualMachineInstanceStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestReadyReport {
    pub running: bool,
    pub ready: bool,
    pub has_ip: bool,
    pub agent_connected: bool,
    pub cloud_init_ready: bool,
    pub reason: String,
}

impl GuestReadyReport {
    pub fn ready_for_login(&self) -> bool {
        self.running && self.ready && self.has_ip
    }
}

fn cond_true(status: &VirtualMachineInstanceStatus, ty: &str) -> bool {
    status
        .conditions
        .as_ref()
        .map(|cs| {
            cs.iter()
                .any(|c| c.type_.eq_ignore_ascii_case(ty) && c.status.eq_ignore_ascii_case("True"))
        })
        .unwrap_or(false)
}

pub fn guest_ready_from_status(status: Option<&VirtualMachineInstanceStatus>) -> GuestReadyReport {
    let Some(status) = status else {
        return GuestReadyReport {
            running: false,
            ready: false,
            has_ip: false,
            agent_connected: false,
            cloud_init_ready: false,
            reason: "VMI not running".into(),
        };
    };
    let running = status
        .phase
        .as_deref()
        .map(|p| p.eq_ignore_ascii_case("running"))
        .unwrap_or(false);
    let ready = cond_true(status, "Ready");
    // `guestOSInfo: {}` (present but empty) is what KubeVirt reports for a guest
    // with no agent, so only count it when it actually carries data.
    let agent = cond_true(status, "AgentConnected")
        || status.guest_os_info.as_ref().is_some_and(|g| {
            g.name.is_some() || g.id.is_some() || g.version.is_some() || g.kernel_release.is_some()
        });
    let has_ip = status.interfaces.iter().any(|i| {
        i.ip_address
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            || i.ip_addresses.iter().any(|s| !s.is_empty())
    });
    // Cloud-init is treated as done when the guest is Ready and has an address.
    // qemu-ga is extra confirmation, not a hard requirement (Windows / minimal images).
    let cloud_init_ready = running && ready && has_ip;
    let reason = if cloud_init_ready {
        "guest has IP and Ready=True".into()
    } else if !running {
        "VMI phase is not Running".into()
    } else if !ready {
        "VMI Ready condition is not True".into()
    } else {
        "no guest IP yet (cloud-init / DHCP still running)".into()
    };
    GuestReadyReport {
        running,
        ready,
        has_ip,
        agent_connected: agent,
        cloud_init_ready,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kube::types::{Condition, GuestOsInfo, VmiInterface};

    fn status(
        phase: &str,
        ready: bool,
        ip: Option<&str>,
        ga: bool,
    ) -> VirtualMachineInstanceStatus {
        VirtualMachineInstanceStatus {
            phase: Some(phase.into()),
            node_name: Some("node-a".into()),
            interfaces: ip
                .map(|ip| {
                    vec![VmiInterface {
                        name: Some("default".into()),
                        ip_address: Some(ip.into()),
                        ip_addresses: vec![ip.into()],
                        mac: None,
                        interface_name: None,
                    }]
                })
                .unwrap_or_default(),
            conditions: Some(vec![Condition {
                type_: "Ready".into(),
                status: if ready { "True".into() } else { "False".into() },
                reason: None,
                message: None,
                last_probe_time: None,
                last_transition_time: None,
            }]),
            guest_os_info: ga.then_some(GuestOsInfo {
                name: Some("Ubuntu".into()),
                id: Some("ubuntu".into()),
                version: Some("24.04".into()),
                kernel_release: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn not_ready_without_vmi() {
        let r = guest_ready_from_status(None);
        assert!(!r.cloud_init_ready);
        assert!(!r.ready_for_login());
    }

    #[test]
    fn ready_when_running_ready_and_ip() {
        let s = status("Running", true, Some("10.0.0.8"), true);
        let r = guest_ready_from_status(Some(&s));
        assert!(r.cloud_init_ready);
        assert!(r.agent_connected);
        assert!(r.ready_for_login());
    }

    #[test]
    fn waits_for_dhcp() {
        let s = status("Running", true, None, false);
        let r = guest_ready_from_status(Some(&s));
        assert!(!r.cloud_init_ready);
        assert!(r.reason.contains("IP"));
    }

    #[test]
    fn an_empty_guest_os_info_is_not_an_agent() {
        // KubeVirt reports `guestOSInfo: {}` for a guest with no agent; that
        // used to count as "agent connected".
        let mut st = status("Running", true, Some("10.0.0.1"), false);
        st.guest_os_info = Some(GuestOsInfo {
            name: None,
            id: None,
            version: None,
            kernel_release: None,
        });
        assert!(!guest_ready_from_status(Some(&st)).agent_connected);
        st.guest_os_info = Some(GuestOsInfo {
            name: Some("Debian GNU/Linux".into()),
            id: None,
            version: None,
            kernel_release: None,
        });
        assert!(guest_ready_from_status(Some(&st)).agent_connected);
    }
}
