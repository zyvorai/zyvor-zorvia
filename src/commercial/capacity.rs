// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Local capacity observation: a stable cluster identity plus node counts.
//!
//! Deliberately minimal. It reads node *labels* to count roles and nothing
//! else: no workload names, no guest contents, no node names.

use kube::api::{Api, ApiResource, DynamicObject, ListParams};

/// What the collector saw. On any failure `counts` is `None`, which the
/// billing side treats as "unknown", never as zero.
#[derive(Debug, Clone)]
pub struct LocalObservation {
    pub cluster_id: String,
    pub counts: Result<NodeCounts, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeCounts {
    pub total: u32,
    pub control_plane: u32,
    pub workers: u32,
}

const CONTROL_PLANE_LABELS: [&str; 2] = [
    "node-role.kubernetes.io/control-plane",
    "node-role.kubernetes.io/master",
];

/// Classify nodes from their label sets. A node carrying a control-plane
/// label counts as control plane even if it also schedules workloads; how
/// such mixed-role nodes are billed is a contract term (`node_treatment`).
pub fn count_nodes<'a>(
    label_sets: impl Iterator<Item = Option<&'a std::collections::BTreeMap<String, String>>>,
) -> NodeCounts {
    let mut total = 0u32;
    let mut control_plane = 0u32;
    for labels in label_sets {
        total += 1;
        if labels.is_some_and(|l| CONTROL_PLANE_LABELS.iter().any(|k| l.contains_key(*k))) {
            control_plane += 1;
        }
    }
    NodeCounts {
        total,
        control_plane,
        workers: total - control_plane,
    }
}

fn resource(kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: kind.into(),
        plural: plural.into(),
    }
}

/// Stable identity: `ZORVIA_CLUSTER_ID` if set, otherwise the UID of the
/// `kube-system` namespace, which is unique to the cluster and survives
/// node and Zorvia changes. The contract's covered-cluster identifier must
/// equal this value.
pub async fn observe_local(client: &kube::Client) -> Result<LocalObservation, String> {
    let cluster_id = match std::env::var("ZORVIA_CLUSTER_ID") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            let ns: Api<DynamicObject> =
                Api::all_with(client.clone(), &resource("Namespace", "namespaces"));
            ns.get("kube-system")
                .await
                .map_err(|e| format!("cannot determine cluster identity: {e}"))?
                .metadata
                .uid
                .ok_or_else(|| "kube-system namespace has no uid".to_string())?
        }
    };
    let nodes: Api<DynamicObject> = Api::all_with(client.clone(), &resource("Node", "nodes"));
    let counts = nodes
        .list(&ListParams::default())
        .await
        .map(|l| count_nodes(l.items.iter().map(|n| n.metadata.labels.as_ref())))
        .map_err(|e| format!("node discovery failed: {e}"));
    Ok(LocalObservation { cluster_id, counts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn labels(keys: &[&str]) -> BTreeMap<String, String> {
        keys.iter()
            .map(|k| (k.to_string(), String::new()))
            .collect()
    }

    #[test]
    fn counts_roles_from_labels_only() {
        let cp = labels(&["node-role.kubernetes.io/control-plane"]);
        let legacy = labels(&["node-role.kubernetes.io/master"]);
        let worker = labels(&["node-role.kubernetes.io/worker"]);
        let mixed = labels(&[
            "node-role.kubernetes.io/control-plane",
            "node-role.kubernetes.io/worker",
        ]);
        let sets = [Some(&cp), Some(&legacy), Some(&worker), None, Some(&mixed)];
        let c = count_nodes(sets.into_iter());
        assert_eq!(
            c,
            NodeCounts {
                total: 5,
                control_plane: 3,
                workers: 2
            }
        );
    }

    #[test]
    fn empty_cluster_is_zero_nodes_observed_not_unknown() {
        // An empty node list that *succeeded* is a real observation; failures
        // are carried as Err and never reach count_nodes.
        let c = count_nodes(std::iter::empty());
        assert_eq!(
            c,
            NodeCounts {
                total: 0,
                control_plane: 0,
                workers: 0
            }
        );
    }
}
