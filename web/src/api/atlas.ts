// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiFetch, apiGet, apiPost, apiPut } from './client'
import { formatHttpErrorBody } from '../utils/apiError'
import { parseJsonResponse } from '../utils/parseJsonResponse'

export interface AtlasStatus {
  enabled: boolean
  connected: boolean
  tenant_id?: string | null
  health?: unknown
  error?: string
}

export interface AtlasCapabilities {
  block: boolean
  file: boolean
  object: boolean
  snapshots: boolean
  clone: boolean
  expansion: boolean
  replication: boolean
}

export interface AtlasBackend {
  id: string
  name: string
  backend_type: 'ceph' | 'nfs' | 'zfs' | 'san' | 'cloud_block' | 'kubernetes'
  mode: 'managed_rook' | 'external' | 'read_only'
  status: string
  capabilities: AtlasCapabilities
  connection_ref?: string | null
  cordoned: boolean
}

export type AtlasHealth = 'ok' | 'warn' | 'critical' | 'unknown'

export interface AtlasCluster {
  id: string
  backend_id: string
  name: string
  native_fsid?: string | null
  health: AtlasHealth
  raw_capacity_bytes?: number | null
  used_capacity_bytes?: number | null
  available_capacity_bytes?: number | null
}

export interface AtlasPool {
  id: string
  cluster_id: string
  name: string
  kind: string
  device_class?: string | null
  replica_size?: number | null
  used_bytes?: number | null
  max_bytes?: number | null
  health: AtlasHealth
}

export interface AtlasVolume {
  id: string
  cluster_id?: string | null
  pool_id?: string | null
  name: string
  kind: 'block' | 'filesystem' | 'object'
  backend_native_id?: string | null
  size_bytes: number
  used_bytes?: number | null
  state: string
  health: AtlasHealth
  kubernetes_namespace?: string | null
  pvc_name?: string | null
  storage_class_name?: string | null
}

export type AtlasJobState = 'pending' | 'queued' | 'running' | 'verifying' | 'succeeded' | 'failed'

/** Every Atlas write (volume create/expand/delete) returns one of these ids --
    this is how to find out what actually happened, instead of only ever seeing "queued". */
export interface AtlasJob {
  id: string
  tenant_id: string
  job_type: string
  state: AtlasJobState
  requested_by: string
  progress_percent: number
  error?: string | null
  result?: unknown
  created_at?: string | null
  updated_at?: string | null
}

export const isAtlasJobTerminal = (job: AtlasJob): boolean => job.state === 'succeeded' || job.state === 'failed'

/** The envelope every Atlas write (volume create/expand/delete) returns --
    a 202 + job id, never a finished result. Poll `getAtlasJob(job_id)`
    with it to find out what actually happened. */
export interface AtlasJobEnvelope {
  job_id: string
  state: string
  resource?: unknown
  links?: { job?: string }
}

export interface AtlasStorageClass {
  name: string
  provisioner: string
  reclaim_policy?: string | null
  volume_binding_mode?: string | null
  allow_volume_expansion?: boolean | null
  is_ceph: boolean
  labels: Record<string, string>
}

export interface AtlasVolumeOwner {
  product: string
  resource_type: string
  resource_id: string
  role?: string
}

export interface AtlasK8sVolumeOpts {
  namespace?: string
  create_pvc?: boolean
  access_modes?: string[]
  volume_mode?: string
  storage_class?: string
}

export interface CreateAtlasVolumeRequest {
  tenant_id?: string
  name: string
  size_bytes: number
  kind?: 'block' | 'filesystem' | 'object'
  policy?: string
  pool?: string
  owner?: AtlasVolumeOwner
  kubernetes?: AtlasK8sVolumeOpts
}

/** Tags a volume as owned by one of Zorvia's own VMs, so it's traceable back
    to Zorvia in Atlas's own inventory. */
export const atlasVolumeOwnerForVm = (vmName: string): AtlasVolumeOwner => ({
  product: 'zorvia',
  resource_type: 'vm',
  resource_id: vmName,
  role: 'data_disk',
})

export const getAtlasStatus = () => apiGet<AtlasStatus>('/api/v1/atlas/status')
export const listAtlasBackends = () => apiGet<AtlasBackend[]>('/api/v1/atlas/backends')
export const getAtlasBackendsSummary = () => apiGet<unknown>('/api/v1/atlas/backends/summary')
export const listAtlasClusters = () => apiGet<AtlasCluster[]>('/api/v1/atlas/clusters')
export const getAtlasClusterHealth = (id: string) => apiGet<unknown>(`/api/v1/atlas/clusters/${encodeURIComponent(id)}/health`)
export const listAtlasPools = () => apiGet<AtlasPool[]>('/api/v1/atlas/pools')
export const getAtlasCephStatus = () => apiGet<unknown>('/api/v1/atlas/ceph/status')
export const getAtlasCephDf = () => apiGet<unknown>('/api/v1/atlas/ceph/df')
export const listAtlasStorageClasses = () => apiGet<AtlasStorageClass[]>('/api/v1/atlas/storage-classes')
export const listAtlasVolumes = () => apiGet<AtlasVolume[]>('/api/v1/atlas/volumes')
export const createAtlasVolume = (body: CreateAtlasVolumeRequest) =>
  apiPost<AtlasJobEnvelope>('/api/v1/atlas/volumes', body)
export const expandAtlasVolume = (id: string, newSizeBytes: number) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/volumes/${encodeURIComponent(id)}/expand`, { new_size_bytes: newSizeBytes })
/** `confirm` must be true for production/protected-class volumes -- Atlas rejects the delete
    with a 400 naming that requirement if it's needed and omitted. Unlike the other writes this
    uses apiFetch directly (not the shared apiDelete, which discards the response body) because
    delete also returns a real job envelope that's worth polling like create/expand. */
export const deleteAtlasVolume = async (id: string, confirm = false): Promise<AtlasJobEnvelope> => {
  const url = `/api/v1/atlas/volumes/${encodeURIComponent(id)}${confirm ? '?confirm=true' : ''}`
  const res = await apiFetch(url, { method: 'DELETE' })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
  return parseJsonResponse<AtlasJobEnvelope>(res)
}

export const listAtlasJobs = () => apiGet<AtlasJob[]>('/api/v1/atlas/jobs')
export const getAtlasJob = (id: string) => apiGet<AtlasJob>(`/api/v1/atlas/jobs/${encodeURIComponent(id)}`)
export const cancelAtlasJob = (id: string) => apiPost<unknown>(`/api/v1/atlas/jobs/${encodeURIComponent(id)}/cancel`)

/** Polls a job to a terminal state (or gives up after `timeoutMs`, returning
    the last-seen state) -- the UI can then show the real outcome of a
    create/expand/delete instead of just "requested". */
export async function pollAtlasJob(
  jobId: string,
  opts?: { intervalMs?: number; timeoutMs?: number },
): Promise<AtlasJob> {
  const intervalMs = opts?.intervalMs ?? 1500
  const deadline = Date.now() + (opts?.timeoutMs ?? 60_000)
  for (;;) {
    const job = await getAtlasJob(jobId)
    if (isAtlasJobTerminal(job) || Date.now() >= deadline) return job
    await new Promise((r) => setTimeout(r, intervalMs))
  }
}

// ── Backend lifecycle ──

export interface CreateAtlasBackendRequest {
  name: string
  backend_type?: 'ceph' | 'nfs' | 'zfs' | 'san' | 'cloud_block' | 'kubernetes'
  mode?: 'managed_rook' | 'external' | 'read_only'
  server?: string
  targets?: string[]
}

export const createAtlasBackend = (body: CreateAtlasBackendRequest) =>
  apiPost<AtlasBackend>('/api/v1/atlas/backends', body)
export const deleteAtlasBackend = (id: string, purge = false) =>
  apiFetchDelete(`/api/v1/atlas/backends/${encodeURIComponent(id)}${purge ? '?purge=true' : ''}`)
export const discoverAtlasBackend = (id: string) =>
  apiPost<unknown>(`/api/v1/atlas/backends/${encodeURIComponent(id)}/discover`)
export const cordonAtlasBackend = (id: string) =>
  apiPost<unknown>(`/api/v1/atlas/backends/${encodeURIComponent(id)}/cordon`)
export const uncordonAtlasBackend = (id: string) =>
  apiPost<unknown>(`/api/v1/atlas/backends/${encodeURIComponent(id)}/uncordon`)

/** Shared with deleteAtlasVolume -- the DELETE routes here return a real
    body worth reading (not just success/fail), so they use apiFetch
    directly instead of the shared void-returning apiDelete. */
async function apiFetchDelete<T>(url: string): Promise<T> {
  const res = await apiFetch(url, { method: 'DELETE' })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
  return parseJsonResponse<T>(res)
}

// ── RBD images -- a separate identity space (rbd:<pool>/<image>) from the
// AtlasVolume list above; Atlas returns just names for the list route, not
// full objects. ──

export interface CreateAtlasRbdImageRequest {
  name: string
  size_bytes: number
  pool?: string
  tenant_id?: string
}

export const listAtlasRbdImages = (pool?: string) =>
  apiGet<{ pool: string; images: string[] }>(`/api/v1/atlas/rbd-images${pool ? `?pool=${encodeURIComponent(pool)}` : ''}`)
export const createAtlasRbdImage = (body: CreateAtlasRbdImageRequest) =>
  apiPost<AtlasJobEnvelope>('/api/v1/atlas/rbd-images', body)
export const deleteAtlasRbdImage = (pool: string, image: string) =>
  apiFetchDelete<AtlasJobEnvelope>(`/api/v1/atlas/rbd-images/${encodeURIComponent(pool)}/${encodeURIComponent(image)}`)
export const resizeAtlasRbdImage = (pool: string, image: string, sizeBytes: number, allowShrink = false) =>
  apiPost<AtlasJobEnvelope>(
    `/api/v1/atlas/rbd-images/${encodeURIComponent(pool)}/${encodeURIComponent(image)}/resize`,
    { size_bytes: sizeBytes, allow_shrink: allowShrink },
  )

// ── Object-store buckets + backups ──

export interface AtlasBucket {
  id: string
  tenant_id: string
  name: string
  bucket_name?: string | null
  endpoint?: string | null
  region?: string | null
  namespace?: string | null
  state: string
  created_at?: string | null
}

export interface AtlasBackup {
  id: string
  tenant_id: string
  volume_id: string
  snapshot_id?: string | null
  bucket_id: string
  object_key: string
  format: string
  state: string
  created_at?: string | null
}

export interface CreateAtlasBucketRequest {
  name: string
  namespace?: string
  storage_class?: string
  max_objects?: number
  max_size?: string
}

export const listAtlasBuckets = () => apiGet<AtlasBucket[]>('/api/v1/atlas/buckets')
export const getAtlasBucketStats = (id: string) => apiGet<unknown>(`/api/v1/atlas/buckets/${encodeURIComponent(id)}/stats`)
export const createAtlasBucket = (body: CreateAtlasBucketRequest) =>
  apiPost<AtlasJobEnvelope>('/api/v1/atlas/buckets', body)
export const deleteAtlasBucket = (id: string, force = false) =>
  apiFetchDelete<AtlasJobEnvelope>(`/api/v1/atlas/buckets/${encodeURIComponent(id)}${force ? '?force=true' : ''}`)

export const listAtlasBackups = (volumeId?: string) =>
  apiGet<AtlasBackup[]>(`/api/v1/atlas/backups${volumeId ? `?volume_id=${encodeURIComponent(volumeId)}` : ''}`)
export const deleteAtlasBackup = (id: string) =>
  apiFetchDelete<AtlasJobEnvelope>(`/api/v1/atlas/backups/${encodeURIComponent(id)}`)

// ── Disaster recovery (RBD mirroring) -- Atlas's own source labels this
// "scaffolding, real ops UNVERIFIED without a 2nd cluster". promote/demote/
// failover require Zorvia's strictest RBAC tier (cluster.admin), not just
// storage.admin like the rest of the Atlas surface. ──

export interface AtlasDrPeer {
  id: string
  name: string
  cluster_fsid?: string | null
  direction: string
  bootstrap_secret_ref?: string | null
  state: string
}

export interface AtlasDrMirror {
  id: string
  tenant_id: string
  volume_id?: string | null
  pool: string
  image: string
  peer_id?: string | null
  mode: string
  role: 'primary' | 'secondary'
  state: string
  rpo_seconds?: number | null
  last_failover_at?: string | null
  last_error?: string | null
  force_promoted: boolean
  updated_at: string
}

export interface AtlasDrStatus {
  peers: number
  mirrors: number
  primary: number
  secondary: number
  error: number
  worst_rpo_seconds?: number | null
  control_plane_ready: boolean
  dataplane_verified: boolean
  verified: boolean
  note: string
}

export interface AtlasDrPreflightCheck {
  id: string
  ok: boolean
  detail: string
}

export interface AtlasDrPreflight {
  ready: boolean
  control_plane_ready: boolean
  dataplane_verified: boolean
  checks: AtlasDrPreflightCheck[]
  blockers: string[]
  warnings: string[]
}

export interface RegisterAtlasDrPeerRequest {
  name: string
  cluster_fsid?: string
  direction?: string
  secret_ref?: string
}

export const listAtlasDrPeers = () => apiGet<AtlasDrPeer[]>('/api/v1/atlas/dr/peers')
export const registerAtlasDrPeer = (body: RegisterAtlasDrPeerRequest) =>
  apiPost<AtlasDrPeer>('/api/v1/atlas/dr/peers', body)
/** Atlas's delete is unconditional (no rows-affected check) -- this always
    returns `{deleted:true}`, even for an id that never existed, unlike
    backend/bucket/backup delete which 404 on a missing id. */
export const deleteAtlasDrPeer = (id: string) =>
  apiFetchDelete<{ peer_id: string; deleted: boolean }>(`/api/v1/atlas/dr/peers/${encodeURIComponent(id)}`)

export const listAtlasDrMirrors = () => apiGet<AtlasDrMirror[]>('/api/v1/atlas/dr/mirrors')
export const getAtlasDrStatus = () => apiGet<AtlasDrStatus>('/api/v1/atlas/dr/status')
export const getAtlasDrPreflight = () => apiGet<AtlasDrPreflight>('/api/v1/atlas/dr/preflight')

export const enableAtlasMirror = (volumeId: string, mode: 'snapshot' | 'journal' = 'snapshot', peer?: string) => {
  const params = new URLSearchParams({ mode })
  if (peer) params.set('peer', peer)
  return apiPost<AtlasJobEnvelope>(`/api/v1/atlas/volumes/${encodeURIComponent(volumeId)}/mirror?${params.toString()}`)
}
export const disableAtlasMirror = (volumeId: string) =>
  apiFetchDelete<AtlasJobEnvelope>(`/api/v1/atlas/volumes/${encodeURIComponent(volumeId)}/mirror`)

/** `force` is for split-brain / non-clean failover only -- without it,
    promote requires the mirror to currently be `role=secondary`. */
export const promoteAtlasMirror = (id: string, force = false) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/dr/mirrors/${encodeURIComponent(id)}/promote${force ? '?force=true' : ''}`)
export const demoteAtlasMirror = (id: string) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/dr/mirrors/${encodeURIComponent(id)}/demote`)
export const setAtlasMirrorRpo = (id: string, rpoSeconds: number) =>
  apiPost<{ mirror_id: string; rpo_seconds: number }>(`/api/v1/atlas/dr/mirrors/${encodeURIComponent(id)}/rpo`, {
    rpo_seconds: rpoSeconds,
  })

/** One-click failover runbook: runs preflight, then promotes `mirrorId`.
    `confirm: true` is required -- Atlas rejects with a `400` otherwise.
    If preflight isn't `ready`, Atlas rejects with a `409` naming the
    blockers unless `force` is also set. */
export const atlasDrFailover = (mirrorId: string, force = false) =>
  apiPost<AtlasJobEnvelope>('/api/v1/atlas/dr/failover', { mirror_id: mirrorId, confirm: true, force })

// ── AI-assisted insights -- compute-only (recommendation queries, not state
// mutation) despite the POST verbs on advisor/what-if. The local advisor is
// always available and never mutates storage. ──

export interface AtlasAdvisorAction {
  priority: number
  title: string
  rationale: string
  inspect: string
}

export interface AtlasAdvisorResponse {
  mode: string
  risk_score: number
  risk_level: string
  summary: string
  evidence: {
    capacity_used_percent: number
    days_to_full?: number | null
    open_alerts: number
    critical_alerts: number
    warning_alerts: number
    failed_jobs_15m: number
    degraded_objects: number
    unfound_objects: number
    alert_titles: string[]
  }
  actions: AtlasAdvisorAction[]
  warnings: string[]
  can_execute: boolean
}

export interface AtlasAnomaly {
  id: string
  metric: string
  label: string
  severity: string
  score: number
  current: number
  baseline: number
  change_percent: number
  direction: string
  explanation: string
  inspect: string
}

export interface AtlasAnomaliesResponse {
  generated_at: string
  window_minutes: number
  telemetry_status: string
  sensitivity: number
  anomalies: AtlasAnomaly[]
  warnings: string[]
}

export interface AtlasIncident {
  id: string
  category: string
  severity: string
  confidence: number
  title: string
  likely_cause: string
  inspect: string[]
}

export interface AtlasIncidentsResponse {
  generated_at: string
  count: number
  incidents: AtlasIncident[]
  narrative?: string | null
}

export interface AtlasWhatIfRequest {
  add_capacity_bytes?: number
  horizon_days?: number
  assume_alerts_resolved?: boolean
  assume_recovery_complete?: boolean
}

export interface AtlasWhatIfResponse {
  horizon_days: number
  baseline: { risk_score: number; risk_level: string; capacity_used_percent: number; days_to_full?: number | null }
  projected: { risk_score: number; risk_level: string; capacity_used_percent: number; days_to_full?: number | null }
  risk_delta: number
  actions: AtlasAdvisorAction[]
  assumptions: string[]
  can_execute: boolean
}

export const askAtlasAdvisor = (question: string) =>
  apiPost<AtlasAdvisorResponse>('/api/v1/atlas/ai/advisor', { question, mode: 'local' })
export const getAtlasAnomalies = () => apiGet<AtlasAnomaliesResponse>('/api/v1/atlas/ai/anomalies')
export const getAtlasIncidents = () => apiGet<AtlasIncidentsResponse>('/api/v1/atlas/ai/incidents')
export const runAtlasWhatIf = (body: AtlasWhatIfRequest) => apiPost<AtlasWhatIfResponse>('/api/v1/atlas/ai/what-if', body)

// ── Observability: metrics, alerts, audit, chargeback, policy drift, events.
// All read-only except the alert lifecycle actions (evaluate/ack/silence/
// resolve). Alerts are surfaced on the existing Alerts page (web/src/pages/
// Alerts.tsx) as an additional source, not a second alerts UI; audit/
// chargeback/policy-drift/metrics get simple read-only views scoped to the
// Atlas area of the Storage page instead of merging into Zorvia's own
// audit-trail/cost features. ──

export const getAtlasMetricsSummary = () => apiGet<Record<string, unknown>>('/api/v1/atlas/metrics/summary')
export const getAtlasMetricsHistory = (minutes = 60) => apiGet<Record<string, unknown>[]>(`/api/v1/atlas/metrics/history?minutes=${minutes}`)
export const getAtlasMetricsForecast = (minutes = 1440) =>
  apiGet<{ days_to_full: number | null; growth_bytes_per_day: number; used_capacity_bytes: number; raw_capacity_bytes: number; samples: number }>(
    `/api/v1/atlas/metrics/forecast?minutes=${minutes}`,
  )

export interface AtlasAlert {
  id: string
  severity: string
  source: string
  resource_type: string
  resource_id: string
  title: string
  description: string
  state: string
  created_at?: string | null
  resolved_at?: string | null
  acknowledged_at?: string | null
  acknowledged_by?: string | null
  silenced_until?: string | null
}

export const listAtlasAlerts = (state?: string) => apiGet<AtlasAlert[]>(`/api/v1/atlas/alerts${state ? `?state=${encodeURIComponent(state)}` : ''}`)
export const evaluateAtlasAlerts = () => apiPost<{ evaluated: boolean; open_alerts: number }>('/api/v1/atlas/alerts/evaluate')
export const ackAtlasAlert = (id: string) => apiPost<{ id: string; acknowledged_by: string }>(`/api/v1/atlas/alerts/${encodeURIComponent(id)}/ack`)
export const silenceAtlasAlert = (id: string, secs = 3600) =>
  apiPost<{ id: string; silenced_secs: number }>(`/api/v1/atlas/alerts/${encodeURIComponent(id)}/silence?secs=${secs}`)
export const resolveAtlasAlert = (id: string) => apiPost<{ id: string; state: string }>(`/api/v1/atlas/alerts/${encodeURIComponent(id)}/resolve`)

export interface AtlasAuditEntry {
  id: number
  actor_id: string
  action: string
  resource_type: string
  resource_id: string
  status: string
  created_at: string
  tenant_id?: string | null
}

export const listAtlasAudit = (limit = 50) => apiGet<AtlasAuditEntry[]>(`/api/v1/atlas/audit?limit=${limit}`)

/** Raw CSV text, not JSON -- Zorvia relays Atlas's export byte-for-byte. */
export async function exportAtlasAuditCsv(): Promise<string> {
  const res = await apiFetch('/api/v1/atlas/audit.csv')
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
  return res.text()
}

export interface AtlasChargebackTenant {
  tenant_id: string
  used_bytes: number
  used_gib: number
  volume_count: number
  quota_bytes: number
  estimated_usd_month: number
}

export const getAtlasChargeback = () => apiGet<{ usd_per_gib_month: number; tenants: AtlasChargebackTenant[] }>('/api/v1/atlas/chargeback')
export const getAtlasPolicyDrift = () => apiGet<{ count: number; drift: Record<string, unknown>[] }>('/api/v1/atlas/policy-drift')

export interface AtlasEvent {
  id: string
  kind: string
  actor: string
  title: string
  detail: string
  severity?: string
  resource_type: string
  resource_id: string
  ts: string
}

export const listAtlasEvents = (limit = 50) => apiGet<AtlasEvent[]>(`/api/v1/atlas/events?limit=${limit}`)

// ── Governance: tenants, quotas, protection schedules. Tenant policy
// overrides and volume labels/bindings are proxied (src/atlas/client.rs)
// but deliberately have no UI yet -- lower-frequency than quota/schedules.
// Deliberately excludes Atlas's own console auth (login/users/tokens). ──

export interface AtlasTenant {
  tenant_id: string
  used_bytes: number
  volume_count: number
  max_bytes: number
  max_volumes: number
}

export const listAtlasTenants = () => apiGet<AtlasTenant[]>('/api/v1/atlas/tenants')
export const putAtlasTenantQuota = (tenantId: string, maxBytes: number, maxVolumes: number) =>
  apiPut<AtlasTenant>(`/api/v1/atlas/tenants/${encodeURIComponent(tenantId)}/quota`, {
    max_bytes: maxBytes,
    max_volumes: maxVolumes,
  })

export interface AtlasSchedule {
  id: string
  tenant_id: string
  volume_id: string
  kind: string
  bucket_id?: string | null
  mode: string
  interval_secs: number
  keep: number
  enabled: boolean
  last_run_at?: string | null
  next_run_at?: string | null
  created_at?: string | null
}

export interface CreateAtlasScheduleRequest {
  interval_secs: number
  keep?: number
  kind?: 'snapshot' | 'backup'
  bucket_id?: string
  mode?: string
}

export const listAtlasSchedules = (volumeId?: string) =>
  apiGet<AtlasSchedule[]>(`/api/v1/atlas/schedules${volumeId ? `?volume_id=${encodeURIComponent(volumeId)}` : ''}`)
export const createAtlasSchedule = (volumeId: string, body: CreateAtlasScheduleRequest) =>
  apiPost<AtlasSchedule>(`/api/v1/atlas/volumes/${encodeURIComponent(volumeId)}/schedule`, body)
export const deleteAtlasSchedule = (id: string) =>
  apiFetchDelete<{ deleted: string }>(`/api/v1/atlas/schedules/${encodeURIComponent(id)}`)
