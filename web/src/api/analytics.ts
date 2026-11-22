// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet } from './client'

/** Mirrors src/api/http_server/web/analytics_handlers.rs -- the same real
 * per-VM usage numbers `getMetrics()` already reports one VM at a time,
 * ranked across the whole fleet. */
export interface TopVmEntry {
  vm_name: string
  cpu_usage: number
  memory_usage: number
  disk_usage: number
  source: string
}

const API_BASE = '/api'

export async function getTopVms(metric: 'cpu_usage' | 'memory_usage' | 'disk_usage', limit = 10): Promise<TopVmEntry[]> {
  return apiGet<TopVmEntry[]>(`${API_BASE}/analytics/top-vms?metric=${metric}&limit=${limit}`)
}
