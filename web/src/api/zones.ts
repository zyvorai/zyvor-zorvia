// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet } from './client'

/** Mirrors src/api/http_server/web/zone_handlers.rs -- real Nodes grouped
 * by the standard `topology.kubernetes.io/zone` label. Not a multi-cluster
 * or multi-datacenter concept; this is a single-cluster platform. */
export interface Zone {
  zone: string
  node_count: number
  allocatable_cpu: number
  allocatable_memory_gib: number
  vm_count: number
}

const API_BASE = '/api'

export async function listZones(): Promise<Zone[]> {
  return apiGet<Zone[]>(`${API_BASE}/zones`)
}
