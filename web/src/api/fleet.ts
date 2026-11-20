// Copyright 2026 Zyvor AI Labs · SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0
import { apiGet } from './client'

export interface FleetVm {
  namespace: string
  name: string
  status: string
  ready: boolean | null
}
export interface FleetCluster {
  name: string
  namespaces: string[]
  environment: string
  region: string
  observed_at: string
  health: 'healthy' | 'degraded' | 'unknown'
  node_count: number | null
  ready_nodes: number | null
  vm_count: number | null
  ready_vms: number | null
  vms: FleetVm[] | null
  issues: string[]
}
export interface FleetSnapshot {
  observed_at: string
  clusters: FleetCluster[]
  totals: {
    configured_clusters: number
    complete_clusters: number
    partial: boolean
    observed_vms: number
    observed_nodes: number
  }
}
export async function getFleet(): Promise<FleetSnapshot> {
  const response = await apiGet<{ success: boolean; data: FleetSnapshot }>('/api/v1/enterprise/fleet')
  return response.data
}
