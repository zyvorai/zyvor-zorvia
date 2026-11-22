// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenancePolicy {
    LiveMigrate,
    Stop,
    Skip,
    Block,
}

impl MaintenancePolicy {
    pub fn parse(value: Option<&str>) -> Self {
        match value
            .unwrap_or("migrate")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "stop" | "shutdown" => Self::Stop,
            "skip" | "pinned" => Self::Skip,
            "block" | "deny" => Self::Block,
            _ => Self::LiveMigrate,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceWorkload {
    pub name: String,
    pub namespace: String,
    pub node: String,
    pub policy: MaintenancePolicy,
    pub priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceAction {
    Migrate,
    Stop,
    Skip,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceStep {
    pub workload: MaintenanceWorkload,
    pub action: MaintenanceAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenancePlan {
    pub node: String,
    pub cordon: bool,
    pub steps: Vec<MaintenanceStep>,
    pub migration_batches: Vec<Vec<String>>,
    pub blockers: Vec<String>,
    pub estimated_seconds: u64,
}

impl MaintenancePlan {
    pub fn can_enter(&self) -> bool {
        self.blockers.is_empty()
    }
    pub fn vm_count(&self) -> usize {
        self.steps.len()
    }
}

pub struct MaintenancePlanner {
    pub max_parallel: usize,
    pub avg_migration_seconds: u64,
}
impl MaintenancePlanner {
    pub fn new(max_parallel: usize) -> Self {
        Self {
            max_parallel: max_parallel.max(1),
            avg_migration_seconds: 120,
        }
    }
    pub fn plan(&self, node: &str, mut workloads: Vec<MaintenanceWorkload>) -> MaintenancePlan {
        workloads.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.name.cmp(&b.name))
        });
        let mut steps = Vec::new();
        let mut migrations = Vec::new();
        let mut blockers = Vec::new();
        for w in workloads {
            let (action, reason) = match w.policy {
                MaintenancePolicy::LiveMigrate => {
                    migrations.push(format!("{}/{}", w.namespace, w.name));
                    (
                        MaintenanceAction::Migrate,
                        "live-migrate before host maintenance",
                    )
                }
                MaintenancePolicy::Stop => (
                    MaintenanceAction::Stop,
                    "workload policy requires a controlled stop",
                ),
                MaintenancePolicy::Skip => {
                    blockers.push(format!("{}/{} is pinned/skip", w.namespace, w.name));
                    (MaintenanceAction::Skip, "workload is pinned to the node")
                }
                MaintenancePolicy::Block => {
                    blockers.push(format!(
                        "{}/{} explicitly blocks maintenance",
                        w.namespace, w.name
                    ));
                    (
                        MaintenanceAction::Block,
                        "workload policy blocks maintenance",
                    )
                }
            };
            steps.push(MaintenanceStep {
                workload: w,
                action,
                reason: reason.into(),
            });
        }
        let migration_batches = migrations
            .chunks(self.max_parallel)
            .map(|c| c.to_vec())
            .collect::<Vec<_>>();
        let estimated_seconds = (migration_batches.len() as u64) * self.avg_migration_seconds;
        MaintenancePlan {
            node: node.into(),
            cordon: true,
            steps,
            migration_batches,
            blockers,
            estimated_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(n: &str, p: MaintenancePolicy, priority: u8) -> MaintenanceWorkload {
        MaintenanceWorkload {
            name: n.into(),
            namespace: "default".into(),
            node: "n1".into(),
            policy: p,
            priority,
        }
    }
    #[test]
    fn batches_migrations() {
        let p = MaintenancePlanner::new(2).plan(
            "n1",
            vec![
                w("a", MaintenancePolicy::LiveMigrate, 1),
                w("b", MaintenancePolicy::LiveMigrate, 1),
                w("c", MaintenancePolicy::LiveMigrate, 1),
            ],
        );
        assert_eq!(p.migration_batches.len(), 2);
        assert!(p.can_enter());
    }
    #[test]
    fn pinned_workload_blocks() {
        let p = MaintenancePlanner::new(2).plan("n1", vec![w("a", MaintenancePolicy::Skip, 1)]);
        assert!(!p.can_enter());
        assert_eq!(p.blockers.len(), 1);
    }
    #[test]
    fn priority_orders_steps() {
        let p = MaintenancePlanner::new(1).plan(
            "n1",
            vec![
                w("low", MaintenancePolicy::Stop, 1),
                w("high", MaintenancePolicy::Stop, 10),
            ],
        );
        assert_eq!(p.steps[0].workload.name, "high");
    }
    #[test]
    fn zero_parallel_is_safe() {
        assert_eq!(MaintenancePlanner::new(0).max_parallel, 1);
    }
}
