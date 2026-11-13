# Validated combinations and measured results

This page records what has actually been run, on which versions, with what outcome. It is
evidence, not a promise: a combination not listed here has not been validated, and a result
here is a single run on one lab host, not a benchmark. Re-run `tests/e2e/guest.sh` on your own
stack before relying on any number.

## Reference lab (single node)

| Component | Version |
|---|---|
| Zorvia | `main` including PRs #145 to #148 (built and deployed 2026-10-01) |
| Kubernetes | v1.36.4+k3s1, one control-plane/worker node (12 vCPU, 30 GiB) |
| OS / kernel / runtime | Ubuntu 26.04.1 LTS, 7.0.0-34, containerd 2.3.4 |
| KubeVirt | v1.9.0 |
| CDI | v1.66.0 |
| Storage | Rook v1.20.8, Ceph 19.2.3 (single OSD, `HEALTH_WARN` as expected for one node), RBD class `zyvor-rbd-prod` |
| Second CSI driver | Kubernetes `csi-driver-host-path` v1.18.0 (`csi-hostpath-sc`, node-local, supports snapshots), added 2026-10-02 |
| Object storage | Ceph RGW bucket provisioned through Atlas |
| Guest image | `quay.io/containerdisks/ubuntu:24.04` |

## Results, 2026-10-01

Produced by `E2E_BACKUP=1 E2E_STORAGE_CLASS=zyvor-rbd-prod tests/e2e/guest.sh` (see
[tests/e2e/README.md](../tests/e2e/README.md)).

| Scenario | Result | Time | Detail |
|---|---|---|---|
| Create, boot, guest ready | PASS | 35 s | Guest has an IP and `Ready=True`; no guest agent in this image |
| Snapshot of a containerdisk guest | PASS | 5 s | Configuration only (no PVC); the API says so in a warning |
| Data snapshot on CSI storage | PASS | 104 s | PVC-backed guest on RBD; `VolumeSnapshot` ready |
| Off-cluster backup | PASS | 95 s | 1 GiB disk, AES-256-GCM, SHA-256, read-back verified |
| Restore from the backup | PASS | 49 s | New VM and PVC created from the manifest |
| Recovery drill | PASS | 216 s | Restore 68 s, boot 131 s, boot proof from the serial console, torn down cleanly |
| API pod restart, guest untouched | PASS | 43 s | `rollout status` to healthy; session still valid; VM still running |
| Live migration under load | SKIP | n/a | Needs two schedulable nodes |

The security checks in [SECURITY.md](../SECURITY.md) were also run live against this build
(permission matrix as Viewer/User/namespace-restricted, socket gating, MFA re-enrolment, logout,
password change, lockout).

## Results on the second CSI driver, 2026-10-02

`E2E_STORAGE_CLASS=csi-hostpath-sc E2E_BACKUP=1 tests/e2e/guest.sh` (same stack, guest agent off for this run):

| Scenario | Result | Time |
|---|---|---|
| Data snapshot on CSI storage | PASS | 140 s |
| Off-cluster backup (encrypted, read-back verified, 1 GiB) | PASS | 149 s |
| Restore | PASS | 57 s |
| Recovery drill | PASS | 251 s (restore 104 s, boot 132 s) |
| API restart, guest untouched | PASS | 48 s |

## Guest agent (Zyvor guest agent via `guest_agent: zyvor`), 2026-10-01 and 2026-10-02

Installed through the create API by cloud-init and reported connected by KubeVirt after 130 to 355 s
across runs. 2026-10-02 findings and results:

- **Online snapshots of PVC-backed guests failed at the freeze** with the 1.2.4 package (agent unprivileged, no
  capabilities: `fsfreeze: Operation not permitted`, measured on a fresh guest). Worked around by a systemd drop-in Zorvia's
  cloud-init wrote (ambient `CAP_SYS_ADMIN` only; with it `guest-fsfreeze-freeze`/`thaw` succeed, an online snapshot becomes
  ready and an off-cluster backup succeeds), then **fixed in GuestKit 1.2.5** (freeze through the privileged helper). The 1.2.5
  binaries were verified in a real guest with an empty capability set: freeze/thaw succeed and the helper socket survives an agent
  restart. Zorvia now pins 1.2.5 and writes the drop-in only for a mirrored or older package.
- Read-only guest views (`/guest/agent`, `/guest/inventory/*`) returned real data (665 packages, 122 certificates, users,
  security posture, container inventory).
- Snapshots and backups recorded `guest_quiesce: application` (the guest's pre-snapshot hooks ran; the lab guest has only the
  default flush scripts, which report the database as not active).
- Full E2E with `guest_agent: zyvor` (snapshot, data snapshot, backup, restore, drill, API restart) passed; the drill still
  falls back to the serial console because the E2E guest's root is an ephemeral containerdisk.

See [GUEST_AGENT.md](GUEST_AGENT.md).

## PostgreSQL store and two replicas, 2026-10-02 (single-node lab, Postgres 17 in the cluster)

| Check | Result |
|---|---|
| Import of the existing admin from `auth.db`, sign-in, restart persistence | PASS |
| Two replicas: logout on one rejects the token on the other | PASS |
| Operations queue on Postgres: boot, snapshots, encrypted backup, restore, drill, API restart | PASS |
| Audit trail: 199 existing entries imported; entries written on one replica visible on the other | PASS |
| JSON documents: an alert rule created on one replica visible on the other, deleted from the other | PASS |
| `helm template`: refuses `replicaCount > 1` without a database, leader election or the local-state acknowledgement; renders no PVC and no `Recreate` for [values-ha.yaml](../charts/zorvia/values-ha.yaml) | PASS (not installed with Helm on the lab; the lab is deployed by script) |

| Leader pod killed during an off-cluster backup | PASS: re-queued after ~120 s, completed encrypted at 188 s, one interruption recorded |
| PostgreSQL pod deleted under load | PASS after a fix: first run answered 401 (clients would sign out) and hung requests; now 503 with `Retry-After`, bounded requests, automatic reconnect |

Concurrency (idempotency keys, operation claims, document and audit visibility) is also covered by tests that run against a
real PostgreSQL. See [POSTGRES.md](POSTGRES.md) and [HA.md](HA.md).

## Not validated

- **Live migration**, on any hardware (single-node lab).
- **Node failure and control-plane failover time**, with one replica or two. [HA.md](HA.md) describes the supported
  topologies; no node-loss run has been recorded, so there is no RTO figure. Leader failover with an operation in flight and
  a PostgreSQL pod restart were measured (above); failover of a replicated PostgreSQL and load on a shared database were not.
  The lab has one node, so two replicas share a node.
- **Windows guests**, including virtio drivers and the guest agent.
- **Guest agent paths not yet exercised**: the drill's `guest_probe` (the drill guest used the serial console), real
  database-flush hooks (none installed in the lab guest), Windows and RPM-based guests
  (see [GUEST_AGENT.md](GUEST_AGENT.md)).
- **VMware import** against a real vCenter (and the Migration Cockpit's NIC mapping against
  real Multus networks). Unit/mock tests + lab `mode=stub` Job orchestration exercised 2026-11-13 on `80.79.5.173` (Job Complete, `vm_created: false`); not a real vCenter. Experimental.
- **GPU / SR-IOV passthrough on real devices.** Inventory/preflight/permit and placement-plan live `preflight_issues` checked 2026-11-13 on a lab with neither ([DEVICES.md](DEVICES.md), `scripts/lab-fake-gpu-device.sh`). Experimental.
- **Multi-cluster fleet inventory** against real remote clusters. Lab dual-enrollment of the same API (`scripts/lab-fleet-dual-enroll.sh`) on `80.79.5.173` 2026-11-13 returned two healthy enrollments (`local` + `lab-alias`). Experimental.
- **Cross-cluster / site DR.** `/cross-cluster-dr/run` is a same-cluster off-cluster backup→restore rehearsal only (lab 2026-11-13: plan OK; `/run` correctly 409/disabled without S3 target).
- **Block-mode backup beyond one run**: a 1 GiB RBD Block volume holding 64 MiB of random data was backed up and restored with identical SHA-256 (2026-10-01); restoring into Block volumes was also verified (same SHA-256 into a Block volume and, with `volume_mode: Filesystem`, into a file, 2026-10-02); larger or real guest disks are not covered. **CSI drivers beyond RBD and the hostpath
  driver (no networked/replicated driver other than Ceph)**, **other
  Kubernetes/KubeVirt/CDI versions**, upgrades between Zorvia versions, site-level DR.

## Problems the runs found (and where they stand)

| Finding | Status |
|---|---|
| `POST /vms` waited up to 120 s for the guest but the router cut requests at 30 s, answering 408 and cancelling the rest of the handler | Fixed (wait bounded to 25 s; poll `wait-ready`) |
| Re-enrolling TOTP left 2FA off until the new code was verified | Fixed (pending secret) |
| Deploy script's `kubectl apply` resets `ATLAS_URL`; a stuck `Terminating` API pod keeps the scheduler lease | Operational pitfall; re-set the variable and force-delete the stuck pod |
| On the RBD class CDI chooses Block volume mode and the importer cannot open the device (`Permission denied`) | Workaround: create DataVolumes with `volumeMode: Filesystem`. Not fixed in Zorvia |
| Backup agent rejected Block-mode source volumes | Fixed: raw-device read, restored as `disk.img`. The live run also found that the agent's own path check refused the device node, which unit tests had missed |
| Restored VMs' PVCs outlive the VM | Expected Kubernetes behaviour; delete them yourself |
| `guest_agent: zyvor` guests could not be snapshotted online with PVC disks: the agent's `fsfreeze` is not permitted | Fixed in Zorvia (cloud-init drop-in); existing VMs need the drop-in. Upstream: GuestKit's QGA freeze handlers do not use its privileged helper |
| The recovery drill of a guest with an ephemeral root disk cannot see the agent | Known limit; the drill falls back to the serial console |
| Deleting the PostgreSQL pod made the API answer 401 and hang | Fixed: 503 `AUTH_STORE_UNAVAILABLE`, request timeouts, connect timeout and keepalives |
| The build host's disk filled up (incremental build cache) | Operational: build with `CARGO_INCREMENTAL=0` |

## Reproduce

```bash
ZORVIA_LAB_HOST=<host> ZORVIA_E2E_PASSWORD=... \
E2E_STORAGE_CLASS=<snapshot-capable class> E2E_BACKUP=1 \
KUBECTL="ssh root@<host> KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl" \
tests/e2e/guest.sh
```

`E2E_BACKUP=1` needs the off-cluster target configured on the deployment
([OFFCLUSTER_BACKUP.md](OFFCLUSTER_BACKUP.md)). A scenario that cannot run reports SKIP with the
reason; it never passes silently.
