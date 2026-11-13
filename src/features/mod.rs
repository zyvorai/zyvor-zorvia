//! Feature maturity registry and experimental gates.
//!
//! Levels match docs/FEATURE_MATURITY.md. Model-only and experimental
//! surfaces require an explicit env opt-in before they are advertised or
//! executed from operator-facing paths.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Maturity {
    /// Real backend, persistence, RBAC, E2E coverage, upgrade path.
    Ga,
    /// Operational with documented limitations.
    Beta,
    /// Behind a feature flag; no production promise.
    Experimental,
    /// Types/library only — not a shipping capability.
    ModelOnly,
}

impl Maturity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ga => "ga",
            Self::Beta => "beta",
            Self::Experimental => "experimental",
            Self::ModelOnly => "model-only",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Feature {
    pub id: &'static str,
    pub name: &'static str,
    pub maturity: Maturity,
    pub notes: &'static str,
}

/// Canonical registry (keep in sync with docs/FEATURE_MATURITY.md).
pub const FEATURES: &[Feature] = &[
    Feature {
        id: "vm-lifecycle",
        name: "VM create/start/stop/delete",
        maturity: Maturity::Ga,
        notes: "KubeVirt-backed; server RBAC enforced",
    },
    Feature {
        id: "snapshots-restore",
        name: "Snapshots & restore",
        maturity: Maturity::Ga,
        notes: "KubeVirt VirtualMachineSnapshot; empty-disk caveat documented",
    },
    Feature {
        id: "live-migration",
        name: "Live migration",
        maturity: Maturity::Ga,
        notes: "Requires shared storage / migration-capable cluster",
    },
    Feature {
        id: "web-console",
        name: "Web console + auth",
        maturity: Maturity::Ga,
        notes: "JWT + scoped API tokens; OIDC opt-in with PKCE/JWKS",
    },
    Feature {
        id: "audit-trail",
        name: "Audit trail",
        maturity: Maturity::Ga,
        notes: "SQLite (or PostgreSQL) persistence; GET /api/audit/export JSONL; optional ZORVIA_AUDIT_JSONL sidecar (per replica)",
    },
    Feature {
        id: "pod-ops",
        name: "Pods: logs, exec, events, YAML, restart/delete",
        maturity: Maturity::Beta,
        notes: "cluster.admin only; needs pods delete + pods/log + pods/exec RBAC; exec/restart/delete audited; see docs/PODS.md",
    },
    Feature {
        id: "rescue-mode",
        name: "Rescue mode: hostname/SSH-key/enable-SSH via a Job-mounted disk",
        maturity: Maturity::Beta,
        notes: "cluster.admin only; needs batch/jobs RBAC; 3 of 5 UI operations wired (reset-password, install-packages, inspect deferred); see docs/RESCUE.md",
    },
    Feature {
        id: "schedulers",
        name: "Backup/power/alert/warm-pool schedulers",
        maturity: Maturity::Beta,
        notes: "Lease leader election; still in-process with API",
    },
    Feature {
        id: "rook-storage",
        name: "Rook-Ceph management",
        maturity: Maturity::Beta,
        notes: "Manages existing Ceph CRs; bootstrap SA optional",
    },
    Feature {
        id: "atlas-storage",
        name: "Atlas storage integration: inventory, backend/RBD/object-store, AI insights, observability, governance, DataBridge",
        maturity: Maturity::Beta,
        notes: "Optional, gated on ATLAS_URL; storage.admin; disaster recovery is scaffolding on Atlas's own side, unverified without a 2nd real Ceph cluster; see docs/ATLAS_INTEGRATION.md",
    },
    Feature {
        id: "postgres-store",
        name: "PostgreSQL store and two active replicas",
        maturity: Maturity::Beta,
        notes: "Users/tokens, operations, audit and JSON documents in PostgreSQL (ZORVIA_DATABASE_URL); values-ha.yaml runs two replicas. Live-verified on a single-node lab; DB failover, leader failover under load and node-loss RTO not measured; commercial records and the JSONL sidecar stay per replica — docs/POSTGRES.md, docs/HA.md",
    },
    Feature {
        id: "guest-agent-integration",
        name: "Zyvor guest agent: install, guest views, snapshot hooks, drill probe",
        maturity: Maturity::Beta,
        notes: "Debian-family guests; read-only method set over the QGA channel; freeze needs the systemd drop-in Zorvia writes; Windows/RPM guests and real database-flush hooks not verified — docs/GUEST_AGENT.md",
    },
    Feature {
        id: "helm-chart",
        name: "Helm production chart",
        maturity: Maturity::Beta,
        notes: "Lab + production values; GHCR digest pinning recommended",
    },
    Feature {
        id: "oidc",
        name: "OIDC / enterprise SSO",
        maturity: Maturity::Beta,
        notes: "Opt-in via ZORVIA_OIDC_ENABLED=1; PKCE + JWKS; see docs/OIDC_LAB.md",
    },
    Feature {
        id: "ai-troubleshoot",
        name: "AI troubleshooting wizard",
        maturity: Maturity::ModelOnly,
        notes: "Rules demo — not ML RCA; requires ZORVIA_EXPERIMENTAL=1",
    },
    Feature {
        id: "dr-replication",
        name: "DR replication pairs",
        maturity: Maturity::ModelOnly,
        notes: "In-memory model; does not move data",
    },
    Feature {
        id: "incremental-backup",
        name: "Incremental backup fields",
        maturity: Maturity::ModelOnly,
        notes: "Executions remain full VolumeSnapshots today",
    },
    Feature {
        id: "s3-immutable-backup",
        name: "Off-cluster S3 backups (encrypted, Object Lock), restore, recovery drills",
        maturity: Maturity::Beta,
        notes: "Agent Job uploads checksummed, AES-GCM, Object-Lock copies with read-back verification; restore, recovery drills and an encryption-key check (docs/OFFCLUSTER_BACKUP.md). Verified live end to end on two CSI drivers (Rook-Ceph RBD with Filesystem and Block volumes, and the Kubernetes hostpath driver) on KubeVirt 1.9 / CDI 1.66 with an Atlas-provisioned RGW bucket: snapshot→temp PVC→agent Job→encrypted upload→read-back, restore (sha256 identical), a recovery drill with serial-console boot proof, and the key check; Object Lock earlier against SeaweedFS. Limits: single-node lab, one Kubernetes/KubeVirt/CDI version, Object Lock not yet run on RGW, drills do no application-level checks (docs/SUPPORT_MATRIX.md). Plan API at /api/v1/enterprise/s3-backup/plan",
    },
    Feature {
        id: "cross-cluster-dr",
        name: "Cross-cluster VM mobility / DR",
        maturity: Maturity::Experimental,
        notes: "Phase 5 — plan + POST /api/v1/enterprise/cross-cluster-dr/run (same-cluster MinIO/S3 backup→restore rehearsal; lab: scripts/lab-minio-backup.sh)",
    },
    Feature {
        id: "transiva-migration",
        name: "VMware → KubeVirt via Transiva",
        maturity: Maturity::Experimental,
        notes: "Plan API only; superseded by vm-import-h2kvm (drives h2kvm directly, no Transiva). Plan API at /api/v1/enterprise/transiva/plan",
    },
    Feature {
        id: "vm-import-h2kvm",
        name: "VMware → KubeVirt import via h2kvm Jobs",
        maturity: Maturity::Experimental,
        notes: "Requires ZORVIA_EXPERIMENTAL=1; mode=stub Job-only; mode=local Job+stopped emptyDisk VM; vsphere needs ZORVIA_H2KVM_IMAGE; not yet exercised against a real vCenter",
    },
    Feature {
        id: "golden-image-pipeline",
        name: "Golden-image build/scan/sign/promote",
        maturity: Maturity::Experimental,
        notes: "Plan marks scan:deferred/sign:deferred; /run applies CDI DV/DS (build+promote); cosign/scan operator-owned",
    },
    Feature {
        id: "commercial-contracts",
        name: "Commercial catalog, quote requests, contracts and coverage",
        maturity: Maturity::Beta,
        notes: "Contracts, coverage, support cases with contract-hours timers, redacted diagnostic bundles (preview/download; upload only on explicit confirm). Service engagements and managed-operation records (records only; no cluster access, remote operation off by default). Capacity reconciliation (unknown is never zero) and invoice records. Single-instance SQLite (ZORVIA_COMMERCIAL_DB); no notifications, payments, remote access or log collection; expiry never affects VMs — docs/COMMERCIAL_OFFERINGS.md",
    },
    Feature {
        id: "gpu-sriov-numa",
        name: "GPU/SR-IOV/NUMA placement",
        maturity: Maturity::Experimental,
        notes: "GPU / host-device passthrough and SR-IOV at VM create with scheduling preflight, inventory, permit/remove (Helm devices.managePermitted), NUMA/dedicated-CPU/hugepage checks (docs/DEVICES.md). Placement plan runs live inventory preflight. Lab: deploy/lab-fake-gpu-device-plugin.yaml + scripts/lab-fake-gpu-device.sh; not proven on real GPU/SR-IOV hardware",
    },
    Feature {
        id: "fleet-multicluster",
        name: "Multi-cluster fleet inventory",
        maturity: Maturity::Experimental,
        notes: "Live read-only inventory with explicit enrollment; GET /api/v1/enterprise/fleet; lab dual-enrollment via scripts/lab-fleet-dual-enroll.sh; true multi-API validation pending",
    },
];

pub fn experimental_enabled() -> bool {
    match std::env::var("ZORVIA_EXPERIMENTAL") {
        Ok(v) => {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

pub fn feature(id: &str) -> Option<&'static Feature> {
    FEATURES.iter().find(|f| f.id == id)
}

/// Returns true if a model-only / experimental feature may be used.
pub fn allow_non_ga(id: &str) -> bool {
    match feature(id).map(|f| f.maturity) {
        Some(Maturity::Ga) | Some(Maturity::Beta) => true,
        Some(Maturity::Experimental) | Some(Maturity::ModelOnly) => experimental_enabled(),
        None => experimental_enabled(),
    }
}

/// Phase 5 enterprise feature toggles (all require experimental mode too).
pub fn enterprise_flag(env_key: &str) -> bool {
    experimental_enabled()
        && match std::env::var(env_key) {
            Ok(v) => {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true")
            }
            Err(_) => false,
        }
}

pub fn registry_json() -> serde_json::Value {
    serde_json::json!({
        "experimental_enabled": experimental_enabled(),
        "features": FEATURES.iter().map(|f| serde_json::json!({
            "id": f.id,
            "name": f.name,
            "maturity": f.maturity.as_str(),
            "notes": f.notes,
            "available": allow_non_ga(f.id),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ga_always_allowed() {
        assert!(allow_non_ga("vm-lifecycle"));
    }

    #[test]
    fn model_only_blocked_by_default() {
        // Do not set ZORVIA_EXPERIMENTAL in unit tests.
        if experimental_enabled() {
            return;
        }
        assert!(!allow_non_ga("ai-troubleshoot"));
        assert!(!allow_non_ga("dr-replication"));
    }
}
