// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { featureJson } from './experimental'

/** DR replication pairs (Experimental; ZORVIA_EXPERIMENTAL=1 plus
    ZORVIA_FEATURE_DR_REPLICATION=1, cluster.admin). See docs/DR_REPLICATION.md.
    Every response is wrapped as `{ success, data }`. */
const BASE = '/api/v1/enterprise/dr'

export type DrHealth =
  | 'healthy'
  | 'rpo_breached'
  | 'no_backup_yet'
  | 'failing'
  | 'paused'
  | 'failed_over'

export interface DrPairStatus {
  name: string
  vm: string
  namespace: string
  dr_vm: string
  dr_namespace: string
  phase: string
  enabled: boolean
  health: DrHealth
  rpo_seconds: number
  lag_seconds: number
  rpo_met: boolean
  last_success: string | null
  last_attempt: string | null
  next_due: string | null
  in_flight_op: string | null
  last_error: string | null
  consecutive_failures: number
  rto_target_seconds: number
  rto_measured_seconds: number | null
  rto_met: boolean | null
}

export interface CreateDrPair {
  name: string
  vm: string
  namespace?: string
  dr_namespace?: string
  rpo_seconds: number
  rto_seconds?: number
}

export interface FailoverPlan {
  pair: string
  source: string
  source_state: string
  dr_target: string
  backup_op: string | null
  backup_age_seconds: number | null
  potential_data_loss_seconds: number
  rpo_met: boolean
  rto_target_seconds: number
  steps: string[]
  warnings: string[]
  blockers: string[]
  allowed: boolean
  confirm_phrase: string
}

export interface FailoverResult {
  dry_run: boolean
  restore_operation_id?: string
  plan: FailoverPlan
}

interface Envelope<T> {
  success: boolean
  data: T
}

const pairUrl = (name: string) => `${BASE}/pairs/${encodeURIComponent(name)}`

export async function listDrPairs(): Promise<DrPairStatus[]> {
  return (await featureJson<Envelope<DrPairStatus[]>>(`${BASE}/pairs`)).data
}

export async function createDrPair(req: CreateDrPair): Promise<DrPairStatus> {
  return (await featureJson<Envelope<DrPairStatus>>(`${BASE}/pairs`, 'POST', req)).data
}

export async function deleteDrPair(name: string): Promise<void> {
  await featureJson<unknown>(pairUrl(name), 'DELETE')
}

export async function getFailoverPlan(name: string): Promise<FailoverPlan> {
  return (await featureJson<Envelope<FailoverPlan>>(`${pairUrl(name)}/failover/plan`)).data
}

/** dry_run=true never changes anything; a real run needs `confirm` equal to
    the plan's confirm_phrase ("failover <pair>"). */
export async function failoverDrPair(
  name: string,
  opts: { dryRun: boolean; confirm?: string; start?: boolean },
): Promise<FailoverResult> {
  const body: Record<string, unknown> = { dry_run: opts.dryRun }
  if (opts.confirm !== undefined) body.confirm = opts.confirm
  if (opts.start !== undefined) body.start = opts.start
  return (await featureJson<Envelope<FailoverResult>>(`${pairUrl(name)}/failover`, 'POST', body)).data
}
