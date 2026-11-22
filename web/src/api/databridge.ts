// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiFetch, apiGet, apiPost } from './client'
import { formatHttpErrorBody } from '../utils/apiError'
import { parseJsonResponse } from '../utils/parseJsonResponse'

/** Cloud-to-edge DB migration -- a genuinely different product domain from
    VM/storage provisioning, proxied from Atlas's DataBridge surface. Most
    writes here are synchronous (source/plan/object-migration create/
    delete) except the job-backed lifecycle triggers (discover, assess,
    provision, full-load, cdc start/stop/restart, validate, cutover,
    rollback, object-migration start), which return the same
    {job_id,state,resource,links} envelope as the rest of the Atlas
    integration. */

export interface AtlasJobEnvelope {
  job_id: string
  state: string
  resource?: unknown
  links?: { job?: string }
}

export type AtlasJobState = 'pending' | 'queued' | 'running' | 'verifying' | 'succeeded' | 'failed'

export interface AtlasJob {
  id: string
  state: AtlasJobState
  progress_percent: number
  error?: string | null
}

export const isAtlasJobTerminal = (job: AtlasJob): boolean => job.state === 'succeeded' || job.state === 'failed'

export async function pollAtlasJob(jobId: string, opts?: { intervalMs?: number; timeoutMs?: number }): Promise<AtlasJob> {
  const intervalMs = opts?.intervalMs ?? 1500
  const deadline = Date.now() + (opts?.timeoutMs ?? 60_000)
  for (;;) {
    const job = await apiGet<AtlasJob>(`/api/v1/atlas/jobs/${encodeURIComponent(jobId)}`)
    if (isAtlasJobTerminal(job) || Date.now() >= deadline) return job
    await new Promise((r) => setTimeout(r, intervalMs))
  }
}

async function apiFetchDelete<T>(url: string): Promise<T> {
  const res = await apiFetch(url, { method: 'DELETE' })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
  return parseJsonResponse<T>(res)
}

// ── Sources ──

export interface DataBridgeSource {
  id: string
  tenant_id: string
  name: string
  kind: string
  cloud: string
  endpoint?: string | null
  port?: number | null
  database?: string | null
  tls_mode: string
  driver_mode: string
  state: string
  discovered?: Record<string, unknown>
  created_at?: string | null
}

export interface CreateDataBridgeSourceRequest {
  name: string
  kind: 'postgres' | 'mysql' | 'mariadb' | 'oracle' | 'sqlserver' | 'mongodb'
  cloud?: 'rds' | 'aurora' | 'cloudsql' | 'generic'
  endpoint?: string
  port?: number
  database?: string
  tls_mode?: string
}

export const listDataBridgeSources = () => apiGet<DataBridgeSource[]>('/api/v1/atlas/databridge/sources')
export const getDataBridgeSource = (id: string) => apiGet<DataBridgeSource>(`/api/v1/atlas/databridge/sources/${encodeURIComponent(id)}`)
export const createDataBridgeSource = (body: CreateDataBridgeSourceRequest) =>
  apiPost<DataBridgeSource>('/api/v1/atlas/databridge/sources', body)
export const deleteDataBridgeSource = (id: string) =>
  apiFetchDelete<{ source_id: string; deleted: boolean }>(`/api/v1/atlas/databridge/sources/${encodeURIComponent(id)}`)
export const discoverDataBridgeSource = (id: string) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/databridge/sources/${encodeURIComponent(id)}/discover`)

// ── Migration plans ──

export interface MigrationPlan {
  id: string
  tenant_id: string
  name: string
  source_id: string
  state: string
  readiness_score: number
  assessment?: Record<string, unknown>
  edge_cluster_id?: string | null
  cdc_stream_id?: string | null
  cutover_at?: string | null
  rollback_window_secs: number
  created_at?: string | null
}

export interface CreateMigrationPlanRequest {
  name: string
  source_id: string
  rollback_window_secs?: number
}

export const listMigrationPlans = () => apiGet<MigrationPlan[]>('/api/v1/atlas/databridge/plans')
export const getMigrationPlan = (id: string) => apiGet<MigrationPlan>(`/api/v1/atlas/databridge/plans/${encodeURIComponent(id)}`)
export const createMigrationPlan = (body: CreateMigrationPlanRequest) =>
  apiPost<MigrationPlan>('/api/v1/atlas/databridge/plans', body)
export const deleteMigrationPlan = (id: string) =>
  apiFetchDelete<{ plan_id: string; deleted: boolean }>(`/api/v1/atlas/databridge/plans/${encodeURIComponent(id)}`)

const planStage = (id: string, stage: string) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/databridge/plans/${encodeURIComponent(id)}/${stage}`)

export const assessMigrationPlan = (id: string) => planStage(id, 'assess')
export const provisionMigrationEdge = (id: string) => planStage(id, 'provision')
export const fullLoadMigrationPlan = (id: string) => planStage(id, 'full-load')
export const startMigrationCdc = (id: string) => planStage(id, 'cdc/start')
export const stopMigrationCdc = (id: string) => planStage(id, 'cdc/stop')
export const restartMigrationCdc = (id: string) => planStage(id, 'cdc/restart')
export const validateMigrationPlan = (id: string) => planStage(id, 'validate')
/** Guarded on Atlas's side: plan must be `validated`, last validation must
    have `passed`, and (if CDC is attached) lag must be under Atlas's 10s
    threshold -- Atlas rejects with a specific 409 naming which precondition
    failed, not a generic error. */
export const cutoverMigrationPlan = (id: string) => planStage(id, 'cutover')
/** Guarded on Atlas's side: only within the cutover's rollback window. */
export const rollbackMigrationPlan = (id: string) => planStage(id, 'rollback')

// ── Edge clusters (read-only + delete) ──

export interface EdgeDbCluster {
  id: string
  plan_id: string
  engine: string
  operator: string
  namespace: string
  cr_name: string
  instances: number
  storage_class: string
  service_endpoint?: string | null
  state: string
  size_bytes: number
}

export const listEdgeClusters = () => apiGet<EdgeDbCluster[]>('/api/v1/atlas/databridge/edge-clusters')
export const deleteEdgeCluster = (id: string) =>
  apiFetchDelete<{ edge_cluster_id: string; deleted: boolean }>(`/api/v1/atlas/databridge/edge-clusters/${encodeURIComponent(id)}`)

// ── CDC streams (read-only) ──

export interface CdcStream {
  id: string
  plan_id: string
  engine: string
  state: string
  lag_seconds: number
  lag_bytes: number
  events_total: number
  restart_count: number
}

export const listCdcStreams = () => apiGet<CdcStream[]>('/api/v1/atlas/databridge/cdc-streams')

// ── Object-store migrations (S3-protocol cloud -> Ceph RGW) ──

export interface ObjectMigration {
  id: string
  tenant_id: string
  name: string
  source_provider: string
  source_endpoint: string
  source_bucket: string
  dest_provider: string
  dest_endpoint: string
  dest_bucket: string
  mode: string
  state: string
  objects_done: number
  objects_total: number
  bytes_done: number
  bytes_total: number
  throughput_mbps: number
  last_error?: string | null
}

export interface CreateObjectMigrationRequest {
  name: string
  source_endpoint: string
  source_bucket: string
  dest_endpoint: string
  dest_bucket: string
  source_provider?: string
  dest_provider?: string
  mode?: 'full' | 'incremental'
}

export const listObjectMigrations = () => apiGet<ObjectMigration[]>('/api/v1/atlas/databridge/object')
export const createObjectMigration = (body: CreateObjectMigrationRequest) =>
  apiPost<ObjectMigration>('/api/v1/atlas/databridge/object', body)
/** Atlas returns `204 No Content` on success -- delete is unconditional
    (no rows-affected check), so a repeat delete of the same id also
    succeeds rather than 404ing. */
export const deleteObjectMigration = async (id: string): Promise<void> => {
  const res = await apiFetch(`/api/v1/atlas/databridge/object/${encodeURIComponent(id)}`, { method: 'DELETE' })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
}
export const startObjectMigration = (id: string) =>
  apiPost<AtlasJobEnvelope>(`/api/v1/atlas/databridge/object/${encodeURIComponent(id)}/start`)

// ── Validations & cutovers (read-only) ──

export interface MigrationValidation {
  id: string
  plan_id: string
  kind: string
  state: string
  tables_total: number
  tables_mismatched: number
  summary?: Record<string, unknown>
  completed_at?: string | null
}

export interface MigrationCutover {
  id: string
  plan_id: string
  from_endpoint: string
  to_endpoint: string
  state: string
  drain_deadline?: string | null
  rollback_deadline?: string | null
  completed_at?: string | null
}

export const listMigrationValidations = (planId?: string) =>
  apiGet<MigrationValidation[]>(`/api/v1/atlas/databridge/validations${planId ? `?plan_id=${encodeURIComponent(planId)}` : ''}`)
export const listMigrationCutovers = () => apiGet<MigrationCutover[]>('/api/v1/atlas/databridge/cutovers')
