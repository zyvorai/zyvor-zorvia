// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Explainable placement and rebalance advisor. This is intentionally recommendation-first:
//! it does not silently move workloads. Operators can review a plan and then invoke the
//! existing Zorvia/KubeVirt migration path.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeSnapshot {
    pub name: String,
    pub allocatable_cpu: f64,
    pub allocatable_memory_gib: f64,
    pub used_cpu: f64,
    pub used_memory_gib: f64,
    pub vm_count: usize,
    pub labels: BTreeMap<String, String>,
    pub taints: BTreeSet<String>,
    pub unschedulable: bool,
}
impl NodeSnapshot {
    pub fn free_cpu(&self) -> f64 {
        (self.allocatable_cpu - self.used_cpu).max(0.0)
    }
    pub fn free_memory_gib(&self) -> f64 {
        (self.allocatable_memory_gib - self.used_memory_gib).max(0.0)
    }
    pub fn load_score(&self) -> f64 {
        let c = if self.allocatable_cpu > 0.0 {
            self.used_cpu / self.allocatable_cpu
        } else {
            1.0
        };
        let m = if self.allocatable_memory_gib > 0.0 {
            self.used_memory_gib / self.allocatable_memory_gib
        } else {
            1.0
        };
        ((c + m) / 2.0).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkloadRequirements {
    pub name: String,
    pub namespace: String,
    pub cpu: f64,
    pub memory_gib: f64,
    pub current_node: Option<String>,
    pub required_labels: BTreeMap<String, String>,
    pub avoid_nodes: BTreeSet<String>,
    pub preferred_zones: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateScore {
    pub node: String,
    pub eligible: bool,
    pub score: f64,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlacementRecommendation {
    pub workload: String,
    pub selected_node: Option<String>,
    pub candidates: Vec<CandidateScore>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceMove {
    pub workload: String,
    pub namespace: String,
    pub from_node: String,
    pub to_node: String,
    pub expected_improvement: f64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PlacementAdvisor {
    pub zone_label: String,
}
impl Default for PlacementAdvisor {
    fn default() -> Self {
        Self {
            zone_label: "topology.kubernetes.io/zone".into(),
        }
    }
}
impl PlacementAdvisor {
    pub fn recommend(
        &self,
        w: &WorkloadRequirements,
        nodes: &[NodeSnapshot],
    ) -> PlacementRecommendation {
        let mut candidates = nodes.iter().map(|n| self.score(w, n)).collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.node.cmp(&b.node))
        });
        let selected_node = candidates
            .iter()
            .find(|c| c.eligible)
            .map(|c| c.node.clone());
        PlacementRecommendation {
            workload: format!("{}/{}", w.namespace, w.name),
            selected_node,
            candidates,
        }
    }
    fn score(&self, w: &WorkloadRequirements, n: &NodeSnapshot) -> CandidateScore {
        let mut reasons = Vec::new();
        if n.unschedulable {
            return CandidateScore {
                node: n.name.clone(),
                eligible: false,
                score: 0.0,
                reasons: vec!["node is unschedulable/maintenance".into()],
            };
        }
        if w.avoid_nodes.contains(&n.name) {
            return CandidateScore {
                node: n.name.clone(),
                eligible: false,
                score: 0.0,
                reasons: vec!["node is explicitly avoided".into()],
            };
        }
        for (k, v) in &w.required_labels {
            if n.labels.get(k) != Some(v) {
                return CandidateScore {
                    node: n.name.clone(),
                    eligible: false,
                    score: 0.0,
                    reasons: vec![format!("required label {}={} does not match", k, v)],
                };
            }
        }
        if n.free_cpu() + 1e-9 < w.cpu || n.free_memory_gib() + 1e-9 < w.memory_gib {
            return CandidateScore {
                node: n.name.clone(),
                eligible: false,
                score: 0.0,
                reasons: vec![format!(
                    "insufficient capacity: free {:.2} CPU / {:.2} GiB",
                    n.free_cpu(),
                    n.free_memory_gib()
                )],
            };
        }
        let cpu_headroom = if n.allocatable_cpu > 0.0 {
            (n.free_cpu() - w.cpu) / n.allocatable_cpu
        } else {
            0.0
        };
        let mem_headroom = if n.allocatable_memory_gib > 0.0 {
            (n.free_memory_gib() - w.memory_gib) / n.allocatable_memory_gib
        } else {
            0.0
        };
        let mut score = ((cpu_headroom + mem_headroom) / 2.0 * 80.0).clamp(0.0, 80.0)
            + 20.0 / (n.vm_count as f64 + 1.0);
        if !w.preferred_zones.is_empty() {
            let zone = n.labels.get(&self.zone_label);
            if zone.is_some_and(|z| w.preferred_zones.contains(z)) {
                score += 10.0;
                reasons.push(format!("preferred zone {}", zone.unwrap()));
            } else {
                reasons.push("outside preferred zones".into());
            }
        }
        reasons.push(format!(
            "post-placement headroom: CPU {:.0}%, memory {:.0}%",
            cpu_headroom.max(0.0) * 100.0,
            mem_headroom.max(0.0) * 100.0
        ));
        CandidateScore {
            node: n.name.clone(),
            eligible: true,
            score: score.clamp(0.0, 100.0),
            reasons,
        }
    }
    pub fn rebalance(
        &self,
        workloads: &[WorkloadRequirements],
        nodes: &[NodeSnapshot],
        threshold: f64,
        max_moves: usize,
    ) -> Vec<RebalanceMove> {
        if nodes.len() < 2 {
            return vec![];
        }
        let avg = nodes.iter().map(NodeSnapshot::load_score).sum::<f64>() / nodes.len() as f64;
        let overloaded = nodes
            .iter()
            .filter(|n| n.load_score() > avg + threshold)
            .map(|n| n.name.clone())
            .collect::<BTreeSet<_>>();
        let mut moves = Vec::new();
        for w in workloads.iter().filter(|w| {
            w.current_node
                .as_ref()
                .is_some_and(|n| overloaded.contains(n))
        }) {
            let from = w.current_node.clone().unwrap();
            let rec = self.recommend(w, nodes);
            if let Some(to) = rec.selected_node {
                if to != from {
                    let from_load = nodes
                        .iter()
                        .find(|n| n.name == from)
                        .map(NodeSnapshot::load_score)
                        .unwrap_or(0.0);
                    let to_load = nodes
                        .iter()
                        .find(|n| n.name == to)
                        .map(NodeSnapshot::load_score)
                        .unwrap_or(1.0);
                    let improvement = (from_load - to_load).max(0.0);
                    if improvement > threshold / 2.0 {
                        moves.push(RebalanceMove{workload:w.name.clone(),namespace:w.namespace.clone(),from_node:from,to_node:to,expected_improvement:improvement,reason:"reduce node CPU/memory imbalance while preserving hard placement constraints".into()});
                    }
                }
            }
            if moves.len() >= max_moves {
                break;
            }
        }
        moves
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(n: &str, used: f64) -> NodeSnapshot {
        NodeSnapshot {
            name: n.into(),
            allocatable_cpu: 8.0,
            allocatable_memory_gib: 32.0,
            used_cpu: used,
            used_memory_gib: used * 4.0,
            vm_count: used as usize,
            labels: BTreeMap::new(),
            taints: BTreeSet::new(),
            unschedulable: false,
        }
    }
    fn work() -> WorkloadRequirements {
        WorkloadRequirements {
            name: "vm".into(),
            namespace: "default".into(),
            cpu: 1.0,
            memory_gib: 2.0,
            current_node: Some("n1".into()),
            required_labels: BTreeMap::new(),
            avoid_nodes: BTreeSet::new(),
            preferred_zones: BTreeSet::new(),
        }
    }
    #[test]
    fn chooses_less_loaded_node() {
        let r = PlacementAdvisor::default().recommend(&work(), &[node("n1", 7.0), node("n2", 1.0)]);
        assert_eq!(r.selected_node.as_deref(), Some("n2"));
    }
    #[test]
    fn rejects_unschedulable() {
        let mut n = node("n", 0.0);
        n.unschedulable = true;
        assert!(
            !PlacementAdvisor::default()
                .recommend(&work(), &[n])
                .candidates[0]
                .eligible
        );
    }
    #[test]
    fn honors_required_labels() {
        let mut w = work();
        w.required_labels.insert("gpu".into(), "true".into());
        let n = node("n", 0.0);
        assert!(!PlacementAdvisor::default().recommend(&w, &[n]).candidates[0].eligible);
    }
    #[test]
    fn capacity_is_hard_constraint() {
        let mut w = work();
        w.cpu = 99.0;
        assert!(PlacementAdvisor::default()
            .recommend(&w, &[node("n", 0.0)])
            .selected_node
            .is_none());
    }
    #[test]
    fn rebalance_suggests_move() {
        let out = PlacementAdvisor::default().rebalance(
            &[work()],
            &[node("n1", 7.0), node("n2", 1.0)],
            0.1,
            2,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_node, "n2");
    }
}
