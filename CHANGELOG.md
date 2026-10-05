# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Golden Images page** (`/app/golden-images`): build Windows golden images through Kryton (dockur install, Sysprep,
  qcow2 capture, CDI bootstrap, guestkit passport) next to Zorvia's own Linux imports and VM captures.
- Kryton golden-build proxy: `GET|POST /api/v1/kryton/golden`, `GET /api/v1/kryton/golden/{id}`,
  `GET /api/v1/kryton/golden/{id}/passport`, `POST /api/v1/kryton/golden/{id}/bootstrap`. Writes need `vm.create`,
  like `POST /api/images/from-vm`.

## [0.4.0] - 2026-10-03

The production-platform release: what ships here is the work behind [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) (measured
results on one reference stack, and what is not yet validated) and [docs/FEATURE_MATURITY.md](docs/FEATURE_MATURITY.md).

**Highlights**

- **Data protection you can prove:** encrypted off-cluster S3 backups with read-back verification, restore (including Block volumes),
  recovery drills, durable operations that survive restarts.
- **Identity and tenancy:** namespace allow-lists for users and API tokens, TOTP secrets sealed at rest (local key or Vault Transit),
  OIDC group-to-role mapping, certificate hot reload, a security audit's fixes.
- **High availability (Beta):** PostgreSQL store (users, operations, audit, schedules, shared lockout) and a two-replica Helm profile
  (`values-ha.yaml`).
- **Guests:** Zyvor guest agent (GuestKit v1.2.5) with application-consistent snapshots, read-only guest views, a Guest agent tab.
- **Hardware (Experimental):** GPU/vGPU/SR-IOV passthrough, NUMA/hugepages/dedicated CPUs, permit/remove devices from the console.
- **Console:** passthrough devices page, guest agent tab, sign-in page explains how to get the admin password.

**Upgrade notes**

- **CLI is restructured into nested subcommands** (breaking): `zorvia create` is now `zorvia vm create`, and so on. `zorvia --help` and
  `zorvia commands` show the current syntax.
- **Lab mode is no longer set by the shipped manifests.** A lab deployment that relied on the default admin password or shared API
  key must set `ZORVIA_LAB_MODE=1` explicitly (or create real credentials with `scripts/create-auth-secret.sh`). `ZORVIA_JWT_SECRET`
  must be at least 32 bytes outside lab mode.
- **Read access is tighter:** Viewers can no longer read audit logs, webhooks or VM logs; deleting backups and snapshots needs `vm.delete`.
- **Pods and sockets:** `/ws/console` and `/ws/vnc` need `vm.power`, `/ws/ssh` and pod sockets need `cluster.admin`.
- **SQLite remains the default store.** PostgreSQL is opt-in (`ZORVIA_DATABASE_URL`, optional one-time `ZORVIA_DATABASE_IMPORT=1`); the
  chart refuses `replicaCount > 1` without it. Existing Zyvor-agent VMs created before this release keep their old agent
  (see docs/GUEST_AGENT.md).
- The Helm chart gains `devices.managePermitted`, `ha.acceptLocalState` and `database.*` values; the service account may now `list`
  network-attachment-definitions.

### Added (passthrough devices, complete)

- Permit/remove passthrough devices in the KubeVirt CR from Zorvia (`POST`/`DELETE /api/v1/devices/permitted`, cluster.admin, audited;
  Helm `devices.managePermitted` adds the `patch` right on `kubevirts`; optimistic-concurrency merge patch). NUMA passthrough
  (`cpu_numa_passthrough`), hugepages and dedicated-CPU placement rules and node checks (static CPU manager label, free hugepages),
  SR-IOV pool inventory with free VFs, device-kind hints, a **Passthrough devices** console page and NUMA/hugepage options on
  Create VM. Checked against a real KubeVirt API (server-side dry runs and live permit/remove); still not run on a GPU or SR-IOV NIC.
  See docs/DEVICES.md.

### Changed

- The pinned Zyvor guest agent is GuestKit **v1.2.5**, which freezes filesystems through its privileged helper; the `CAP_SYS_ADMIN` drop-in is now only written for a mirrored or older package (`ZORVIA_GUEST_AGENT_URL`).

### Fixed

- With PostgreSQL the failed-login lockout is shared by all replicas (`login_failures`); before, each replica counted separately, so N replicas allowed N times the attempts.

- While PostgreSQL is unreachable the API answered 401 (clients sign the user out) and requests could hang for minutes. It now answers
  503 `AUTH_STORE_UNAVAILABLE` with `Retry-After`, bounds every database request (`ZORVIA_DATABASE_TIMEOUT_SECS`, default 8 s), and
  connects with a 5 s timeout and keepalives.

- `guest_agent: zyvor` guests could not be snapshotted online when they had PVC-backed disks: the packaged agent unit has no
  capabilities, so its `fsfreeze` failed and KubeVirt failed the snapshot. Cloud-init now adds a systemd drop-in granting
  `CAP_SYS_ADMIN` (ambient) to the agent. Existing Zyvor-agent VMs need the drop-in (docs/GUEST_AGENT.md).

### Added (GuestKit integration)

- `GET /api/vms/{name}/guest/agent` and `/guest/inventory/{packages|users|certificates|containers|security}`: read-only
  views from inside the guest through the Zyvor agent (JSON-RPC over the QGA channel via `virsh` in the launcher pod; fixed
  read-only method set). Snapshots and off-cluster backups run the guest's pre-snapshot hooks and record `guest_quiesce`
  (application / filesystem / crash); recovery drills add `guest_probe`. See docs/GUEST_AGENT.md.

### Added (PostgreSQL backend, phase 4: shared documents and two-replica chart)

- Schedules, warm pools, alerts, webhooks and migration history are stored in PostgreSQL when configured (files remain the fallback
  and the default). The chart accepts `replicaCount > 1` with a database, leader election and `ha.acceptLocalState: true`
  (`values-ha.yaml`: emptyDir `/data`, rolling updates, PodDisruptionBudget, anti-affinity); without them it refuses.
  Commercial-offerings records and the JSONL audit sidecar stay per replica.

### Added (PostgreSQL backend, phase 3: audit trail)

- The audit trail is written to and read from PostgreSQL (`ZORVIA_DATABASE_URL`), so every replica shows one trail; one-time import of an
  existing `audit.db` with `ZORVIA_DATABASE_IMPORT=1`. Failed audit writes are now logged instead of silently dropped.

### Added (PostgreSQL backend, phase 2: operations)

- The durable operations queue also runs on PostgreSQL (`ZORVIA_DATABASE_URL`). Idempotency keys are claimed by the insert itself, so
  racing replicas create one operation; start-up recovery on a shared store skips operations with a live heartbeat.

### Added (PostgreSQL backend, phase 1)

- Users, tokens and session revocations can live in PostgreSQL (`ZORVIA_DATABASE_URL`, Helm `database.*`) instead of SQLite, through
  a small SQL layer that runs the same queries on both. One-time import of an existing `auth.db` (`ZORVIA_DATABASE_IMPORT=1`).
  Cross-replica race tests (token revocation, TOTP replay) run against a real Postgres. Audit and JSON
  state are still per replica, so active/active is not complete yet. See docs/POSTGRES.md.

### Added (GPU / SR-IOV passthrough)

- `devices` on `POST /api/vms` (GPUs and host devices emitted as KubeVirt `gpus`/`hostDevices`), a preflight that refuses
  VMs the cluster cannot schedule (422 with reasons), `GET /api/v1/devices` inventory and `POST /api/v1/devices/preflight`,
  an SR-IOV network check, a Create VM section, and read access to the KubeVirt CR in the RBAC. Experimental: not run on
  real GPU/SR-IOV hardware. See docs/DEVICES.md.

### Added (key provider)

- Optional encryption of TOTP secrets at rest: local AES-256-GCM key (`ZORVIA_KEY_FILE`/`ZORVIA_KEY_HEX`, with rotation) or HashiCorp
  Vault Transit (`ZORVIA_VAULT_*`); existing secrets are sealed at startup; values are bound to their user. New
  `DELETE /api/v1/users/{id}/totp` (users.admin) resets a user's 2FA (lost authenticator or lost key). See
  docs/KEY_PROVIDER.md.

### Added (enterprise identity)

- OIDC group-to-role (and namespace) mapping: `ZORVIA_OIDC_GROUP_ROLES`, `ZORVIA_OIDC_GROUPS_CLAIM`,
  `ZORVIA_OIDC_DEFAULT_ROLE`; applied at every sign-in, revokes sessions on change, denies unmatched users by
  default. See docs/OIDC.md.

### Added (TLS rotation)

- The API server hot-reloads its TLS certificate and key when the files change (`ZORVIA_TLS_RELOAD_SECS`, default
  60; a failed reload keeps the previous certificate). See docs/TLS_ROTATION.md.

### Changed

- `s3-immutable-backup` (off-cluster backup, restore, drills, key check) is now **Beta**: verified end to end on two CSI
  drivers (Rook-Ceph RBD incl. Block volumes, and the hostpath driver). Limits are listed in
  docs/FEATURE_MATURITY.md and docs/SUPPORT_MATRIX.md.

### Added (Block restore)

- Off-cluster restore into Block volumes: the manifest records `source_block`, restores default to the source's
  volume mode, and `volume_mode` (`Block`/`Filesystem`) on the restore request overrides it. See
  docs/OFFCLUSTER_BACKUP.md.

### Added (guest agent)

- `guest_agent` on `POST /api/vms` (`zyvor` | `qemu` | `none`, default `ZORVIA_GUEST_AGENT`) installs
  GuestKit's Zyvor guest agent (checksum-verified, pinned release, mirrorable) or qemu-guest-agent through
  cloud-init; Create VM page selector. See [docs/GUEST_AGENT.md](docs/GUEST_AGENT.md).

### Added (backup key check)

- `POST /api/backups/offcluster/{id}/verify-key`: proves the configured encryption key still
  decrypts a stored backup (first part of each disk) without a restore. See
  [docs/OFFCLUSTER_BACKUP.md](docs/OFFCLUSTER_BACKUP.md).

### Added (maintenance)

- `GET /api/v1/maintenance/plan?node=` (cluster.admin): read-only pre-drain plan per VM (live-migrate
  destination, downtime, no destination), from KubeVirt's migratability, RWO volumes, pinned
  devices and destination capacity. See [docs/MAINTENANCE.md](docs/MAINTENANCE.md).

### Added (validation)

- `tests/e2e/guest.sh`: real-guest E2E (boot, snapshot, data snapshot on CSI storage,
  off-cluster backup/restore/recovery drill, live migration when two nodes exist, API
  restart) and [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) with the first measured
  results and an explicit list of what is not validated.
- Chart refuses `replicaCount > 1` on a ReadWriteOnce volume; [docs/HA.md](docs/HA.md).

### Added (Block-mode backup)

- Off-cluster backup accepts `volumeMode: Block` source volumes (raw-device read; restore
  writes a `disk.img` file). Verified on Ceph RBD with matching SHA-256.

### Added (migration cutover guards)

- Admin VMware Imports console: readiness checks, explicit target networking,
  wave progress, boot policy and cancellation.
- Same-namespace Multus/pod NIC mappings with validation and optimistic
  resourceVersion checks before cutover; optional guest-agent readiness.
- Import preflight blocks on unreadable resources and existing root
  PVCs/DataVolumes, and rechecks names at execution time.
- Import Jobs always deploy stopped guests. Cancellation and failure delete
  only the exact Job UID, preserving VM disks for operator inspection. This
  replaces automatic name-based VM/PVC cleanup; see `docs/VM_IMPORT.md`.
- Scheduler election fails closed, retries initialization, honors foreign lease
  duration, bounds API waits and expires local authority with a monotonic
  deadline; see `docs/SCHEDULER_LEADERSHIP.md`.
- Small Clippy cleanups for the current stable toolchain, without changing
  tenancy, WebSocket permissions or backup HTTP response behavior.

### Added (live fleet inventory)

- Experimental live fleet inventory and administrator console at `/app/fleet`:
  explicit remote kubeconfig/context/namespace enrollment, paginated read-only
  nodes/VM collection, bounded probes, shared refresh cache, partial totals,
  unknown counts, stale-response warnings, and inventory search. Fleet reads
  require `cluster.admin`; credentials and remote errors are never exposed.
  See `docs/FLEET.md` for enrollment, minimal RBAC, and live-validation limits.

### Security

- API tokens can be restricted to a namespace allow-list at creation (`namespaces`), enforced like
  user restrictions; see docs/TENANCY.md.

- Bootstrap admin password goes to a 0600 file instead of the log; OIDC JWKS cache with
  refresh on unknown `kid`, `azp`/`nbf`/`iat` checks.

- TOTP replay protection, no disabled-account/username enumeration by response or timing, and
  fail-closed API-token expiry (PR `fix/audit-low`).

Findings from an identity and tenancy audit (all server-side; PRs #138-#141):

- **VM sockets are permissioned.** `/ws/console` and `/ws/vnc` require `vm.power`,
  `/ws/ssh` requires `cluster.admin` (a Viewer could previously open all three).
  VM names are validated, cross-origin WebSocket handshakes are refused, and
  sessions are audited.
- **MFA.** Re-enrolling TOTP while 2FA is on needs the password and a current code and
  revokes sessions. New `POST /api/v1/auth/password` and `POST /api/v1/auth/logout`.
- **Hardening.** Failed-login lockout (8 / 15 min per username); `ZORVIA_JWT_SECRET`
  must be 32+ bytes; lab mode removed from the shipped manifests.
- **Least privilege reads.** Audit logs/stats, webhooks, the Atlas audit export and VM
  logs are no longer readable by Viewers; deleting backups/snapshots needs `vm.delete`.
- **Tenancy.** Per-user namespace allow-lists (`PUT /api/v1/users/{id}/namespaces`),
  enforced on every request, failing closed — see [docs/TENANCY.md](docs/TENANCY.md).
- **OIDC.** State bound to the browser by cookie (login CSRF), session token returned
  in the URL fragment, and strict `kid` handling.
- **PAM sessions** can be revoked via logout.

### Added (durable operations, backup, import)

- **Durable operations** (SQLite, leader-elected reconciler, retries that survive
  restarts) with admin `GET /api/operations` and cancel.
- **Verified off-cluster backup** to S3 (including an Atlas-provisioned bucket):
  per-part AES-256-GCM, read-back verification, Object Lock, restore, recovery drills,
  retention — [docs/OFFCLUSTER_BACKUP.md](docs/OFFCLUSTER_BACKUP.md).
- **VMware import** through h2kvm Jobs with waves, preflight and rollback —
  [docs/VM_IMPORT.md](docs/VM_IMPORT.md).
- Refreshed image catalog (only images that exist on `quay.io/containerdisks`).

### Added

- **`zorvia adopt`** — read-only report on the VMs Zorvia can see and which
  GA/Beta capabilities apply, with `-A` and `-o table|json|yaml`; also
  `GET /api/v1/adopt/report`. Shared logic in `src/adopt`.
- **Rescue mode** — the Rescue tab's frontend (built but never wired) now
  has a real backend: `POST /api/vms/:name/rescue` (set-hostname,
  inject-ssh-key, enable-ssh) creates a privileged Kubernetes Job that
  mounts the VM's own PVC and runs a new `rescue-agent` binary against it
  via [GuestKit](https://github.com/zyvorai/guestkit) (`Guestfs::set_hostname`/
  `set_ssh_authorized_keys`, and the same systemd-enable symlink trick
  GuestKit's own CLI uses). `GET`/`DELETE /api/vms/:name/rescue/:job_name`
  poll/clean up the job. `cluster.admin`-gated end to end, same as the Pods
  page — see [docs/RESCUE.md](docs/RESCUE.md). Reset-password,
  install-packages, and disk-inspect stay on the tab marked "Not yet
  available" rather than calling a route that doesn't exist — each needs
  its own follow-up (Linux has no direct password mutator in GuestKit;
  package install needs guest network egress from inside a privileged Job;
  inspect needs its own API research).
  `guestkit` is pinned to a `git` revision of `zyvorai/guestkit`, not a
  crates.io version — the published `guestkit` crate is stale, from a
  different repo, and LGPL-3.0-or-later, not the Apache-2.0 source this is
  pinned to — and is only pulled in by the new opt-in `rescue-agent` Cargo
  feature/binary, never the main server's dependency graph. New
  `deploy/rescue-agent.Dockerfile` + a best-effort CI job build and push
  its container image independently of the main release.

- **Atlas DataBridge (cloud-to-edge DB migration)** — the final piece of the
  Atlas storage integration plan. `GET|POST /api/v1/atlas/databridge/
  sources`, `GET|DELETE .../sources/:id`, `POST .../sources/:id/discover`,
  `GET|POST .../plans`, `GET|DELETE .../plans/:id`, `POST .../plans/:id/
  {assess,provision,full-load,validate,cutover,rollback}`, `POST .../
  plans/:id/cdc/{start,stop,restart}`, `GET .../edge-clusters[/:id]`,
  `DELETE .../edge-clusters/:id`, `GET .../cdc-streams[/:id]`, `GET|POST
  .../object`, `GET|DELETE .../object/:id` (delete returns `204`), `POST
  .../object/:id/start`, `GET .../validations[?plan_id=]`, `GET .../
  cutovers`. New top-level page (`/app/databridge`,
  `web/src/pages/DataBridge.tsx`) rather than a Storage-page section — a
  genuinely different product domain from VM/storage provisioning, per the
  plan. Covers the full staged pipeline (source → discover → plan →
  assess → provision → full-load → cdc → validate → cutover → rollback)
  plus the independent object-store (S3→RGW) migration leg.
  `cutover`/`rollback` are guarded on Atlas's own side (validated + passed
  validation + CDC lag under 10s; existing cutover + open rollback
  window) — Zorvia relays Atlas's specific errors rather than duplicating
  the checks, and the UI's confirm dialogs describe the preconditions up
  front. Verified live end-to-end against a real `atlas-gateway`: a full
  plan pipeline through a real rejected-cutover-before-validation `409`,
  a real rejected-cutover-while-CDC-lag-too-high `409` (confirming the
  fake driver's lag decays via its reconciler over real wall-clock time,
  not instantly), successful cutover once lag drained, and rollback. Also
  found and documented: source delete is refused (`409`, naming the
  referencing plan) while a plan still exists, but plan/edge-cluster/
  object-migration delete is unconditional — a repeat delete of an
  already-deleted id still returns success, same quirk as `DELETE
  /dr/peers/:id`.

- **Atlas governance** — `GET /api/v1/atlas/tenants`, `GET /api/v1/atlas/policies`,
  `GET .../tenants/:id/policies`, `PUT|DELETE .../tenants/:id/policies/:intent`,
  `GET|PUT .../tenants/:id/quota`, `POST .../volumes/:id/schedule`,
  `GET .../schedules`, `DELETE .../schedules/:id`,
  `GET|PUT .../volumes/:id/labels`, `GET .../volumes/:id/bindings`.
  New `web/src/pages/storage/AtlasGovernanceSection.tsx` covers tenant
  quota editing and schedule create/list/delete — the day-2 operations an
  operator actually reaches for; tenant policy overrides and volume
  labels/bindings are proxied but deliberately have no UI yet.
  **Deliberately excludes Atlas's own console authentication** (`/auth/
  login`, `/auth/users*`, `/auth/tokens*`) from proxying entirely — those
  manage Atlas's own accounts, not Zorvia's, and this is a permanent
  security-boundary exclusion, not a "not yet". Verified live against a
  real `atlas-gateway`: tenant quota get/put, policy override put/list/
  delete (with a real 404 on deleting an already-removed override),
  schedule create/list/delete (with a real 404 on double-delete), and
  volume labels get/put all round-trip correctly.

- **Atlas observability** — `GET /api/v1/atlas/metrics/{summary,ceph,history,forecast}`,
  `GET .../alerts[?state=]`, `POST .../alerts/evaluate`,
  `POST .../alerts/:id/{ack,silence,resolve}`, `GET .../audit`,
  `GET .../audit.csv` (raw CSV passthrough), `GET .../chargeback`,
  `GET .../policy-drift`, `GET .../events`. Atlas alerts now surface on
  Zorvia's existing `/app/alerts` page as an additional source (tagged
  "Atlas", with ack/silence/resolve actions) instead of a second
  disconnected alerts UI — Zorvia already has a real alerts feature.
  Audit/chargeback/policy-drift/metrics get simple read-only views scoped
  to the Atlas area of the Storage page (new
  `web/src/pages/storage/AtlasObservabilitySection.tsx`) rather than being
  merged into Zorvia's own audit-trail or cost/FinOps surfaces. Verified
  live against a real `atlas-gateway`: the full alert lifecycle
  (evaluate → ack → silence → resolve, with a real audit trail entry per
  action and a real 404 on re-resolving an already-resolved alert), all
  four metrics endpoints, and audit-CSV/chargeback/policy-drift/events
  round-trip correctly.

- **Atlas AI-assisted insights** — `POST /api/v1/atlas/ai/advisor`,
  `GET .../ai/anomalies`, `GET .../ai/incidents`, `POST .../ai/what-if`.
  Compute-only despite the `POST` verbs on advisor/what-if — every response
  carries `can_execute: false`; the local deterministic advisor is always
  available, an optional external provider (configured on Atlas's side)
  can only rewrite the executive-summary text, never the risk score or
  evidence. New `web/src/pages/storage/AtlasAiSection.tsx` card: ask-the-
  advisor with a free-text question, an anomalies/correlated-incidents
  read view, and a what-if capacity projector — always passes `mode:
  "local"` so loading the Storage page never triggers a real external-
  network call on Atlas's behalf. Verified live against a real
  `atlas-gateway`: advisor returns real evidence (capacity/alerts/failed-
  jobs), anomalies/incidents return the documented shape, and what-if
  projects a capacity/risk delta without mutating anything.

- **Atlas disaster recovery (RBD mirroring)** — `GET|POST /api/v1/atlas/dr/peers`,
  `DELETE .../dr/peers/:id`, `GET .../dr/mirrors`, `GET .../dr/status`,
  `GET .../dr/preflight`, `POST .../dr/mirrors/:id/{promote,demote}`,
  `POST .../dr/mirrors/:id/rpo`, `POST .../dr/failover`, and
  `POST|DELETE .../volumes/:id/mirror`. Atlas's own source labels this
  "scaffolding — real ops UNVERIFIED without a 2nd cluster"; `promote`/
  `demote`/`failover` require Zorvia's strictest `cluster.admin` permission
  (checked before the general `storage.admin` rule covering the rest of
  Atlas), stricter defense-in-depth beyond Atlas's own token-role check
  since these routes can flip which cluster is primary. New
  `web/src/pages/storage/AtlasDrSection.tsx` shows a persistent (non-
  dismissable) warning quoting that caveat plus this deployment's real
  `dataplane_verified` state, and gates promote/demote/failover behind the
  same confirm-dialog pattern used elsewhere, naming preflight blockers in
  the confirmation text when preflight isn't `ready`. Verified live against
  a real `atlas-gateway`: peer register/delete, mirror enable/disable, the
  promote/demote role-transition guards (409 on an invalid transition),
  `rpo` recording, and `failover`'s `confirm=true` requirement (400
  without it) all round-trip correctly — this exercises the control-plane
  catalog and RBAC, not a live two-site `rbd mirror` data-plane drill,
  which needs a second real Ceph cluster neither this repo nor Atlas's own
  dev instance has. Also found and documented: `DELETE /dr/peers/:id` is
  unconditional on Atlas's side (no rows-affected check) and always
  returns `{"deleted":true}`, even for a nonexistent id, unlike backend/
  bucket/backup delete which 404.

- **Atlas backend/RBD/bucket UI** — the Storage page's Atlas section now
  exposes the backend, RBD, and object-store bucket lifecycle proxied in
  the three prior entries below: backend create/discover/cordon/uncordon/
  delete inline in the main Atlas card, plus two new sibling cards
  (`web/src/pages/storage/AtlasRbdSection.tsx`,
  `AtlasBucketsSection.tsx`) for RBD image create/resize/delete (grouped
  by pool) and bucket create/delete. Split the growing inline Atlas block
  out of `RookStorage.tsx` into `web/src/pages/storage/AtlasSection.tsx`
  as part of this change rather than letting one file keep growing.
  Deliberately out of scope for the UI: RBD clone/migrate/flatten/QoS/
  snapshots, bucket object-level operations, and backup/restore creation
  — the backend routes exist and are proxied, just not this pass's UI.

- **Atlas object-store buckets + backups/restores** — bucket CRUD, object
  list/delete/upload-url/download-url/prune (presigned S3 URLs — bytes
  never transit through Zorvia or Atlas), `POST /api/v1/atlas/backup-jobs`,
  `POST /api/v1/atlas/restore-jobs`, `GET|DELETE /api/v1/atlas/backups[/:id]`.
  This completes Phase 2 of the Atlas storage integration (backend
  lifecycle, RBD images, object-store — everything except frontend UI for
  the newer pieces). Verified live: create returns Atlas's real job
  envelope, reads work, and 404s on a nonexistent bucket/volume/backup are
  real Atlas errors, not client-side guesses. Bucket/backup *creation*
  itself needs a real Kubernetes cluster attached to Atlas to fully
  succeed (same constraint already documented for volume creation) — not
  exercisable end-to-end against the no-kubeconfig local dev instance used
  for these tests, so the happy path is verified by contract (request/
  response shapes, job envelope, auth) rather than a completed job.

- **Atlas RBD image ops** — `GET|POST /api/v1/atlas/rbd-images[?pool=]`,
  `DELETE .../{pool}/{image}`, `clone`/`resize`/`migrate`/`flatten`/`qos`,
  snapshot create/list/rollback/delete, and `POST
  /api/v1/atlas/rbd-usage/refresh`. RBD images are a separate identity space
  (`rbd:<pool>/<image>`) from the `StorageVolume` abstraction the volume
  routes use. Verified live against a real `atlas-gateway`: full lifecycle
  (create → resize → qos → snapshot → rollback → delete) and clone+flatten
  both round-trip correctly.

- **Atlas backend lifecycle + OSD ops** — `POST/DELETE /api/v1/atlas/backends`,
  discover/cordon/uncordon, `GET|POST /api/v1/atlas/maintenance` (pause/resume
  Atlas's job engine), `GET /api/v1/atlas/maintenance/orphans`,
  `GET /api/v1/atlas/upgrade/preflight`, `GET /api/v1/atlas/osds` +
  `out`/`in`/`reweight`. Unlike the volume writes, backend lifecycle is
  synchronous on Atlas's side (no job envelope); OSD ops are async jobs, same
  shape as volumes. Verified live against a real `atlas-gateway`: backend
  create → cordon → uncordon → delete, maintenance pause/resume, and OSD
  out/in all round-trip correctly.

- **Atlas job status polling** — `GET /api/v1/atlas/jobs[/:id]`,
  `POST /api/v1/atlas/jobs/:id/cancel`. Every Atlas write (volume
  create/expand/delete) returns a `202` + job id that Zorvia previously
  never checked again; this closes that gap end to end. Verified live:
  cancelling an already-terminal job correctly returns Atlas's real `409`.
  The Storage page's Atlas section now polls the returned job to a
  terminal state after every create/expand/delete and toasts the real
  outcome (not just "requested"), and shows a "Recent jobs" list with a
  Cancel action on non-terminal jobs.

- **Atlas storage integration (optional)** — `ATLAS_URL`/`ATLAS_TOKEN` wires
  Zorvia to Atlas, the Zyvor-suite storage control plane, mirroring the
  existing Kryton integration pattern. Read-only inventory (backends,
  clusters, pools, Ceph status/capacity, storage classes, volumes) plus
  volume create/expand/delete (`owner: {product: "zorvia", ...}` tags a
  created volume so it's traceable back to a Zorvia VM in Atlas's own
  inventory; delete always passes `confirm=true` and lets Atlas decide
  whether that was actually required). New `src/atlas/` client module,
  `/api/v1/atlas/*` routes (always registered, `503 ATLAS_DISABLED` when
  unconfigured), an Atlas section with inline create/expand/delete controls
  on the existing Storage page (`/app/storage`) alongside Rook-Ceph, an
  `Atlas` platform-status line, and
  [docs/ATLAS_INTEGRATION.md](docs/ATLAS_INTEGRATION.md). Absent by default —
  nothing changes unless `ATLAS_URL` is set. Verified live against a real
  running `atlas-gateway` (dev config, fake Ceph driver), not just code
  review — all volume-mutating routes are genuinely async (`202` + a job
  envelope, not a finished result).

### Changed

- **Docs: README is a landing page.** The detail moved into `docs/` (getting-started, whats-inside, console-and-api, profiles-blueprints-templates, day-2-ops, operator-toolkit, platform-surface, config-and-library, develop, roadmap); hero and badges restyled with a light/dark share card.
- **CLI restructured into Cilium/kubectl-style nested subcommands, with colored `--help`** —
  **breaking**: the ~180 previously-flat top-level commands (`zorvia create`,
  `zorvia migrate`, `zorvia ha-config`, ...) now live under 25 category
  groups (`zorvia vm create`, `zorvia migration migrate`,
  `zorvia ha ha-config`, ...): `vm`, `template`, `profile`, `blueprint`,
  `advisor`, `snapshot`, `monitor`, `disk`, `network`, `migration`, `ha`,
  `backup`, `security`, `cost`, `automation`, `observability`, `tenancy`,
  `inventory`, `maintenance`, `placement`, `dev`, `change`, `guest`, `api`,
  `config` — plus `init`, `info`, and `commands` staying top-level, same as
  `git init`/`git --version`. `zorvia --help` now shows every group with a
  one-line description instead of one undifferentiated ~180-entry list, via
  a real `clap::builder::Styles` (colored headers/usage/literals/
  placeholders, matching the palette `tui::colors::cli` already uses
  elsewhere) rather than a hand-styled wrapper. `zorvia commands` is
  regenerated to show the same grouping with the new two-word invocations.
  Every flag, positional argument, and handler behavior is unchanged —
  only the subcommand path changed. Shell completions (`zorvia completions`)
  pick up the new structure automatically since they're generated from
  `Cli::command()`, not hand-maintained. Builds on the color-crate
  consolidation (`colored` as the one CLI/TUI color crate, `miette` for
  top-level error rendering) shipped just before this.
  **Docs and example scripts referencing the old flat commands are being
  updated in a follow-up pass** — `--help`/`zorvia commands` are the
  authoritative source of the current syntax in the meantime.

## [0.3.4] - 2026-09-27

### Added

- **Golden image capture from a running VM** — `POST /api/images/from-vm/:vm_name` clones a VM's PVC/DataVolume-backed disk into a standalone CDI DataVolume (`GET /api/images/convert/:id` polls the job). The Create VM "save as golden image" flow previously called routes that were never implemented server-side and silently 404'd. VMs with only an ephemeral (blank/containerdisk) disk now get a clear `400 NO_PERSISTENT_DISK` instead.

### Fixed

- **CDI PVC-clone missing `accessModes`** — `data_volume_clone_manifest()` (shared by the new golden-image route and the existing Clone VM feature) didn't set an explicit `accessModes` on the clone DataVolume's storage spec, so cloning a PVC-backed VM's disk failed with `ErrClaimNotValid` on any StorageClass without a StorageProfile default (e.g. k3s's built-in `local-path`) — the same fix already applied to the cloud-image-download path.

### Changed

- **README rewrite** — tighter marketing pitch (five-verb hook: create/operate/reach/govern/prove), the 24-row "Why Zorvia" table regrouped into six scannable highlight clusters under a new "What's inside" heading, a live demo GIF added to the Console gallery, and regenerated social preview cards.

### Removed

- **Dead frontend API functions** — `getSnapshotTree`, `getAuditLog`, `getBackup`, `getBackupStats` called backend routes that never existed and were unused by any page.

### Changed

- **README + docs polish** — Netra-style hero (docs CTA, five-step “What you get”, live console gallery), Helm install + Fabric API quick table, new [docs/LAB.md](docs/LAB.md) (deploy, auth Secret, `lab-smoke.sh`), docs index refresh, audit export section in [docs/WEB_CONSOLE.md](docs/WEB_CONSOLE.md).

### Fixed

- **VM serial console input** — keystrokes never reached the guest: xterm.js sends text WebSocket frames and KubeVirt's console only reads binary ones. The proxy now forwards text as binary, so Enter brings up the `login:` prompt.
- **Disk Images prompt** — shows the real `zorvia images` command instead of a non-existent `zorvia images ls -l`.
- **Repo hygiene** — `.gitignore` covers `web/test-results/`, `web/playwright-report/` and `.cursor/*.log`.
- **Top-nav admin gating** — direct top-nav links now honor `adminOnly` (previously only mega-menu items were filtered).
- **JWT / Lease lab auth** — `jsonwebtoken` rust_crypto feature; Kubernetes Lease `MicroTime` formatted with exactly six fractional digits so leader election renew succeeds.

### Added

- **Pods RBAC self-check** — `GET /api/v1/pods/capabilities` (SelfSubjectAccessReview for `pods/log`, `pods/exec`, `pods` delete); the Pods page greys out actions whose ClusterRole rule is missing and shows a notice naming it, instead of failing on click after an upgrade.
- **Pods page extras** — **Events** and **YAML** panels (Terminal.app black; events colored by type with `×N` repeats, YAML syntax-colored with line numbers, copy/download), **Restart** (controller-owned pods only — `virt-launcher`/Job pods refused with 409) and **Delete** with confirmation, and **logs in a new tab** at `/app/pods/:ns/:name/logs`. New `DELETE /api/v1/pods/{ns}/{pod}`, `POST …/restart`, `GET …/events`, `GET …/yaml` (all `cluster.admin`, audited). RBAC adds `pods` delete.
- **`zorvia images` CLI** — lists the same disk image catalog as `GET /api/images` / `/app/disk-images` (`-o table|json|yaml`); catalog now shared via `kube::catalog::disk_image_catalog()`.
- **Terminal.app look everywhere** — VM serial console and in-browser SSH use the black Terminal.app profile (fit-to-window, copy-on-select, title-bar reconnect); Event Stream renders in a black terminal with filter/pause/clear in the title bar.
- **Colorizer unit tests** — Vitest coverage for the log colorizer and YAML tokenizer; quoted access-log request lines (`"GET /x HTTP/1.1"`) now highlight the method.
- **Pods page — Terminal.app logs & exec** — admin-only `/app/pods` (top-nav link) lists every pod across all namespaces with a namespace filter, search, status chips and 10 s refresh. Bottom panel (resizable, maximizable) opens a black macOS Terminal.app-style xterm for **live logs** (colorized levels / klog / JSON / logfmt / HTTP, container select, tail, timestamps, previous, find, pause, download) or an **interactive shell** (`auto`/`bash`/`sh`, resize, exit code, copy-on-select). New `GET /api/v1/pods`, `GET /api/v1/namespaces`, `/ws/pods/{ns}/{pod}/logs`, `/ws/pods/{ns}/{pod}/exec` — all `cluster.admin`, DNS-1123 name validation, fixed shell argv; exec start/end audited at High severity (`exec`, `view_logs` actions). RBAC adds `pods/log` get and `pods/exec` create,get. Docs: [docs/PODS.md](docs/PODS.md).
- **Disk Images terminal listing** — `/app/disk-images` renders the catalog as a black Terminal.app `ls -l` view: color-coded format / size / registry refs, `grep` filter in the title bar, keyboard-selectable rows.
- **Social / README share cards** — rebuildable HTML → PNG/JPG under [`docs/social/`](docs/social/README.md) (`zorvia-share-card.png` for README hero + docs Open Graph; `zorvia-social-card.jpg` for LinkedIn/X). Website serves `../docs/social` and sets `themeConfig.image`.
- **Positioning refresh** — tagline and about copy centered on “the control plane for KubeVirt VMs” (README, docs site hero, social cards).
- **Security hardening** — pin GitHub Actions and container base images by digest; tighten workflow `permissions`; fix sign-in XSS (`js/xss-through-dom`); bump `jsonwebtoken` 10.x + website overrides for Trivy CVEs; migrate `totp-rs` 6 Builder API.
- **Secure OIDC (Beta)** — opt-in with `ZORVIA_OIDC_ENABLED=1`: OpenID discovery, PKCE S256, authorization-code token exchange, JWKS-verified `id_token` (iss/aud/nonce/exp), JIT local user (`oidc:<sub>`), then mint a normal Zorvia JWT with `token_version` revocation. Docs: [docs/OIDC.md](docs/OIDC.md).
- **Phase 5 enterprise plan APIs** — gated by `ZORVIA_EXPERIMENTAL=1` + `ZORVIA_FEATURE_*`: S3 immutable backup, Transiva, golden pipeline, cross-cluster DR, GPU/NUMA placement, fleet inventory (`/api/v1/enterprise/...`). Dry-run plans only; admin RBAC required. Docs: [docs/PHASE5_ENTERPRISE.md](docs/PHASE5_ENTERPRISE.md).
- **Expanded GitHub CI** — Helm lint/template, ShellCheck, actionlint, Hadolint, typos, kubeconform, CodeQL, Scorecard, Trivy, Semgrep, coverage, MSRV, nightly, rustdoc, feature matrix, Docker on PRs, dependency-review, PR labeler/stale/title lint, lockfile checks, license/docs gates, security-secret scan, web/website dedicated workflows.
- **CI hardening follow-ups** — remote deploy generates random lab auth secrets (no fixed JWT/password in tree); Playwright config valid for ESLint; Scorecard/Trivy action pins fixed.
- **Feature maturity registry** — `GET /api/v1/features` and `docs/FEATURE_MATURITY.md` (GA / Beta / Experimental / Model-only). Model-only surfaces (`ai-troubleshoot`, `dr-replication`) require `ZORVIA_EXPERIMENTAL=1`.
- **Supply-chain CI** — Dependabot, CODEOWNERS, `deny.toml`, `.github/workflows/supply-chain.yml` (audit/deny/SBOM/gitleaks).
- **E2E skeletons** — Playwright console smoke (`web/e2e`) and kind/KubeVirt workflow stubs.
- **Release hardening** — GHCR image digests, optional cosign keyless sign, `SHA256SUMS` on GitHub Releases; Helm `image.digest` pinning for production.
- **Upgrade guide** — `docs/UPGRADE.md` for 0.3.2 → 0.3.3+.
- **Helm chart** (`charts/zorvia`) with `values-lab.yaml` and `values-production.yaml`: Ingress/cert-manager hooks, PDB, NetworkPolicy, topology spread, namespace-scoped RBAC mode, optional Rook bootstrap SA, Lease RBAC for scheduler leader election.
- **Persistent audit trail** — SQLite at `ZORVIA_AUDIT_DB` (default alongside auth.db) survives restarts.
- **Scheduler leader election** — Kubernetes Lease (`ZORVIA_LEADER_ELECTION=1`) so only one API replica runs backup/power/alert/warm-pool loops.

### Added (earlier)

- **Cilium-style platform status** — `zorvia status` with no VM name prints a colorful interlocking logo plus KubeVirt / CDI / Zorvia API / Snapshots / Rook Storage (`✅ OK` / `ℹ️ disabled` / `❌ errors` / `⚠️ warnings`), then Deployments/DaemonSets, containers, Cluster VMs, image versions, and an ✨ Features block (Backup, HA, migration, Kryton, …). Flags: `-o summary|json`, `--wait`, `--wait-duration`, `--interactive`. `zorvia status <vm>` remains for per-VM detail (now colorized). New module: `src/platform_status/`. `make status` and `make deploy-remote H=<host> U=sus` wrap the same commands. `make help` lists every target.

### Fixed

- **CI Clippy** — `handle_status` exceeded Clippy's argument limit after platform-status flags were added; platform status now dispatches from `lib.rs`, and the disabled-state unit test initializes `ErrorCount` in one expression so `-D warnings` stays green.

### Changed

- **Broader dependency refresh** — Rust mid-risk bumps (dialoguer 0.12, indicatif 0.18, ratatui 0.30, rustyline 18, dirs 6, bcrypt 0.17, rusqlite 0.37, tokio-tungstenite 0.26) with WS/TUI adapters; web npm (vitest 5.0.1, eslint 10.11, lucide-react 1.47, react-router 8.4); website lockfile refresh. Deferred: kube/k8s-openapi, axum 0.8, secrecy 0.10, schemars/toml 1.x, TS 7.

### Changed (follow-up)

- **Full remaining Dependabot stack** — kube 2.0 + k8s-openapi 0.26 + schemars 1 + toml 1 + secrecy 0.10 + thiserror 2 + tokio-tungstenite 0.30; website TypeScript 7; deploy `ubuntu:26.04` + `alpine/openssl:3.5.8`; MSRV **1.85**.

- **kube 4.x + axum 0.8** — kube **4.2** / k8s-openapi **0.28** (v1_32, jiff timestamps), axum **0.8** / axum-server **0.8** (route `{param}` syntax, WS Utf8Bytes); MSRV **1.89**.

- **Beta→GA / Phase 5 deepen / lab E2E** — audit JSONL export (`GET /api/audit/export`, `ZORVIA_AUDIT_JSONL`); Helm production hardening; OIDC lab bake-off doc; golden-pipeline `/run` applies CDI; `scripts/lab-smoke.sh` + Playwright live suite.

## [0.3.3] - 2026-09-22

### Security

- **Server-side RBAC** — JWT roles (`admin` / `user` / `viewer`) map to coarse permissions (`vm.read`, `vm.power`, `vm.create`, `vm.delete`, `storage.admin`, `cluster.admin`, `users.admin`) enforced in `auth_middleware`. Viewer accounts can no longer call mutating APIs.
- **OIDC disabled** — the unsafe callback that minted a local JWT without token exchange is removed. `ZORVIA_OIDC_*` is ignored; OIDC routes return 404 until a secure implementation lands.
- **Lab credential guards** — known defaults (`Admin@321`, `zorvia-lab-jwt-change-me-30152`) refuse startup unless `ZORVIA_LAB_MODE=1`. Auth Secrets are no longer committed; use `./scripts/create-auth-secret.sh`.
- **Scoped API tokens** — `POST/GET/DELETE /api/v1/api-tokens` stores SHA-256 hashes with role, scopes, and optional expiry. Shared `ZORVIA_API_KEY` is ignored outside lab mode.
- **JWT revocation** — `token_version` on users; access TTL defaults to 60 minutes (`ZORVIA_JWT_TTL_MINUTES`). Disable/delete/role/password/TOTP-disable bumps the version so outstanding JWTs fail validation.
- **TOTP disable step-up** — requires password + current TOTP code and revokes sessions.
- **Rook privilege split** — core ClusterRole no longer creates CRDs/ClusterRoles/workloads; optional `deploy/rook-bootstrap-rbac.yaml` for bootstrap only.
- **Webhook SSRF** — DNS resolution, private-IP rejection, connect pinning, redirect revalidation, optional `ZORVIA_WEBHOOK_ALLOWLIST`.

### Added

- Regression suite `tests/security_p0.rs` and Cursor rule `.cursor/rules/zorvia-pr-security.mdc`.

## [0.3.2] - 2026-09-14

### Fixed

- **Resource Optimizer no-op recommendation** — a VM already at the 1-core/1GB right-sizing floor with low usage got a `RightSize` recommendation reading "reduce from 1C/1GB to 1C/1GB" at `High` priority despite `0` potential savings. `analyze_right_sizing` now skips the recommendation once the floor leaves nothing to actually reduce (`src/cost/optimization.rs`).
- **Migration Readiness false "shared storage" pass** — the storage pre-check tested "every volume is `emptyDisk` or `containerDisk`" to decide a VM was local-storage-only; a `containerDisk` VM with a cloud-init volume (present on virtually every cloud-init VM) failed that `all()`, so it fell through to reporting "uses shared storage (PVC/DataVolume)" despite having zero PVC/DataVolume volumes. The check now tests for the actual presence of a PVC/DataVolume instead (`src/migration/assistant.rs`).
- **Silent no-op backups/snapshots** — a VM with no PVC/DataVolume-backed disks still gets a KubeVirt `VirtualMachineSnapshot` reporting `Succeeded`/`readyToUse: true`, because "succeeded" there just means the VM's spec was captured, not that any disk data was. Backups and Snapshots showed this as a plain "Completed" backup with no indication every volume was excluded. Both the Backups and Snapshots APIs now track `snapshotVolumes.excludedVolumes` from the real CRD status and return a `warning` field when nothing was actually captured; both web pages surface it as a hover-tooltipped warning icon on the affected row (`src/snapshots/`, `src/api/http_server/web/backup_handlers.rs`, `src/api/http_server.rs`, `web/src/pages/Backups.tsx`, `web/src/pages/Snapshots.tsx`). See the new caveat in [docs/SNAPSHOTS.md](docs/SNAPSHOTS.md).

- **CI unblocked** — `main` had been red for several commits across every job:
  - `cargo fmt --check` had accumulated formatting drift across 19 files; `cargo fmt --all` clears it.
  - Clippy failed on a real `unnecessary_lazy_evaluations` lint in `src/kube/mod.rs` (`get_or_insert_with(|| ...)` → `get_or_insert(...)`).
  - `npm ci` failed outright — `typescript@^7.0.2` has no supporting `@typescript-eslint` release yet (checked latest and alpha/canary; all cap at `<6.1.0`). Downgraded to `typescript@^5.9.3`. That unmasked a second problem `npm ci`'s failure had been hiding: `eslint-plugin-react-hooks`'s `recommended` preset had silently become its newer React Compiler ruleset (mostly hard errors) that this codebase's `useEffect` data-fetching patterns were never written against — `eslint.config.js` now declares the two classic hook-safety rules explicitly instead of spreading an unstable preset name, plus one genuine `no-control-regex` false positive fixed in `ansi.ts`.
  - `vitest`/`@vitest/coverage-v8` bumped 4 → 5 while in there (only other outdated deps), full suite re-verified.
  - `Dockerfile`'s builder image was pinned to `rust:1.76-slim-bookworm`, but `Cargo.lock` has been lockfile format v4 since 0.3.0 — Cargo below 1.78 can't even parse it. Every Docker build off this Dockerfile has been failing since that bump; the `release.yml` `docker` job would have failed the same way on the next tag push. Bumped to `rust:1.98-slim-bookworm` and corrected `rust-version` in `Cargo.toml` (and the README) from 1.76 to 1.78, the real minimum. Also installed `build-essential`/`pkg-config`/`libssl-dev` in the builder stage — the slim image had none of them, which `openssl-sys` and rusqlite's bundled-SQLite build both need.

### Changed

- **Inline-panel redesign for 7 "create action" dialogs** — Start Migration, Clone VM, Manage Tags, Resize Disk, Create Snapshot, Create Golden Image, and Download an OS Image each hand-rolled the same `fixed inset-0` popup-modal recipe independently. All seven now use the same bordered inline-expanding-panel pattern already shipped on Quotas/Network Policies/Alerts/Schedules — no backdrop, no overlay, mounted directly in the page's own layout. `ConfirmDialog`, the command palette, and read-only help panels are unchanged; those are a different UX category (destructive confirmations, search, reference).

## [0.3.1] - 2026-09-11

### Fixed

- **Stale-chunk error after redeploy** — a tab left open across a deploy still held the old `index.html`, whose content-hashed chunk filenames no longer exist once the new build lands; navigating to any not-yet-loaded route threw `Failed to fetch dynamically imported module` and landed on the "Section error" fallback until the user manually reloaded. `ErrorBoundary` now recognizes this error shape and reloads once per tab session (`web/src/utils/chunkReload.ts`), and `main.tsx` adds Vite's `vite:preloadError` listener for the same failure outside a render boundary.

### Changed

- README rewritten with a logo header, a table of contents, real product screenshots from a live deploy, and a new "Platform surface" section documenting the security, cost/FinOps, multi-tenancy, backup/DR, HA, automation, and observability command categories.

## [0.3.0] - 2026-09-11

### Added

- **User & role management** — `admin`/`user`/`viewer` accounts backed by a real backend: `GET/POST /api/v1/users`, `DELETE /api/v1/users/:id`, `PUT /api/v1/users/:id/role`, `PUT /api/v1/users/:id/enabled`, all admin-only via a dedicated `route_layer` middleware (`require_admin_middleware` in `src/api/http_server.rs`) rather than per-handler checks. Guards against dropping the last enabled admin (409 on delete/demote/disable). Disabled accounts are rejected at login (403). New `/app/access-control` page, admin-gated in both the nav (hidden for non-admins) and the route itself (`AdminRoute` in `web/src/App.tsx`). Replaces a previous Access Control page that called `/api/users` endpoints that never existed on the server.
- **Multi-disk / multi-NIC Create VM wizard** — the wizard previously created VMs with exactly one disk and one NIC even though the backend (`POST /api/vms`'s `disks`/`interfaces` arrays) already accepted more; the wizard now has collapsible "Additional disks" and "Additional NICs" sections to add more of either at create time.
- **RDP `.rdp` connect launcher** — VM Details and the Kryton Windows page offer a one-click download of a ready-to-open `.rdp` file (host, port, and username for Kryton machines) instead of requiring the user to copy connection details into their own RDP client by hand.
- **VM hotplug** — CPU (`POST /api/vms/:name/hotplug/cpu`), memory (`…/hotplug/memory`), disk attach/detach (`…/hotplug/disk[/:id]`), NIC attach/detach (`…/hotplug/nic[/:id]`); web console Hotplug tab. Fabric `POST /api/vms` now sets `cpu.maxSockets`/`memory.maxGuest` headroom by default (4x sockets / 2x memory) so VMs created through the wizard are hotplug-capable without extra steps.
- **Disk resize** — `POST /api/vms/:name/disks/:disk_name/resize` grows a PVC-backed disk in place; `GET /api/vms/:name/disks` lists disks with bus/source/resizable
- **Live migration, for real** — `POST /api/vms/:name/migrate` creates a `VirtualMachineInstanceMigration`; `GET /api/vms/:name/migrations`, `GET/POST /api/migrations/:id[/cancel]`; `/app/migrations` rewritten around KubeVirt's actual migration phases (Pending → Scheduling → PreparingTarget → TargetReady → Running → Succeeded/Failed) — previously real at the kube-client layer but CLI-only, with a UI mockup that never routed
- **VM creation feature parity** — `POST /api/vms` accepts the full `VMConfig` surface: CPU model, dedicated placement, isolate-emulator-thread, memory hugepages, firmware (BIOS/UEFI/secure boot), machine type, TPM/RNG, HyperV/ACPI/APIC features, multiple disks and NICs. Create VM wizard gained a collapsible Advanced Options step
- **Rook-Ceph storage** — `src/rook`: typed CephCluster/CephBlockPool/CephFilesystem/CephObjectStore client, pinned-manifest operator bootstrap, StorageClass + VolumeSnapshotClass provisioning; `/api/storage/rook/*`; `/app/storage` page
- **Kryton in the main wizard** — Create VM's Windows path now renders the live Kryton golden-image catalog and creates through `POST /api/v1/kryton/machines`, instead of requiring a separate flow; see [docs/KRYTON_INTEGRATION.md](docs/KRYTON_INTEGRATION.md)
- **Live metrics graph** — metrics API returns a 30-point `history` ring; `ZORVIA_PROM_SAMPLES` overlays Prometheus text; console shows source
- **Terraform module** — `terraform/modules/zorvia_vm` create/destroy via the Fabric API
- **Guest ready wait** — create+start waits for Ready+IP; `zorvia wait-ready`; `POST /api/vms/:name/wait-ready`
- **Prometheus** — `/api/metrics` exposition, virt-launcher text parser, `deploy/servicemonitor.yaml`; readiness uses `/api/readyz` (kube reachable)
- **Guest-agent metrics** — `GET /api/vms/:name/metrics` reads `guestosinfo` + `filesystemlist`; `GET /api/vms/:name/guest-insight`
- **CDI wait** — clone waits for DataVolume Succeeded before start; `zorvia wait-image`; `POST /api/datavolumes/:name/wait`
- **Terraform provider schema** — scaffold writes `schema.json` (`zorvia_vm`, snapshots, downloads)
- **In-browser SSH** — `/ws/ssh/:vm?user=` proxies `ssh` or `virtctl ssh` into an xterm tab
- **CDI clone** — `POST /api/vms/:name/clone` with `clone_mode=cdi` creates a DataVolume from the source PVC
- **Cloud download jobs** — `POST /api/images/cloud/download` + `GET /api/images/downloads` (SPA-compatible)
- **Terraform scaffold** — `zorvia terraform-scaffold` writes example TF that drives the Fabric API
- **Pause / resume** — CLI (`zorvia pause|resume`), Fabric `POST /api/vms/:name/pause|resume`, and console buttons call KubeVirt VMI `pause` / `unpause` instead of returning 501
- **Template-backed cloud catalog** — `GET /api/images/cloud` lists unique containerdisks from the OS template library
- **Linux expose_vnc** — create-time VNC NodePort is no longer Windows-only
- **PVC-aware clone** — clone allocates empty same-size PVCs for PVC-backed source disks
- **Live-ish metrics/logs** — metrics include phase/paused/node; logs read virt-launcher pods
- **Web console VM ops** — Create VM (Linux cloud-init / Windows), serial console + VNC WebSocket proxies to KubeVirt, NodePort expose (SSH/VNC/RDP), clone, snapshot create/delete/revert; SPA routes under `/app/create` and `/app/vms/:name/console`
- **Fabric-compatible HTTP** — `POST /api/vms`, `/api/images`, port-forwards, cloud-init annotate, clone, snapshots; metrics/logs from KubeVirt; pause/resume via VMI subresources
- **Kube expose helpers** — managed NodePort Services labeled `zorvia.io/vm`; `ZORVIA_EXPOSE_HOST` for UI connection hints
- **Docs** — [docs/WEB_CONSOLE.md](docs/WEB_CONSOLE.md) for UI/API/WebSocket/RBAC
- **Drift Guard** — semantic desired-vs-live VM drift detection with Kubernetes-noise normalization, named-list canonicalization, operational risk scoring, JSON-pointer ignores, CI severity gates, and table/JSON/YAML output
- **Change Planner** — `zorvia plan` turns Drift Guard findings into conservative online/restart/recreate/manual-review execution plans with downtime gates
- **Guest Insight** — `zorvia guest-insight` summarizes QEMU Guest Agent state, guest OS/kernel, interfaces, and readiness score
- **CDI Golden Images** — `zorvia image-bundle` generates versioned DataVolumes plus stable DataSource aliases

### Changed

- **Console top nav** — dropped the "zyvor" wordmark next to the Zorvia logo mark; only the product name shows there now.
- **Kubernetes HTTPS (Veyron-style)** — in-pod rustls TLS via openssl init container; Service NodePort **30152** (`https://HOST:30152`). Host systemd `zorvia-web` is no longer the lab front door.
- **SPA Core nav** — Dashboard, VMs, Create VM, Favorites, Snapshots; product branding **Zorvia**
- **Renamed project to Zorvia** — crate, binary, config paths, deploy manifests, and docs now use `zorvia` / `Zorvia` (repository: [zyvorai/zorvia](https://github.com/zyvorai/zorvia))
- **Apache License 2.0 only** — removed the MIT dual-license; `LICENSE` is Apache-2.0 exclusively

### Removed

- **~37,000 lines of dead code** — an entire backend handler tree (`src/api/handlers/`, 44 files) that was never mounted on the live router (merged into an `all_routes()` function nothing called), and ~125 orphaned frontend pages/sub-components/API clients in `web/src/` that only that dead backend tree (or each other) referenced, and were unreachable from any real route or nav item. `src/api/routes.rs` — a different, live module backing the `zorvia api-routes` CLI command — was explicitly kept.

### Security

- **Path traversal prevention** - Profile and blueprint storage now sanitize names to block directory traversal attacks
- **CORS restricted by default** - API server CORS defaults to disabled instead of wildcard `*` origins
- **RDP cert validation enforced** - `ignore_cert` field is now ignored; TLS certificate validation is always enforced
- **Error message sanitization** - HTTP API responses no longer leak internal Kubernetes error details to clients
- **Request ID uniqueness** - API request IDs now include random suffix to prevent collisions under concurrency
- **PAM username length** - Username validation tightened from 256 to 32 characters (PAM LOGIN_NAME_MAX)
- **Console WS auth** — in-cluster proxy uses SA `token_file` + cluster CA when dialing KubeVirt subresources

### Fixed

- **`expose_rdp` did nothing for Linux VMs** — the "Expose RDP" checkbox in the Create VM wizard was gated on `guestOs === 'windows'` and the Linux submit path hardcoded `expose_rdp: false`, so choosing Linux silently dropped the option no matter what the user picked. Checkbox now gates on Linux (RDP is a Linux/xrdp scenario in this wizard; Windows exposure goes through Kryton) and the value is actually sent.
- **In-browser SSH couldn't authenticate with a password** — the SSH tab spawned `ssh -tt user@host` with plain `Stdio::piped()` stdin/stdout/stderr, i.e. no controlling terminal. OpenSSH's password prompt reads from `/dev/tty`, not stdin; with no tty available, that open fails and `ssh` silently submits 3 empty passwords and gives up — instantly, with no chance for the user to type anything, regardless of whether the password was correct. `proxy_ssh` (`src/api/http_server/web/ws_proxy_handlers.rs`) now allocates a real pty via `pty-process` for the spawned process, which also merges stdout+stderr into one stream like an actual terminal (the separate stderr pipe/reader is gone). Also fixes a prerequisite found while diagnosing this: the deployed container image had no `ssh` client binary at all (`deploy/Dockerfile.local` now installs `openssh-client`).
- **User database didn't survive a restart** — `deploy/k8s.yaml`'s `auth-data` volume was an `emptyDir`; every pod restart or redeploy silently reset all users back to just the bootstrap admin. Now backed by a `PersistentVolumeClaim` (`zorvia-auth-data`, `storageClassName` intentionally left unset so it binds to whatever the cluster's default class is, rather than hardcoding one that might not exist on another host).
- **Deploy manifest hardcoded a stale host's IP** — `ZORVIA_EXPOSE_HOST`/`HOST` in `deploy/k8s.yaml` had a literal IP address left over from an earlier deploy target, silently carried into every later deploy to a different host and producing wrong NodePort connection hints (e.g. in RDP `.rdp` files). `remote-deploy.sh` now substitutes the real target host into a `__ZORVIA_EXPOSE_HOST__` placeholder at apply time.

#### Error Messages Hidden Behind Generic 500s
Found while browser-testing VM creation and day-2 ops end-to-end against a real cluster: several handlers ran the generic `sanitize_error()` *before* their own classification logic, which collapsed Zorvia's own safe, structured error messages (e.g. `ZorviaError::VmExists`, `"VOLUME_NOT_FOUND: …"`, `"SHRINK_NOT_SUPPORTED: …"` — none start with `sanitize_error`'s allowed prefixes) down to a bare "Internal server error" before the classification checks ever saw the real text — so the classification always fell through to a generic 500.
- **VM create name conflicts** — `POST /api/vms` now reports `409` with the real "VM 'x' already exists" message instead of a generic 500
- **Disk resize failures** — `POST /api/vms/:name/disks/:name/resize` now reports `404`/`501`/`409` with the real reason instead of a generic 500
- **`sanitize_error()` itself** — cut the "keep the first sentence" boundary at the first `": "`, which for kube-rs's `"ApiError: <reason>"` format meant *any* passthrough error was truncated to the literal word `"ApiError"` with zero detail; now cuts at sentence-end (`". "`), bounded to 500 chars
- **NIC hotplug 404s** — `addinterface`/`removeinterface` unconditionally turned any 404 into "VM not found", including the case where the subresource route itself isn't registered on the cluster's KubeVirt version — now reports `501 UNSUPPORTED` for that case instead of falsely claiming a running VM doesn't exist
- **vCPU count after hotplug** — every VM API response (list/get/hotplug) computed vCPU count from `domain.cpu.cores` alone; CPU hotplug raises `sockets`, not `cores`, so the reported count silently reverted to the pre-hotplug value forever after a successful hotplug. Now `cores * sockets * threads`

#### Golden Image Downloads Were a No-Op
`DownloadRegistry::start()` built the CDI DataVolume/DataSource manifests and reported "Completed" without ever calling the Kubernetes API — a VM "created" from a downloaded image would fail since nothing was ever imported. Now actually applies both objects; the `dv:`-prefixed image reference it previously returned (which nothing recognized) is now `datavolume:<name>`, handled by the Create VM disk-image parser. Also added an explicit `accessModes: [ReadWriteOnce]` to the DataVolume spec — CDI rejects the import with `ErrClaimNotValid` on any StorageClass without a StorageProfile access mode (e.g. k3s's built-in `local-path`).

#### Crash Prevention
- **Terminal cleanup on panic** - TUI now restores terminal state even if `app.run()` panics or errors
- **Remove server panics** - Replaced `.unwrap()` with safe fallbacks in HTTP server JSON serialization and K8s client initialization
- **Lock poisoning recovery** - TUI blueprint/profile views use `unwrap_or_else` instead of `.expect()` on RwLock
- **Storage init fallback** - Profile/blueprint storage falls back to read-only mode instead of panicking on init failure
- **Rollback lifetime safety** - `execute_rollback()` returns owned `RollbackExecution` instead of borrowed reference

#### Error Handling
- **K8s connection failures surfaced** - Replaced `.unwrap_or_default()` with proper `?` error propagation on `list_vms()` calls in handlers and HTTP server
- **Evacuation error reporting** - `list_all_vms()` in backup handler now logs warnings on failure instead of silently returning empty
- **Audit log overflow warning** - Audit log now emits `log::warn!` when dropping oldest events at capacity
- **Buffer trim logging** - Anomaly detector, autoscaler, leak detector, and log aggregator now log when trimming history buffers

#### Arithmetic Safety
- **Integer overflow prevention** - Cost handler casts use `(val as u64).min(u32::MAX as u64) as u32` for memory/storage values
- **Pagination precision** - Page count calculation uses `u64` arithmetic before clamping to `u32` to avoid truncation
- **Year overflow** - Cost report `monthly_report()` uses `year.saturating_add(1)` instead of `year + 1`
- **Division by zero** - Cost forecast `project_weighted()` now guards `recent_days > 0.0` before dividing

#### Drain Panic Prevention
- Fixed 10 `Vec::drain()` operations across `audit_trail`, `search_history`, `tui/state`, `autoscaler`, `anomaly`, `log_aggregation`, `custom_metrics`, `leak_detector`, and `notifications` that could panic on boundary conditions

#### Logic Bugs
- **Anti-affinity rule fix** - Empty `vm_selector` now correctly means "no match" instead of unconditionally matching all nodes with VMs
- **Cron range validation** - Invalid ranges like `"5-1"` now return `false` instead of silently misbehaving
- **String slice bounds check** - Log pattern extraction guards against out-of-bounds string slicing on trailing quote characters

#### TUI Fixes
- **Tab state preserved** - VM details view retains selected tab when switching views (was always reset to 0)
- **Widget rendering bounds** - Input widget help text uses `saturating_add/sub` to prevent rendering outside allocated area
- **Placement bounds check** - Placement engine uses `.get(i)` instead of direct `[i]` indexing for node alternatives
- **Filter index validation** - `cycle_status_filter()` resets selection index safely against filtered list bounds
- **IP lookup error logging** - TUI state refresh logs debug message on `get_vm_ip()` failure instead of silently dropping
- **Unused import removed** - Removed unused `Span` import from bar chart widget (eliminated compiler warning)

#### Connection Pooling
- **HTTP server client reuse** - All 8 API handlers refactored to use shared `WebState.get_client()` instead of creating new `KubeClient::new()` per request

#### RDP Session
- **Serialization error handling** - RDP session creation logs error and returns error JSON instead of silently returning empty object

## [0.2.0] - 2026-02-28

### Added

#### Application Configuration
- Layered config file support: `/etc/zorvia/config.toml` (system) + `~/.config/zorvia/config.toml` (user)
- `config-show` command - display active configuration with source indicators
- `config-init` command - generate default config file
- Configurable: namespace, kubeconfig, API port/host/TLS/auth, logging level, output format, TUI preferences
- CLI args always take priority over config file values

#### Storage Module
- PVC management types: `PvcSpec`, `PvcStatus`, `StorageClassInfo`
- Storage size utilities: `parse_size_to_bytes()`, `format_bytes()` for K8s size strings (Ki/Mi/Gi/Ti)
- Builder pattern for PVC specs with namespace, storage class, access modes, volume mode

#### TUI Improvements
- Implemented actual VM creation in interactive mode (was stub)
- Implemented actual snapshot creation in interactive mode (was stub)
- Extract disk info from KubeVirt volumes instead of hardcoded values
- Extract node info from VM status conditions

#### CLI Tests
- 25 new CLI argument parsing tests
- Tests for core commands, resource overrides, flags, error cases

#### Integration Tests
- 15 new integration tests (6 → 21 total)
- All 44 templates validated and converted to KubeVirt
- All profiles and blueprints validated
- Example file parsing tests
- End-to-end workflow tests

### Changed

#### Architecture Refactor
- Extracted all command handlers from `lib.rs` into `src/handlers/` module (11 submodules)
- `lib.rs` reduced from 5,335 to 575 lines (-89%)
- Handler modules: vm, profiles, infra, backup, security, cost, automation, observability, multitenancy, devexp, api

#### Code Quality
- Eliminated all 79 compiler warnings
- Eliminated all 64 clippy warnings
- Replaced ~30 unsafe `.unwrap()` calls with proper error handling
- Fixed RwLock guards held across await points
- Renamed `from_str()` methods to `parse()` to avoid `FromStr` trait confusion
- Applied `cargo fmt` across entire codebase (189 files)

#### Naming Conventions
- Fixed non-camel-case enum variants: `PCI_DSS` → `PciDss`, `AI_ML` → `AiMl`, `AWS_KMS` → `AwsKms`, etc.
- Fixed deprecated `Frame::size()` → `Frame::area()`

#### CI/CD
- Added `RUST_MIN_STACK` to prevent test stack overflow
- Added job dependencies: fmt → clippy → test → build
- Consolidated cache paths

#### Documentation
- Updated `DEVELOPMENT.md` to reflect current state
- Removed 11 stale progress/completion reports
- Moved feature docs into `docs/` directory
- Clean project root: README, DEVELOPMENT, CONTRIBUTING, CHANGELOG, SECURITY, QUICK_REFERENCE

### Fixed
- Fixed 2 failing tests (`test_create_custom` in profiles and blueprints) - stale test data cleanup
- Fixed 5 snapshot test compilation errors (async/await, type mismatches)
- Fixed test imports broken by unused import cleanup
- Fixed 17 unused `mut` warnings in test code

#### Release Build
- Added LTO, single codegen unit, strip symbols to release profile
- Binary size reduced from 17MB to 11MB

### Statistics
- Commands: 147 (145 + config-show + config-init)
- Templates: 44 OS templates
- Resource Profiles: 8 built-in
- Deployment Blueprints: 5 built-in
- Tests: 2,076 (all passing)
- Compiler warnings: 0
- Clippy warnings: 0
- Lines of code: ~76,400

## [0.1.0] - 2024-02-05

### Added

#### Core VM Management
- `create` command - Create VMs from templates or configuration files
- `list` command - List VMs with table, YAML, or JSON output
- `get` command - Get detailed VM information
- `delete` command - Delete VMs with confirmation prompt
- `start`, `stop`, `restart` commands - VM lifecycle management
- `status` command - Detailed VM status with watch mode
- `clone` command - Clone existing VMs
- `resources` command - Cluster-wide resource usage summary
- `export` command - Export VM configurations
- `wizard` command - Interactive VM creation
- `batch` command - Batch VM creation from files

#### Templates & Configuration
- 6 built-in VM templates (Ubuntu, CentOS, Fedora, Debian, RHEL, Windows)
- `generate` command - Generate VM manifests
- `validate` command - Validate configuration files
- Cloud-init support, Builder pattern

#### Kubernetes Integration
- Full KubeVirt VirtualMachine CRD support
- VMConfig to KubeVirt manifest converter
- CRUD operations via Kubernetes API

#### Developer Features
- 31 unit and integration tests
- Library API for programmatic usage
- CI/CD with GitHub Actions

[Unreleased]: https://github.com/zyvorai/zorvia/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/zyvorai/zorvia/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/zyvorai/zorvia/releases/tag/v0.1.0
