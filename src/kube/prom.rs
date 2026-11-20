// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Prometheus text-format parser for virt-launcher / kubevirt_vmi_* samples.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromSample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

/// Parse a subset of Prometheus text exposition (gauge/counter lines).
pub fn parse_prom_text(text: &str) -> Vec<PromSample> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name_labels, value_part) = match line.rsplit_once(char::is_whitespace) {
            Some(pair) => pair,
            None => continue,
        };
        let Ok(value) = value_part.trim().parse::<f64>() else {
            continue;
        };
        let (name, labels) = split_name_labels(name_labels.trim());
        if name.starts_with("kubevirt_vmi_") || name.starts_with("zorvia_") {
            out.push(PromSample {
                name,
                labels,
                value,
            });
        }
    }
    out
}

fn split_name_labels(spec: &str) -> (String, BTreeMap<String, String>) {
    if let Some(idx) = spec.find('{') {
        let name = spec[..idx].to_string();
        let inner = spec[idx + 1..].trim_end_matches('}');
        let mut labels = BTreeMap::new();
        for part in inner.split(',') {
            if let Some((k, v)) = part.split_once('=') {
                labels.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
        (name, labels)
    } else {
        (spec.to_string(), BTreeMap::new())
    }
}

/// Reduce virt-launcher samples into the Fabric metrics shape.
pub fn fabric_from_prom(samples: &[PromSample], vm: &str) -> (f64, u64, u64) {
    let for_vm = |s: &PromSample| {
        s.labels
            .get("name")
            .or_else(|| s.labels.get("vm"))
            .map(|n| n == vm)
            .unwrap_or(true)
    };
    let cpu = samples
        .iter()
        .filter(|s| s.name == "kubevirt_vmi_vcpu_seconds" && for_vm(s))
        .map(|s| s.value)
        .sum::<f64>();
    let mem = samples
        .iter()
        .find(|s| {
            (s.name == "kubevirt_vmi_memory_available_bytes"
                || s.name == "kubevirt_vmi_memory_usable_bytes")
                && for_vm(s)
        })
        .map(|s| s.value as u64)
        .unwrap_or(0);
    let disk = samples
        .iter()
        .find(|s| s.name.contains("storage") && for_vm(s))
        .map(|s| s.value as u64)
        .unwrap_or(0);
    (cpu, mem, disk)
}

/// Prometheus HTTP API instant-query URL.
pub fn prom_query_url(base: &str, query: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/api/v1/query?query={}", urlencoding_query(query))
}

fn urlencoding_query(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

/// Parse Prometheus HTTP API JSON (`data.result[].value[1]`).
pub fn parse_prom_query_api(body: &serde_json::Value, vm: &str) -> Vec<PromSample> {
    let mut out = Vec::new();
    let Some(results) = body.pointer("/data/result").and_then(|v| v.as_array()) else {
        return out;
    };
    for row in results {
        let name = row
            .pointer("/metric/__name__")
            .and_then(|v| v.as_str())
            .unwrap_or("kubevirt_vmi_vcpu_seconds")
            .to_string();
        let mut labels = BTreeMap::new();
        if let Some(obj) = row.get("metric").and_then(|m| m.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    labels.insert(k.clone(), s.to_string());
                }
            }
        }
        let vm_ok = labels
            .get("name")
            .or_else(|| labels.get("vm"))
            .map(|n| n == vm)
            .unwrap_or(true);
        if !vm_ok {
            continue;
        }
        let value = row
            .get("value")
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(1))
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v.as_f64())
            })
            .unwrap_or(0.0);
        out.push(PromSample {
            name,
            labels,
            value,
        });
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsPoint {
    pub ts: String,
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub disk_usage: u64,
}

pub struct MetricsRing {
    inner: std::sync::Mutex<
        std::collections::HashMap<String, std::collections::VecDeque<MetricsPoint>>,
    >,
}

impl MetricsRing {
    pub fn global() -> &'static Self {
        static RING: std::sync::OnceLock<MetricsRing> = std::sync::OnceLock::new();
        RING.get_or_init(|| MetricsRing {
            inner: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn push(&self, vm: &str, point: MetricsPoint) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let q = map.entry(vm.to_string()).or_default();
        q.push_back(point);
        while q.len() > 30 {
            q.pop_front();
        }
    }

    pub fn history(&self, vm: &str) -> Vec<MetricsPoint> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(vm)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Optional lab override: `ZORVIA_PROM_SAMPLES` path to Prometheus text.
pub fn samples_from_env_file(vm: &str) -> Option<(f64, u64, u64)> {
    let path = std::env::var("ZORVIA_PROM_SAMPLES").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let samples = parse_prom_text(&text);
    if samples.is_empty() {
        return None;
    }
    Some(fabric_from_prom(&samples, vm))
}

pub fn servicemonitor_yaml(namespace: &str) -> String {
    format!(
        r#"apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: kubevirt-vmis
  namespace: {namespace}
  labels:
    app.kubernetes.io/managed-by: zorvia
spec:
  selector:
    matchLabels:
      kubevirt.io: virt-launcher
  namespaceSelector:
    any: true
  endpoints:
    - port: metrics
      interval: 30s
      honorLabels: true
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kubevirt_samples() {
        let text = r#"
# HELP kubevirt_vmi_vcpu_seconds VCPU time
kubevirt_vmi_vcpu_seconds{name="web-01",namespace="default",id="0"} 12.5
kubevirt_vmi_memory_available_bytes{name="web-01",namespace="default"} 2147483648
ignored_metric 1
"#;
        let samples = parse_prom_text(text);
        assert_eq!(samples.len(), 2);
        let (cpu, mem, _) = fabric_from_prom(&samples, "web-01");
        assert_eq!(cpu, 12.5);
        assert_eq!(mem, 2147483648);
    }

    #[test]
    fn servicemonitor_targets_virt_launcher() {
        let y = servicemonitor_yaml("monitoring");
        assert!(y.contains("virt-launcher"));
        assert!(y.contains("namespace: monitoring"));
    }

    #[test]
    fn parses_query_api_and_builds_url() {
        let url = prom_query_url(
            "http://prom:9090",
            r#"kubevirt_vmi_vcpu_seconds{name="web-01"}"#,
        );
        assert!(url.starts_with("http://prom:9090/api/v1/query?query="));
        let body = serde_json::json!({
            "data": {"result": [{
                "metric": {"__name__": "kubevirt_vmi_vcpu_seconds", "name": "web-01"},
                "value": [1, "8.25"]
            }]}
        });
        let samples = parse_prom_query_api(&body, "web-01");
        assert_eq!(samples[0].value, 8.25);
        let ring = MetricsRing {
            inner: std::sync::Mutex::new(std::collections::HashMap::new()),
        };
        ring.push(
            "web-01",
            MetricsPoint {
                ts: "t".into(),
                cpu_usage: 1.0,
                memory_usage: 2,
                disk_usage: 3,
            },
        );
        assert_eq!(ring.history("web-01").len(), 1);
    }
}
