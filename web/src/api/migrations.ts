// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet, apiPost, apiPostVoid } from './client'

const API_BASE = '/api'

/** KubeVirt VirtualMachineInstanceMigration phases. */
export type MigrationPhase =
  | 'Pending'
  | 'Scheduling'
  | 'Scheduled'
  | 'PreparingTarget'
  | 'TargetReady'
  | 'Running'
  | 'Succeeded'
  | 'Failed'
  | 'Unknown'

export interface MigrationStatus {
  id: string
  vm_name: string
  phase: MigrationPhase
  source_node?: string
  target_node?: string
  start_timestamp?: string
  end_timestamp?: string
  completed: boolean
  failed: boolean
  created?: string
}

/** Start a live migration for a VM. KubeVirt's scheduler selects the target
 * node automatically — there is no user-supplied host or URI. */
export async function startMigration(vmName: string): Promise<MigrationStatus> {
  return apiPost<MigrationStatus>(`${API_BASE}/vms/${encodeURIComponent(vmName)}/migrate`)
}

export async function listVmMigrations(vmName: string): Promise<MigrationStatus[]> {
  return apiGet<MigrationStatus[]>(`${API_BASE}/vms/${encodeURIComponent(vmName)}/migrations`)
}

export async function getMigration(id: string): Promise<MigrationStatus> {
  return apiGet<MigrationStatus>(`${API_BASE}/migrations/${encodeURIComponent(id)}`)
}

export async function cancelMigration(id: string): Promise<void> {
  await apiPostVoid(`${API_BASE}/migrations/${encodeURIComponent(id)}/cancel`)
}

/** Mirrors crate::migration::assistant::{CheckType, CheckStatus, CheckSeverity, PreCheckResult} exactly (unit enums serialize as their variant name). */
export type ReadinessCheckType = 'Resource' | 'Network' | 'Storage' | 'Compatibility' | 'Health'
export type ReadinessCheckStatus = 'Passed' | 'Warning' | 'Failed' | 'Skipped'
export type ReadinessCheckSeverity = 'Required' | 'Recommended' | 'Optional'

export interface ReadinessCheck {
  check_name: string
  check_type: ReadinessCheckType
  status: ReadinessCheckStatus
  severity: ReadinessCheckSeverity
  message: string
}

/** Runs pre-flight migration checks. Omit `vm` to check every VM in the
 * server's namespace; `targetNode` is optional (KubeVirt's scheduler picks
 * a node automatically when starting a migration for real). */
export async function getMigrationReadiness(vm?: string, targetNode?: string): Promise<ReadinessCheck[]> {
  const params = new URLSearchParams()
  if (vm) params.set('vm', vm)
  if (targetNode) params.set('target_node', targetNode)
  const qs = params.toString()
  const { checks } = await apiGet<{ checks: ReadinessCheck[] }>(`${API_BASE}/migrations/readiness${qs ? `?${qs}` : ''}`)
  return checks
}
