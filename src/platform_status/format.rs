// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Platform status model and Cilium-style formatter.

use std::collections::BTreeMap;

use serde::Serialize;

use super::ansi::{self, ErrorCount, BLUE, CYAN, GREEN, MAGENTA, RED, RESET, YELLOW};

/// Workload readiness counts (Deployment / DaemonSet).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PodStateCount {
    pub type_name: String,
    pub desired: i32,
    pub ready: i32,
    pub available: i32,
    pub unavailable: i32,
}

impl PodStateCount {
    pub fn format_line(&self) -> String {
        let mut items = Vec::new();
        if self.desired > 0 {
            items.push(format!("Desired: {}", self.desired));
        }
        if self.ready > 0 || self.desired > 0 {
            let color = if self.ready >= self.desired {
                GREEN
            } else {
                YELLOW
            };
            items.push(format!(
                "Ready: {}{}/{}{}",
                color, self.ready, self.desired, RESET
            ));
        }
        if self.available > 0 || self.desired > 0 {
            let color = if self.available >= self.desired {
                GREEN
            } else {
                YELLOW
            };
            items.push(format!(
                "Available: {}{}/{}{}",
                color, self.available, self.desired, RESET
            ));
        }
        if self.unavailable > 0 {
            items.push(format!(
                "Unavailable: {}{}/{}{}",
                RED, self.unavailable, self.desired, RESET
            ));
        }
        items.join(", ")
    }
}

/// Named feature row (Zorvia extension below Cilium body).
#[derive(Debug, Clone, Serialize)]
pub struct FeatureStatus {
    pub name: String,
    pub state: ErrorCount,
}

/// Overall platform status collected from the cluster.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlatformStatus {
    /// Banner keys: kubevirt, cdi, zorvia-api, snapshots, rook
    pub components: BTreeMap<String, ErrorCount>,
    pub pod_state: BTreeMap<String, PodStateCount>,
    /// component -> phase -> count
    pub phase_count: BTreeMap<String, BTreeMap<String, i32>>,
    pub image_count: BTreeMap<String, BTreeMap<String, i32>>,
    pub vms_running: usize,
    pub vms_total: usize,
    pub features: Vec<FeatureStatus>,
    pub collection_errors: Vec<String>,
    /// deployment -> pod -> ErrorCount (for Errors:/Warnings: blocks)
    pub errors: BTreeMap<String, BTreeMap<String, ErrorCount>>,
}

impl PlatformStatus {
    pub fn component_summary(&self, key: &str) -> String {
        self.components
            .get(key)
            .map(|c| c.summary())
            .unwrap_or_else(|| format!("ℹ️  {}disabled{}", CYAN, RESET))
    }

    /// Format like Cilium `Status.Format()` — logo + tab-aligned body.
    pub fn format(&self) -> String {
        let mut buf = String::new();

        // Exact Cilium logo segment coloring
        buf.push_str(YELLOW);
        buf.push_str(" /¯¯\\\n");
        buf.push_str(CYAN);
        buf.push_str(" /¯¯");
        buf.push_str(YELLOW);
        buf.push_str("\\__/");
        buf.push_str(GREEN);
        buf.push_str("¯¯\\");
        buf.push_str(RESET);
        buf.push_str("\tKubeVirt:\t");
        buf.push_str(&self.component_summary("kubevirt"));
        buf.push('\n');

        buf.push_str(CYAN);
        buf.push_str(" \\__");
        buf.push_str(RED);
        buf.push_str("/¯¯\\");
        buf.push_str(GREEN);
        buf.push_str("__/");
        buf.push_str(RESET);
        buf.push_str("\tCDI:\t");
        buf.push_str(&self.component_summary("cdi"));
        buf.push('\n');

        buf.push_str(GREEN);
        buf.push_str(" /¯¯");
        buf.push_str(RED);
        buf.push_str("\\__/");
        buf.push_str(MAGENTA);
        buf.push_str("¯¯\\");
        buf.push_str(RESET);
        buf.push_str("\tZorvia API:\t");
        buf.push_str(&self.component_summary("zorvia-api"));
        buf.push('\n');

        buf.push_str(GREEN);
        buf.push_str(" \\__");
        buf.push_str(BLUE);
        buf.push_str("/¯¯\\");
        buf.push_str(MAGENTA);
        buf.push_str("__/");
        buf.push_str(RESET);
        buf.push_str("\tSnapshots:\t");
        buf.push_str(&self.component_summary("snapshots"));
        buf.push('\n');

        buf.push_str(BLUE);
        buf.push_str(BLUE);
        buf.push_str(BLUE);
        buf.push_str(" \\__/");
        buf.push_str(RESET);
        buf.push_str("\tRook Storage:\t");
        buf.push_str(&self.component_summary("rook"));
        buf.push('\n');
        buf.push('\n');

        let mut body_rows: Vec<Vec<String>> = Vec::new();

        for (name, state) in &self.pod_state {
            let kind = match state.type_name.as_str() {
                "DaemonSet" => format!("📦 {}", state.type_name),
                "Deployment" => format!("🚀 {}", state.type_name),
                other => other.to_string(),
            };
            body_rows.push(vec![kind, name.clone(), state.format_line()]);
        }

        let mut header = "📦 Containers:";
        for (name, phases) in &self.phase_count {
            body_rows.push(vec![
                header.to_string(),
                name.clone(),
                format_phase_count(phases),
            ]);
            header = "";
        }

        body_rows.push(vec![
            "🖥️  Cluster VMs:".to_string(),
            format!("{}/{} managed by Zorvia", self.vms_running, self.vms_total),
            String::new(),
        ]);

        header = "🖼️  Image versions";
        for (name, images) in &self.image_count {
            for (image, count) in images {
                body_rows.push(vec![
                    header.to_string(),
                    name.clone(),
                    format!("{}: {}", image, count),
                ]);
                header = "";
            }
        }

        header = "✨ Features";
        for feat in &self.features {
            body_rows.push(vec![
                header.to_string(),
                feat.name.clone(),
                feat.state.summary(),
            ]);
            header = "";
        }

        header = "❌ Errors:";
        for (deploy, pods) in &self.errors {
            for (pod, ec) in pods {
                for err in &ec.errors {
                    body_rows.push(vec![
                        header.to_string(),
                        deploy.clone(),
                        format!("{}\t{}", pod, err),
                    ]);
                    header = "";
                }
            }
        }

        header = "⚠️  Warnings:";
        for (deploy, pods) in &self.errors {
            for (pod, ec) in pods {
                for warn in &ec.warnings {
                    body_rows.push(vec![
                        header.to_string(),
                        deploy.clone(),
                        format!("{}\t{}", pod, warn),
                    ]);
                    header = "";
                }
            }
        }

        if !self.collection_errors.is_empty() {
            header = "🔌 Collection:";
            for err in &self.collection_errors {
                body_rows.push(vec![header.to_string(), String::new(), err.clone()]);
                header = "";
            }
        }

        buf.push_str(&align_rows(&body_rows));
        buf
    }

    pub fn is_ready(&self) -> bool {
        for key in ["kubevirt", "cdi", "zorvia-api"] {
            if let Some(c) = self.components.get(key) {
                if c.disabled || !c.errors.is_empty() {
                    return false;
                }
            } else {
                return false;
            }
        }
        self.collection_errors.is_empty()
    }
}

fn format_phase_count(m: &BTreeMap<String, i32>) -> String {
    let mut items = Vec::new();
    for (phase, count) in m {
        let (emoji, color) = match phase.as_str() {
            "Failed" | "Unknown" => ("❌ ", RED),
            "Running" => ("✅ ", GREEN),
            "Pending" => ("⌛ ", YELLOW),
            "Succeeded" => ("✅ ", GREEN),
            _ => ("", ""),
        };
        if color.is_empty() {
            items.push(format!("{}: {}", phase, count));
        } else {
            items.push(format!("{}: {}{}{}{}", phase, emoji, color, count, RESET));
        }
    }
    items.join(", ")
}

/// Minimal tabwriter: pad columns to max width + 4 spaces.
fn align_rows(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let visible = ansi::strip_ansi(cell).len();
            widths[i] = widths[i].max(visible);
        }
    }
    let mut out = String::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let visible = ansi::strip_ansi(cell).len();
            out.push_str(cell);
            if i + 1 < cols {
                let pad = widths[i].saturating_sub(visible) + 4;
                out.push_str(&" ".repeat(pad));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_status::ansi::strip_ansi;

    fn fixture() -> PlatformStatus {
        let mut s = PlatformStatus::default();
        s.components
            .insert("kubevirt".into(), ErrorCount::default());
        s.components.insert("cdi".into(), ErrorCount::default());
        s.components
            .insert("zorvia-api".into(), ErrorCount::default());
        s.components.insert(
            "snapshots".into(),
            ErrorCount {
                disabled: true,
                ..Default::default()
            },
        );
        s.components.insert(
            "rook".into(),
            ErrorCount {
                disabled: true,
                ..Default::default()
            },
        );
        s.pod_state.insert(
            "virt-handler".into(),
            PodStateCount {
                type_name: "DaemonSet".into(),
                desired: 2,
                ready: 2,
                available: 2,
                unavailable: 0,
            },
        );
        s.vms_running = 3;
        s.vms_total = 5;
        s.features.push(FeatureStatus {
            name: "Backup".into(),
            state: ErrorCount::default(),
        });
        s.features.push(FeatureStatus {
            name: "Kryton".into(),
            state: ErrorCount {
                disabled: true,
                ..Default::default()
            },
        });
        s
    }

    #[test]
    fn format_contains_logo_and_ok() {
        let text = strip_ansi(&fixture().format());
        assert!(text.contains("/¯¯\\"));
        assert!(text.contains("KubeVirt:"));
        assert!(text.contains("✅ OK"));
        assert!(text.contains("Rook Storage:"));
        assert!(text.contains("ℹ️  disabled"));
        assert!(text.contains("🖥️  Cluster VMs:"));
        assert!(text.contains("3/5 managed by Zorvia"));
        assert!(text.contains("✨ Features"));
        assert!(text.contains("Backup"));
        assert!(text.contains("Kryton"));
        assert!(text.contains("📦 DaemonSet"));
    }

    #[test]
    fn is_ready_requires_core() {
        let s = fixture();
        assert!(s.is_ready());
        let mut bad = fixture();
        bad.components
            .get_mut("kubevirt")
            .unwrap()
            .errors
            .push("x".into());
        assert!(!bad.is_ready());
    }
}
