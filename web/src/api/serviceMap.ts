// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet } from './client'

/** Mirrors the backend's ServiceMapEntry (src/api/http_server/web/service_map_handlers.rs)
 * -- a projection of the real NodePort/ClusterIP Services crate::kube::expose
 * creates per VM, grouped fleet-wide instead of one VM at a time. */
export interface ServiceMapEntry {
  vm_name: string
  service_name: string
  guest_port: number
  host_port: number
  node_port?: number | null
  protocol: string
  expose_host: string
  cluster_ip?: string | null
}

const API_BASE = '/api'

export async function getServiceMap(): Promise<ServiceMapEntry[]> {
  return apiGet<ServiceMapEntry[]>(`${API_BASE}/services/map`)
}
