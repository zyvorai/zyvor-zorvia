// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost, apiDelete } from './client'

/** Mirrors src/api/http_server/web/warm_pool_handlers.rs, backed by the
 * real, disk-persisted crate::warm_pool::WarmPoolManager. "Warm" means the
 * VM object already exists (built from the real template catalog) but is
 * left stopped -- claiming it starts it for real. Saves create-time work,
 * not KubeVirt boot time -- not a hot/running standby pool. */
export interface WarmPoolMember {
  vm_name: string
  status: 'provisioning' | 'ready' | 'claimed' | 'failed'
  created_at: string
  claimed_at?: string | null
  error?: string | null
}

export interface WarmPool {
  name: string
  template: string
  size: number
  ready_count: number
  members: WarmPoolMember[]
}

export interface CreateWarmPoolRequest {
  name: string
  template: string
  size: number
}

const API_BASE = '/api'

export async function listWarmPools(): Promise<WarmPool[]> {
  return apiGet<WarmPool[]>(`${API_BASE}/warm-pools`)
}

export async function createWarmPool(req: CreateWarmPoolRequest): Promise<WarmPool> {
  return apiPost<WarmPool>(`${API_BASE}/warm-pools`, req)
}

export async function deleteWarmPool(name: string): Promise<void> {
  return apiDelete(`${API_BASE}/warm-pools/${encodeURIComponent(name)}`)
}

export async function claimWarmPoolMember(name: string): Promise<{ vm_name: string; pool: string }> {
  return apiPost(`${API_BASE}/warm-pools/${encodeURIComponent(name)}/claim`)
}
