// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { apiGet, apiPost } from './client'

const API_BASE = '/api'

/** Mirrors `crate::operations::Operation` (src/operations/mod.rs). */
export type OperationState = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled'

export interface Operation {
  id: string
  kind: string
  resource: string
  namespace: string
  state: OperationState
  phase: string
  progress: number
  params: unknown
  result?: unknown
  error?: string
  attempts: number
  max_attempts: number
  /** Restarts/lost owners survived; these do not count against max_attempts. */
  interruptions: number
  idempotency_key?: string
  owner?: string
  cancel_requested: boolean
  created: string
  updated: string
  completed?: string
}

export async function listOperations(kind?: string, limit = 50): Promise<Operation[]> {
  const q = new URLSearchParams({ limit: String(limit) })
  if (kind) q.set('kind', kind)
  const res = await apiGet<{ operations: Operation[] }>(`${API_BASE}/operations?${q}`)
  return res.operations
}

export async function getOperation(id: string): Promise<Operation> {
  return apiGet<Operation>(`${API_BASE}/operations/${encodeURIComponent(id)}`)
}

export async function cancelOperation(id: string): Promise<{ id: string; state: OperationState }> {
  return apiPost<{ id: string; state: OperationState }>(`${API_BASE}/operations/${encodeURIComponent(id)}/cancel`)
}

export const isTerminal = (s: OperationState) =>
  s === 'succeeded' || s === 'failed' || s === 'cancelled'
