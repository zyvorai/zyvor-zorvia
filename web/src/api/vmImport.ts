// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost } from './client'
import type { OperationState } from './operations'

const API_BASE = '/api'

/** Mirrors `crate::migration_import` (src/migration_import/mod.rs). */
export interface VsphereSource {
  vcenter: string
  port?: number
  datacenter?: string
  insecure?: boolean
  /** Secret in the target namespace holding the vCenter credentials. */
  secret_name: string
  username_key?: string
  password_key?: string
}

export interface ImportNetwork {
  name: string
  /** Source port-group label for the operation record; supplied by the operator. */
  source_network?: string
  /** Existing attachment in the target namespace; omitted means pod networking. */
  attachment?: string
  mac_address?: string
}

export interface ImportVm {
  source_vm: string
  /** Defaults to a DNS-label form of source_vm. */
  target_vm_name?: string
  storage_class?: string
  pvc_size?: string
  cpu?: number
  memory?: string
  /** Start the VM once deployed (cutover). Default: leave it stopped. */
  start?: boolean
  /** Replace target NICs while stopped, before cutover. Empty keeps importer defaults. */
  networks?: ImportNetwork[]
  require_guest_agent?: boolean
  /** 30..3600 seconds. */
  boot_timeout_secs?: number
}

export interface ImportRequest {
  source: VsphereSource
  namespace: string
  vms: ImportVm[]
  scratch_size?: string
  scratch_storage_class?: string
  service_account?: string
}

export interface PreflightResult {
  ready: boolean
  blockers: string[]
  warnings: string[]
}

export interface QueuedImport {
  operation_id: string
  source_vm: string
  target_vm_name: string | null
}

export interface CreateImportResponse {
  wave_id: string
  warnings: string[]
  operations: QueuedImport[]
}

export interface ImportSummary {
  operation_id: string
  wave_id: string | null
  source_vm: string
  target_vm_name: string | null
  namespace: string
  state: OperationState
  phase: string
  progress: number
  error: string | null
  created: string
  completed: string | null
}

export interface WaveSummary {
  wave_id: string
  total: number
  queued: number
  running: number
  succeeded: number
  failed: number
  cancelled: number
  operations: {
    id: string
    source_vm: string
    target_vm_name: string | null
    state: OperationState
    phase: string
    progress: number
    error: string | null
  }[]
}

export async function preflightImport(req: ImportRequest): Promise<PreflightResult> {
  return apiPost<PreflightResult>(`${API_BASE}/vm-imports/preflight`, req)
}

/** Rejects with the server's 409 body (`blockers`) when preflight fails. */
export async function createImportWave(req: ImportRequest): Promise<CreateImportResponse> {
  return apiPost<CreateImportResponse>(`${API_BASE}/vm-imports`, req)
}

export async function listImports(): Promise<ImportSummary[]> {
  const res = await apiGet<{ imports: ImportSummary[] }>(`${API_BASE}/vm-imports`)
  return res.imports
}

export async function getImportWave(waveId: string): Promise<WaveSummary> {
  return apiGet<WaveSummary>(`${API_BASE}/vm-imports/waves/${waveId}`)
}
