# Zorvia + Atlas integration

Zorvia stays the KubeVirt VM control plane; Atlas is an optional, separate storage control plane (Ceph/NFS/ZFS today, pluggable backends) that Zorvia can read from — and provision one thing through — for teams who run it instead of (or alongside) managing Rook-Ceph directly.

## Architecture

```text
Browser
  │  Zorvia JWT / API key
  ▼
Zorvia web + API
  │  server-side Atlas bearer token (product.service.* role)
  │  /api/v1/atlas/*
  ▼
Atlas gateway (/api/atlas/v1/*)
  ├─ Ceph driver
  ├─ NFS driver (planned)
  └─ ZFS driver (planned)
```

The browser never receives `ATLAS_TOKEN`. Zorvia owns user authentication; the Atlas service-account token is a server secret, exactly like `KRYTON_TOKEN`.

## Storage page integration

Atlas isn't a separate top-level page — it's a set of sections on the existing Storage page (`/app/storage`, alongside Rook-Ceph), because both are answering the same question ("what storage do I have and what's it doing"), just from two different control planes. If `ATLAS_URL` is unset, the sections render a quiet "not configured" note instead of hiding entirely or erroring.

The Atlas UI lives under `web/src/pages/storage/`: `AtlasSection.tsx` is the main orchestrator (status, backend lifecycle — create/discover/cordon/uncordon/delete, volume create/expand/delete, recent jobs), and `AtlasRbdSection.tsx`/`AtlasBucketsSection.tsx`/`AtlasDrSection.tsx`/`AtlasAiSection.tsx`/`AtlasObservabilitySection.tsx`/`AtlasGovernanceSection.tsx` are sibling cards (only rendered once Atlas is enabled and connected) covering RBD image create/resize/delete, object-store bucket create/delete, disaster recovery, AI-assisted insights, audit/chargeback/policy-drift/metrics, and tenant quotas/protection schedules respectively. Atlas alerts are surfaced on Zorvia's existing `/app/alerts` page instead (see the Observability section below), and **DataBridge is a separate top-level page** (`/app/databridge`, `web/src/pages/DataBridge.tsx`) rather than a Storage-page section — see the DataBridge section below for why. Deliberately out of scope for the UI, even though the backend routes exist and are proxied: RBD clone/migrate/flatten/QoS/snapshots, bucket object-level operations (list/upload/download/prune), backup/restore creation, tenant policy overrides, and volume labels/bindings — these are lower-frequency operations better suited to Atlas's own UI or CLI for now; revisit if there's real demand.

## Configuration

Set these on the Zorvia API process:

| Variable | Required | Meaning |
|---|---:|---|
| `ATLAS_URL` | yes to enable | Atlas gateway base URL, e.g. `http://atlas-gateway.atlas.svc.cluster.local:8080` |
| `ATLAS_TOKEN` | yes for anything beyond `/health`/`/version` | A pre-minted Atlas JWT with a `product.service.*` role (see auth.rs's `role_level` — that role prefix is specifically for sibling-product service accounts) and the right `tenant_id` claim |
| `ATLAS_TENANT_ID` | yes to create volumes without specifying one per-request | Default tenant scope for volume creation |
| `ATLAS_TIMEOUT_SECS` | no | Upstream HTTP timeout, default 30 |
| `ATLAS_TLS_INSECURE` | no | `true` only for explicitly trusted labs using self-signed TLS |

### Connecting a cluster

The deployment manifests and Helm chart already carry the wiring; you only supply the URL and a token.

1. Mint a service-account token from Atlas (admin credential required; shown here with Atlas's
   bootstrap admin token, which never has to leave the cluster):
   ```bash
   curl -X POST http://<atlas>:5110/api/atlas/v1/auth/tokens \
     -H "Authorization: Bearer $ATLAS_ADMIN_TOKEN" -H 'content-type: application/json' \
     -d '{"subject":"zorvia","role":"operator","ttl_secs":7776000}'   # 90 days is the maximum
   ```
   Use `admin` instead of `operator` only if Zorvia should be allowed to delete volumes.
2. Store it: `kubectl -n zorvia-system create secret generic zorvia-atlas --from-literal=token=<jwt>`.
3. Enable it: raw manifests read `ATLAS_URL` (empty = off) and the optional `zorvia-atlas` Secret;
   with Helm set `atlas.url` and `atlas.tokenSecret`. Tokens expire, so re-mint before `ttl_secs` runs out.

Atlas buckets can also be the target for off-cluster VM backups: set `ZORVIA_BACKUP_ATLAS_BUCKET`
(see `docs/OFFCLUSTER_BACKUP.md`).

If `ATLAS_URL` is absent, Zorvia starts normally and the integration reports `enabled=false`. `GET /health` and `GET /version` on the Atlas gateway are unauthenticated; every other route (everything under `/api/atlas/v1/...`) requires the bearer token.

## Zorvia endpoints

- `GET /api/v1/atlas/status` — degrades gracefully to `{"enabled":false}` rather than erroring
- `GET /api/v1/atlas/backends`
- `GET /api/v1/atlas/backends/summary`
- `GET /api/v1/atlas/clusters`
- `GET /api/v1/atlas/clusters/:id/health`
- `GET /api/v1/atlas/pools`
- `GET /api/v1/atlas/ceph/status`
- `GET /api/v1/atlas/ceph/df`
- `GET /api/v1/atlas/storage-classes`
- `GET|POST /api/v1/atlas/volumes` — `POST` creates a volume
- `POST /api/v1/atlas/volumes/:id/expand` — `{new_size_bytes}`
- `DELETE /api/v1/atlas/volumes/:id[?confirm=true]`
- `GET /api/v1/atlas/jobs` — most-recent 100 (Atlas's own server-side cap, no pagination)
- `GET /api/v1/atlas/jobs/:id`
- `POST /api/v1/atlas/jobs/:id/cancel` — `409` if the job is already terminal (`succeeded`/`failed`)
- `POST /api/v1/atlas/backends` — register a backend row; only `nfs`/`zfs` `backend_type`s get instantiated live, others land `pending`
- `DELETE /api/v1/atlas/backends/:id[?purge=true]` — refused if volumes still reference it unless `purge=true`
- `POST /api/v1/atlas/backends/:id/discover`, `/cordon`, `/uncordon`
- `GET|POST /api/v1/atlas/maintenance` — `POST {paused}` pauses/resumes Atlas's whole job engine (all tenants)
- `GET /api/v1/atlas/maintenance/orphans`, `GET /api/v1/atlas/upgrade/preflight` (report-only, always `200`; `ready`/`blockers` is the real gate)
- `GET /api/v1/atlas/osds`
- `POST /api/v1/atlas/osds/:id/{out,in}`, `POST /api/v1/atlas/osds/:id/reweight?weight=0.0-1.0`
- `GET|POST /api/v1/atlas/rbd-images[?pool=]` — a **separate identity space** (`rbd:<pool>/<image>`) from the `StorageVolume` abstraction the volume routes above use; don't conflate a "volume" with an "RBD image"
- `DELETE /api/v1/atlas/rbd-images/:pool/:image`
- `POST /api/v1/atlas/rbd-images/:pool/:image/{clone,resize,migrate,flatten,qos}`
- `GET|POST /api/v1/atlas/rbd-images/:pool/:image/snapshots`, `POST .../rollback`, `DELETE .../snapshots/:snap`
- `POST /api/v1/atlas/rbd-usage/refresh` — synchronous, recomputes `used_bytes` for every RBD-backed volume
- `GET|POST /api/v1/atlas/buckets`, `GET|DELETE /api/v1/atlas/buckets/:id[?force=true]`, `GET /api/v1/atlas/buckets/:id/stats`
- `GET|DELETE /api/v1/atlas/buckets/:id/objects[?key=]`, `POST .../objects/upload-url`, `GET .../objects/download-url?key=`, `POST .../objects/prune`
- `POST /api/v1/atlas/backup-jobs`, `POST /api/v1/atlas/restore-jobs`
- `GET /api/v1/atlas/backups[?volume_id=]`, `GET|DELETE /api/v1/atlas/backups/:id`, `GET /api/v1/atlas/backups/:id/download[?what=manifest|data]`
- `GET|POST /api/v1/atlas/dr/peers`, `DELETE /api/v1/atlas/dr/peers/:id`
- `GET /api/v1/atlas/dr/mirrors`, `GET /api/v1/atlas/dr/status`, `GET /api/v1/atlas/dr/preflight` (report-only, always `200`; `ready`/`blockers` is the real gate)
- `POST /api/v1/atlas/dr/mirrors/:id/{promote,demote}`, `POST /api/v1/atlas/dr/mirrors/:id/rpo`, `POST /api/v1/atlas/dr/failover`
- `POST|DELETE /api/v1/atlas/volumes/:id/mirror` — enable/disable RBD mirroring for a volume
- `POST /api/v1/atlas/ai/advisor` — `{question?, mode?}`; risk-scored advisory with evidence and recommended (not executed) actions
- `GET /api/v1/atlas/ai/anomalies[?minutes=&sensitivity=]` — statistical anomaly detection over recent metric samples
- `GET /api/v1/atlas/ai/incidents[?mode=]` — correlated incidents (alerts + anomalies + failed jobs grouped by likely cause)
- `POST /api/v1/atlas/ai/what-if` — `{add_capacity_bytes?, horizon_days?, projected_growth_bytes_per_day?, assume_alerts_resolved?, assume_recovery_complete?}`; projects capacity/risk under a hypothetical
- `GET /api/v1/atlas/metrics/summary`, `GET .../metrics/ceph[?prefix=]`, `GET .../metrics/history[?minutes=]`, `GET .../metrics/forecast[?minutes=]`
- `GET /api/v1/atlas/alerts[?state=open]`, `POST .../alerts/evaluate`, `POST .../alerts/:id/{ack,resolve}`, `POST .../alerts/:id/silence[?secs=]`
- `GET /api/v1/atlas/audit[?actor=&action=&resource_type=&resource_id=&limit=]`, `GET /api/v1/atlas/audit.csv` (raw CSV, not JSON)
- `GET /api/v1/atlas/chargeback`, `GET /api/v1/atlas/policy-drift`, `GET /api/v1/atlas/events[?limit=]`
- `GET /api/v1/atlas/tenants` — usage + quota overview for every tenant with volumes or a quota
- `GET /api/v1/atlas/policies` — global policy templates (intent → default storage-class placement)
- `GET /api/v1/atlas/tenants/:id/policies`, `PUT|DELETE /api/v1/atlas/tenants/:id/policies/:intent` — per-tenant placement overrides
- `GET|PUT /api/v1/atlas/tenants/:id/quota` — `{max_bytes, max_volumes}`, `0` = unlimited
- `POST /api/v1/atlas/volumes/:id/schedule`, `GET /api/v1/atlas/schedules[?volume_id=]`, `DELETE /api/v1/atlas/schedules/:id`
- `GET|PUT /api/v1/atlas/volumes/:id/labels`, `GET /api/v1/atlas/volumes/:id/bindings`
- `GET|POST /api/v1/atlas/databridge/sources`, `GET|DELETE .../sources/:id`, `POST .../sources/:id/discover`
- `GET|POST /api/v1/atlas/databridge/plans`, `GET|DELETE .../plans/:id`
- `POST /api/v1/atlas/databridge/plans/:id/{assess,provision,full-load,validate,cutover,rollback}`, `POST .../plans/:id/cdc/{start,stop,restart}`
- `GET /api/v1/atlas/databridge/edge-clusters`, `GET|DELETE .../edge-clusters/:id`
- `GET /api/v1/atlas/databridge/cdc-streams`, `GET .../cdc-streams/:id`
- `GET|POST /api/v1/atlas/databridge/object`, `GET|DELETE .../object/:id` (delete returns `204`, not JSON), `POST .../object/:id/start`
- `GET /api/v1/atlas/databridge/validations[?plan_id=]`, `GET /api/v1/atlas/databridge/cutovers`

Bucket create/delete and backup create/delete/restore are async jobs, same shape as the volume writes; everything else under buckets (stats, object list/delete, upload/download URL, prune) is synchronous — object operations never touch bytes through Zorvia or Atlas, they mint presigned S3 URLs so the browser talks to RGW directly. `delete` on a bucket is refused (`409`) while it still holds backups unless `?force=true`.

RBD writes are async jobs, same shape as the volume writes, except `rbd-usage/refresh` (synchronous, like backend lifecycle). `migrate` takes `?dest_pool=`; `qos` takes `?iops=&bps=` (at least one required, `0` clears a cap); `resize` takes `{size_bytes, allow_shrink}` in the body (`allow_shrink` guards against accidental data loss, default `false`); `rollback` takes `{name: <snapshot name>}` in the body, same shape as creating a snapshot.

Unlike the volume routes, **backend lifecycle (create/delete/discover/cordon/uncordon/maintenance) is synchronous** — Atlas returns the finished result directly, no job envelope, no polling needed. OSD ops (`out`/`in`/`reweight`) *are* async jobs, same shape as the volume writes.

All three volume-mutating routes return `202` with Atlas's job envelope (`{"job_id", "state":"queued", "resource": {...}, "links": {"job": "/api/atlas/v1/jobs/:id"}}`), not a finished result — every Atlas write is genuinely async. Poll `GET /api/v1/atlas/jobs/:id` with that `job_id` to find out what actually happened; `state` reaches a terminal value (`succeeded`/`failed`) or stays `pending`/`queued`/`running`/`verifying` in between. The web UI (`RookStorage.tsx`'s Atlas section) does exactly this after every create/expand/delete — `pollAtlasJob()` in `web/src/api/atlas.ts` polls every 1.5s up to a 60s timeout and the UI toasts the real outcome, not just "requested". `POST /volumes` accepts `{name, size_bytes, policy?, pool?, owner?, kubernetes?}` and forwards to Atlas's `CreateVolumeRequest`; when called in the context of a VM, populate `owner: {product: "zorvia", resource_type: "vm", resource_id: <vm name>, role: "data_disk"}` (the web UI's "Attribute to VM" field does this) — that's what makes the volume traceable back to Zorvia in Atlas's own inventory, not a cosmetic detail. `DELETE` always passes `confirm=true` from Zorvia's side; Atlas itself decides (based on storage class / protection tier) whether that was actually required, rejecting with a `400` naming the requirement if it was needed and the caller didn't set it — Zorvia doesn't try to duplicate that judgment. All `/api/v1/atlas/*` routes are protected by Zorvia's existing auth middleware; the volume-mutating routes and `jobs/:id/cancel` additionally require the `storage.admin` permission (same permission that gates Rook administration) — note Atlas's own token-level auth is stricter for delete and cancel specifically (`ROLE_ADMIN` vs. `ROLE_OPERATOR` for create/expand), so those two can still be rejected by Atlas even when Zorvia's own RBAC allows the request through.

## Disaster recovery

Cross-cluster RBD mirroring is **scaffolding on Atlas's own side** — its `dr.rs` source carries the comment "real ops UNVERIFIED without a 2nd cluster". Zorvia proxies the full control-plane surface (peer registration, mirror role/state bookkeeping, preflight checks, promote/demote/failover) because the plan explicitly asked for it, but treat it accordingly:

- **What's actually verified**: the control-plane catalog — registering a peer, enabling/disabling mirroring on a volume, listing mirrors, the role-transition guards (promote refuses `role=primary` without `?force=1`; demote refuses `role=secondary`), `POST /dr/failover`'s `confirm=true` requirement and its preflight gate, and `GET /dr/preflight`'s blockers/warnings — all round-trip correctly against a real `atlas-gateway` in `fake` driver mode. This is genuinely useful for exercising the runbook and RBAC.
- **What's not verified**: the actual `rbd mirror` data-plane operations only run when Atlas's Ceph driver is in `real` mode against a genuine second Ceph cluster. `GET /dr/status`'s `dataplane_verified`/`verified` fields report whether *this specific Atlas deployment* has ever completed that live two-site drill (see Atlas's own `docs/DR.md`) — the web UI surfaces this as a persistent warning, not a one-time dismissable notice.
- **Stricter RBAC**: `promote`, `demote`, and `failover` require Zorvia's `cluster.admin` permission (the strictest tier), checked *before* the general `storage.admin` rule that covers the rest of `/v1/atlas/*` — see `src/api/auth/permissions.rs`. This is defense-in-depth beyond Atlas's own token-role check (`ROLE_ADMIN` for all three), since these routes can flip which cluster is primary.
- **`DELETE /dr/peers/:id` is unconditional**: Atlas's `delete_peer` doesn't check rows-affected, so it always returns `{"deleted":true}` — even for an id that never existed — unlike backend/bucket/backup delete, which 404 on a missing id. Don't infer "the peer existed" from a successful response.
- The web UI (`AtlasDrSection.tsx`) shows a persistent warning quoting this caveat and requires the same confirm-dialog pattern used for other destructive actions before promote/demote/failover — including naming preflight blockers in the confirmation text when preflight isn't `ready`, rather than only stopping the user after the fact.

## AI-assisted insights

Compute-only, despite the `POST` verbs on `advisor`/`what-if`: none of the four routes mutate storage (`can_execute` is always `false` in every response). The local advisor is a deterministic rules-over-evidence engine, always available; an optional external OpenAI-compatible provider (configured on Atlas's side via `ATLAS_AI_BASE_URL`/`ATLAS_AI_MODEL`, not Zorvia's) can only rewrite the executive-summary text, never the risk score or evidence. Zorvia's web UI (`AtlasAiSection.tsx`) always passes `mode: "local"` to the advisor and `ai/incidents`, so it never triggers a real external-network call on Atlas's behalf just from loading the Storage page — a provider-narrated summary would need a direct API call with `mode: "auto"` or `"llm"`, which isn't wired into the UI. `GET /ai/incidents` already defaults to `mode=local` on Atlas's side for the same reason (narration is opt-in, not automatic, on a `GET` a dashboard might poll).

## Observability

Metrics (`summary`/`ceph`/`history`/`forecast`) are simple read views. Alerts are a real lifecycle: `evaluate` runs Atlas's rules on demand (a genuine side effect — creates/updates alert rows, not a pure read, despite returning just a count); `ack` records who saw it without resolving; `silence` suppresses webhook notification for a window while the condition keeps being tracked and still shows in `list_alerts`; `resolve` is an operator override. All three per-alert actions `404` on an id that isn't currently open/tracked.

**Alerts are surfaced on Zorvia's existing `/app/alerts` page** (`web/src/pages/Alerts.tsx`) as an additional source alongside Zorvia's own VM-metric alert rules, tagged with an "Atlas" badge, rather than as a second disconnected alerts UI — Zorvia already has a real alerts feature and unifying the two into one screen is more useful than fragmenting them. Atlas alerts are fetched only when `getAtlasStatus()` reports `enabled && connected`, and failures there degrade to an empty list rather than surfacing as a page-level error, since Atlas is an optional secondary alert source on this page.

**Audit, chargeback, and policy-drift get simple read-only views scoped to the Atlas area** of the Storage page (`AtlasObservabilitySection.tsx`) instead of being merged into Zorvia's own audit trail or cost/FinOps surfaces — those are Zorvia's own equivalents for Zorvia's own resources, and unifying them with Atlas's storage-only audit/chargeback is a separate product decision, not something to fold in here without a deliberate call. `GET /audit.csv` returns raw CSV (not JSON) — Zorvia's `exportAtlasAuditCsv()` relays it byte-for-byte with the same `text/csv` content type, and the UI offers it as a browser download rather than trying to render it as a table alongside the JSON `GET /audit` view.

`GET /events` (jobs + audit + alerts, unified) is proxied but not yet surfaced in the UI — it's operator-gated on Atlas's side because it includes audit records, same as `GET /audit`.

## Governance

Tenant quotas (`{max_bytes, max_volumes}`, `0` = unlimited) and per-tenant policy overrides (which storage class an intent like `production`/`database` resolves to for one tenant) are legitimate day-2 admin surfaces, proxied in full. Protection schedules (periodic snapshot or backup jobs for a volume) are proxied too — Atlas validates the volume is PVC-backed (CSI) before creating one, rejecting a schedule against a raw NFS/ZFS volume with a `400` rather than silently creating a permanent no-op (its "next run" timer would advance forever with no job ever enqueued).

**The web UI (`AtlasGovernanceSection.tsx`) covers tenant quota editing and schedule create/list/delete** — the day-2 operations an operator actually reaches for. Tenant policy overrides and volume labels/bindings are proxied on the backend (`put_tenant_policy`/`delete_tenant_policy`/`get_volume_labels`/`put_volume_labels`/`list_volume_bindings` in `src/atlas/client.rs`) but deliberately have no UI yet — lower-frequency than quota/schedule management; revisit if there's real demand, same as the RBD/bucket UI scope cuts.

**Deliberately excluded from proxying entirely: Atlas's own console authentication** — `POST /auth/login`, `GET|POST /auth/users`, `PUT|DELETE /auth/users/:username`, `POST /auth/tokens`, `GET /auth/tokens/revoked`, `POST /auth/tokens/:jti/revoke`. These manage *Atlas's* own accounts and service-account tokens, not Zorvia's — a compromised or misconfigured Zorvia becoming a pass-through admin console for a different product's user base is a real security-boundary concern independent of implementation effort, not a build-it-later item. This is a deliberate, permanent exclusion from the approved integration scope, not an oversight; revisit only with a separate, explicit conversation if a real need for it emerges.

## DataBridge

Cloud-to-edge database migration — a genuinely different product domain from VM/storage provisioning, so it's its own top-level page (`/app/databridge`, `web/src/pages/DataBridge.tsx`), not a Storage-page section. The pipeline is staged: register a `source` → `discover` its schema (async job) → create a `plan` against that source → `assess` (readiness score + blockers) → `provision` an edge DB cluster on Ceph → `full-load` → `cdc/start` (ongoing change capture) → `validate` (rowcount/checksum comparison) → `cutover` → optionally `rollback`. A separate, independent object-leg migration (`databridge/object`) copies an S3-protocol bucket straight to Ceph RGW, with no plan/source involved.

**`cutover` and `rollback` are guarded on Atlas's own side, not just RBAC**: `cutover` requires the plan to be `validated`, its most recent validation to have `passed`, and (if a CDC stream is attached) lag to be under a 10-second threshold — Atlas rejects with a specific `409` naming which precondition failed, not a generic error. `rollback` requires an existing cutover and that its rollback window (set at plan creation, default 72h) hasn't closed. Zorvia relays these errors as-is rather than trying to duplicate the checks client-side; the web UI's confirm dialogs describe the preconditions up front so a rejection isn't a surprise, but don't attempt to enforce them before submitting.

**Verified live end-to-end** against a real `atlas-gateway` (fake driver mode): a full plan pipeline from source registration through discover (real fake schema with tables/row-counts), assess (real readiness score + blockers), provision (real edge cluster row), full-load, cdc-start, a rejected cutover attempt before validation (`409`), validate (real per-table checksum comparison), a rejected cutover attempt while CDC lag exceeded the threshold (`409`, confirming the fake driver's lag decays over time via its reconciler rather than instantly), a successful cutover once lag drained, and rollback. Also confirmed: `DELETE` on a source is refused (`409`) while any plan still references it (naming the plan), and `DELETE` on a plan/edge-cluster/object-migration is **unconditional** (no rows-affected check) — a second delete of an already-deleted id still returns success, same quirk as `DELETE /dr/peers/:id`.

## Deliberately not proxied yet

This is a read-only inventory integration plus volume/RBD/bucket lifecycle, job status polling, disaster recovery scaffolding, AI-assisted insights, observability, governance, and DataBridge — essentially the full approved integration scope. Not proxied:

- Job SSE watch (`GET /jobs/:id/watch`) — Zorvia has no established SSE-proxy pattern; polling `GET /jobs/:id` on the existing 10-15s refresh cadence is good enough for now
- Atlas's own console authentication (`/auth/login`, `/auth/users*`, `/auth/tokens*`) — see Governance above; this one is a deliberate permanent exclusion, not a "not yet"

If any of these become a real need, they follow the same pattern as the volume routes here — add the client method, the handler, and (if it's a write) a permission check and audit entry.

## Production recommendations

1. Run Atlas behind a ClusterIP service reachable only from Zorvia and trusted operators.
2. Keep `ATLAS_TOKEN` in a Kubernetes Secret, minted with a `product.service.zorvia`-style role and the correct `tenant_id`.
3. Rotate the Atlas token independently of Zorvia user credentials.
4. Keep `ATLAS_TLS_INSECURE=false` outside isolated labs.
5. `POST /volumes` is genuinely async: a successful call returns `202` with a
   `job_id` (`{"state":"queued", "resource":{"volume_id":..., "pvc":...}, ...}`),
   not a finished volume — Zorvia's response mirrors that `202` as-is and
   doesn't poll the job to completion. Verified live against a local
   `atlas-gateway` (fake Ceph driver): the request is accepted correctly, but
   the job then fails with `"no Kubernetes cluster is attached; cannot run
   the write path"` when Atlas itself has no `ATLAS_KUBECONFIG` configured —
   Atlas needs a real Kubernetes cluster attached (via `ATLAS_KUBECONFIG` or
   in-cluster config) to actually create the PVC, independent of anything on
   Zorvia's side. Confirm your Atlas instance has that before relying on
   volume creation from Zorvia, and don't assume a `202` means the volume
   exists yet. The same applies to expand/delete.
6. Volume delete requires the `ATLAS_TOKEN` to carry `ROLE_ADMIN` on Atlas's
   side (create/expand only need `ROLE_OPERATOR`) — a token scoped to
   operator-level will get a real `403` from Atlas on delete even though
   Zorvia's own RBAC allowed the request through. Verified live: deleting a
   volume Atlas considers production/protected-class also requires
   `?confirm=true`, which Zorvia always sends — Atlas is the one deciding
   whether that was actually needed.
