// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Parse KubeVirt guest-agent subresource payloads into Fabric metrics.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuestMetrics {
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub memory_available: u64,
    pub disk_usage: u64,
    pub disk_total: u64,
    pub network_rx: u64,
    pub network_tx: u64,
    pub hostname: Option<String>,
    pub agent: bool,
    pub source: String,
}

impl Default for GuestMetrics {
    fn default() -> Self {
        Self {
            cpu_usage: 0.0,
            memory_usage: 0,
            memory_available: 0,
            disk_usage: 0,
            disk_total: 0,
            network_rx: 0,
            network_tx: 0,
            hostname: None,
            agent: false,
            source: "none".into(),
        }
    }
}

/// Merge guestosinfo + filesystemlist JSON into a metrics snapshot.
pub fn metrics_from_guest_payloads(
    osinfo: Option<&Value>,
    filesystems: Option<&Value>,
) -> GuestMetrics {
    let mut out = GuestMetrics::default();
    if let Some(os) = osinfo {
        out.agent = true;
        out.source = "guest-agent".into();
        out.hostname = os
            .get("hostname")
            .and_then(|v| v.as_str())
            .or_else(|| {
                os.get("guestOSInfo")
                    .and_then(|g| g.get("hostname"))
                    .and_then(|v| v.as_str())
            })
            .map(|s| s.to_string());
        if let Some(mem) = os.get("guestMemoryUsageBytes").and_then(|v| v.as_u64()) {
            out.memory_usage = mem;
        } else if let Some(mem) = os.get("memoryUsageBytes").and_then(|v| v.as_u64()) {
            out.memory_usage = mem;
        }
        if let Some(avail) = os
            .get("guestMemoryAvailableBytes")
            .and_then(|v| v.as_u64())
            .or_else(|| os.get("memoryAvailableBytes").and_then(|v| v.as_u64()))
        {
            out.memory_available = avail;
        }
        if let Some(cpu) = os.get("cpuUsagePercent").and_then(|v| v.as_f64()) {
            out.cpu_usage = cpu.clamp(0.0, 100.0);
        }
    }
    if let Some(fs) = filesystems {
        let items = fs
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut used = 0u64;
        let mut total = 0u64;
        for item in items {
            used = used.saturating_add(item.get("usedBytes").and_then(|v| v.as_u64()).unwrap_or(0));
            total =
                total.saturating_add(item.get("totalBytes").and_then(|v| v.as_u64()).unwrap_or(0));
        }
        if used > 0 || total > 0 {
            out.disk_usage = used;
            out.disk_total = total;
            if !out.agent {
                out.agent = true;
                out.source = "guest-agent".into();
            }
        }
    }
    if out.memory_available > 0 && out.cpu_usage == 0.0 {
        // Rough pressure signal when GA exposes memory but not CPU.
        let used = out.memory_usage.min(out.memory_available);
        out.cpu_usage = ((used as f64 / out.memory_available as f64) * 15.0).clamp(0.0, 100.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_guestosinfo_and_filesystems() {
        let os = json!({
            "hostname": "web-01",
            "cpuUsagePercent": 22.5,
            "guestMemoryUsageBytes": 512_000_000u64,
            "guestMemoryAvailableBytes": 2_048_000_000u64
        });
        let fs = json!({
            "items": [
                {"mountPoint": "/", "usedBytes": 8_000_000_000u64, "totalBytes": 40_000_000_000u64}
            ]
        });
        let m = metrics_from_guest_payloads(Some(&os), Some(&fs));
        assert!(m.agent);
        assert_eq!(m.hostname.as_deref(), Some("web-01"));
        assert_eq!(m.cpu_usage, 22.5);
        assert_eq!(m.disk_usage, 8_000_000_000);
        assert_eq!(m.source, "guest-agent");
    }

    #[test]
    fn empty_payloads_are_zeros() {
        let m = metrics_from_guest_payloads(None, None);
        assert!(!m.agent);
        assert_eq!(m.cpu_usage, 0.0);
    }
}
