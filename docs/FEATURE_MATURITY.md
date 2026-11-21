# Feature maturity registry

Levels used by Zorvia (also exposed at `GET /api/v1/features` when the API is running):

| Level | Meaning |
|-------|---------|
| **GA** | Real backend, persistence, RBAC, tests, upgrade path |
| **Beta** | Operational with documented limitations |
| **Experimental** | Behind `ZORVIA_EXPERIMENTAL=1`; no production promise |
| **Model only** | Internal types / demos — not marketed as working |

## Current matrix (0.3.3+)

| ID | Feature | Level | Notes |
|----|---------|-------|-------|
| `vm-lifecycle` | VM create/start/stop/delete | GA | KubeVirt + server RBAC |
| `snapshots-restore` | Snapshots & restore | GA | Empty-disk caveat in docs |
| `live-migration` | Live migration | GA | Needs shared storage |
| `web-console` | Web console + auth | GA | JWT + tokens; OIDC opt-in |
| `audit-trail` | Audit trail | GA | SQLite (or [PostgreSQL](POSTGRES.md)) + `GET /api/audit/export` JSONL; optional `ZORVIA_AUDIT_JSONL` sidecar (per replica) |
| `pod-ops` | Pods: logs, exec, events, YAML, restart/delete | Beta | `cluster.admin` only; `pods` delete + `pods/log` + `pods/exec` RBAC; exec, restart and delete audited — [PODS.md](PODS.md) |
| `rescue-mode` | Rescue mode: hostname/SSH-key/enable-SSH via a Job-mounted disk | Beta | `cluster.admin` only; 3 of 5 UI operations wired (reset-password, install-packages, inspect deferred); needs on-cluster verification of the mounted PVC's disk-image path before trusting it against a production VM — [RESCUE.md](RESCUE.md) |
| `autoscaler-loop` | Autoscaler reconcile loop (VM pool replicas / VM vCPU) | Experimental | Opt-in `ZORVIA_AUTOSCALER_LOOP=1`; recommendation-only by default; decision logic unit-tested with fakes; executor not live-verified; current load source is simulated so nothing is applied — [AUTOSCALER_LOOP.md](AUTOSCALER_LOOP.md) |
| `rescue-mode` | Rescue mode: hostname/SSH-key/enable-SSH via a Job-mounted disk | Beta | `cluster.admin` only; 3 original operations (the other three are the separate `rescue-ops-extended` row); needs on-cluster verification of the mounted PVC's disk-image path before trusting it against a production VM — [RESCUE.md](RESCUE.md) |
| `rescue-ops-extended` | Rescue mode: reset-password, install-packages, inspect | Experimental | Gated by `ZORVIA_EXPERIMENTAL=1`; `cluster.admin` only, audited; password travels in a per-Job Secret (needs `secrets` create/delete/patch RBAC); Job-spec generation and validation unit-tested only — never run against a live cluster or real guest disk — [RESCUE.md](RESCUE.md) |
| `schedulers` | Backup/power/alert/warm-pool | Beta | Lease election; in-process |
| `rook-storage` | Rook-Ceph management | Beta | Bootstrap SA optional |
| `atlas-storage` | Atlas storage integration: inventory, backend/RBD/object-store lifecycle, AI insights, observability, governance, DataBridge | Beta | Optional, gated on `ATLAS_URL`; `storage.admin`; disaster recovery is scaffolding on Atlas's own side — unverified without a 2nd real Ceph cluster — see [ATLAS_INTEGRATION.md](ATLAS_INTEGRATION.md) |
| `postgres-store` | PostgreSQL store and two active replicas | Beta | Users/tokens, operations, audit and JSON documents in PostgreSQL; `values-ha.yaml` runs two replicas. Live-verified on a single-node lab; DB failover, leader failover under load and node-loss RTO not measured; commercial records and the JSONL sidecar stay per replica — [POSTGRES.md](POSTGRES.md), [HA.md](HA.md) |
| `guest-agent-integration` | Zyvor guest agent: install, guest views, snapshot hooks, drill probe | Beta | Debian-family guests; read-only method set; freeze needs the drop-in Zorvia writes; Windows/RPM and real DB-flush hooks not verified — [GUEST_AGENT.md](GUEST_AGENT.md) |
| `helm-chart` | Helm chart | Beta | Lab + hardened production values |
| `oidc` | OIDC / SSO | Beta | `ZORVIA_OIDC_ENABLED=1` + PKCE/JWKS; lab bake-off [OIDC_LAB.md](OIDC_LAB.md) |
| `saml` | SAML 2.0 SSO | Experimental | Partial: metadata, AuthnRequest, validator; **no signature verification, ACS returns 501**, no login possible. [SAML.md](SAML.md) |
| `ai-troubleshoot` | AI troubleshooting | Model only | Rules demo, not ML RCA |
| `dr-replication` | DR replication pairs | Experimental | Durable pairs (file or PostgreSQL), leader-gated loop takes verified off-cluster backups at the RPO interval, lag/RPO/RTO status, alert hooks, dry-run-first confirmed failover restoring as `{vm}-dr` (never over a running/existing VM), failback plan. Needs `ZORVIA_EXPERIMENTAL=1` + `ZORVIA_FEATURE_DR_REPLICATION=1` and an off-cluster target. Unit-tested with fake backends only, not run against a live cluster; same-cluster object store, no site failover or traffic repointing — [DR_REPLICATION.md](DR_REPLICATION.md). The older CLI model (`dr::replication`) is unchanged and still in-memory. |
| `incremental-backup` | Incremental backup fields | Model only | Executions still full snapshots |
| `s3-immutable-backup` | Off-cluster S3 backups, restore, recovery drills, key check | Beta | Verified live end to end on two CSI drivers (Rook-Ceph RBD with Filesystem and Block volumes; the Kubernetes hostpath driver) on KubeVirt 1.9 / CDI 1.66 with an Atlas-provisioned RGW bucket: encrypted upload with read-back, restore with identical SHA-256, recovery drill with serial-console boot proof, encryption-key check. Limits: single-node lab, one Kubernetes/KubeVirt/CDI version, Object Lock not yet run on RGW (earlier on SeaweedFS), drills do no application-level checks — see [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) and docs/OFFCLUSTER_BACKUP.md |
| `cross-cluster-dr` | Cross-cluster DR | Experimental | Plan + `POST …/cross-cluster-dr/run` queues off-cluster backup then restore-as-`{vm}-dr` when `ZORVIA_BACKUP_S3_*` is set (same-cluster object-store rehearsal; not site failover). Lab: `scripts/lab-minio-backup.sh`. |
| `transiva-migration` | Transiva migration | Experimental | Plan API only; superseded by `vm-import-h2kvm` |
| `vm-import-h2kvm` | VMware → KubeVirt import via h2kvm Jobs | Experimental | Gated by `ZORVIA_EXPERIMENTAL=1`; admin console, fail-closed preflight, NIC mapping, cutover, cancel; `mode=stub` Job-only; `mode=local` Job + stopped emptyDisk VM; vsphere needs `ZORVIA_H2KVM_IMAGE`; not yet exercised against a real vCenter |
| `image-upload` | Disk image upload via the CDI upload proxy | Experimental | `ZORVIA_EXPERIMENTAL=1` + `ZORVIA_CDI_UPLOAD_PROXY_URL`; streamed with size cap and SHA-256, magic-byte format detection (CDI converts to raw); tested against a local mock proxy only, not a live CDI; web console upload form on `/app/disk-images` (unit-tested only) — [IMAGE_UPLOAD.md](IMAGE_UPLOAD.md) |
| `volume-encryption` | Volume envelope encryption (data keys wrapped by local key / Vault Transit) | Experimental | Wrapped key Secret, re-wrap rotation, audited plan/apply of an encrypted DataVolume via StorageClass parameters; cloud KMS stub only; unit-tested, no live-cluster or CSI verification; CSI does not consume the Zorvia key — [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md) |
| `golden-image-pipeline` | Golden-image pipeline | Experimental | Plan marks `scan:deferred` / `sign:deferred`; `/run` applies CDI DataVolume+DataSource (build+promote). Cosign/scan remain operator-owned. |
| `golden-image-pipeline` | Golden-image pipeline | Experimental | `/run` applies the CDI DataVolume, runs operator-configured scan (trivy-style JSON + severity threshold) and cosign sign/verify Jobs, and only then promotes the DataSource; a failing gate blocks promotion (409). Unconfigured steps stay `deferred`. Unit-tested with mocked Jobs and fixtures; not run against a live cluster, trivy or cosign. Registry sources only. |
| `gpu-sriov-numa` | GPU/SR-IOV/NUMA | Experimental | Create/preflight/permit + placement plan with **live** inventory preflight; lab fake GPU (`deploy/lab-fake-gpu-device-plugin.yaml` + `scripts/lab-fake-gpu-device.sh`); not proven on real GPU/SR-IOV hardware — [DEVICES.md](DEVICES.md) |
| `fleet-multicluster` | Fleet multi-cluster | Experimental | Live read-only inventory + admin console; lab dual-enrollment script (`scripts/lab-fleet-dual-enroll.sh`); true multi-API validation pending — [FLEET.md](FLEET.md) |

Set `ZORVIA_EXPERIMENTAL=1` only in non-production environments to exercise model-only / experimental surfaces.
See [PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) for env flags and endpoints.
