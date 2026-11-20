// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet } from './client'

/** Mirrors src/api/http_server/web/capacity_handlers.rs -- built from the
 * same real Node/VM data Phase 5's Placement Advisor already computes. */
export interface CapacityNode {
  name: string
  allocatable_cpu: number
  allocatable_memory_gib: number
  used_cpu: number
  used_memory_gib: number
  vm_count: number
  unschedulable: boolean
}

export interface CapacityOverview {
  node_count: number
  vm_count: number
  total_cpu: number
  total_memory_gib: number
  used_cpu: number
  used_memory_gib: number
  free_cpu: number
  free_memory_gib: number
  nodes: CapacityNode[]
}

export interface CapacityFit {
  requested_cpu: number
  requested_memory_gib: number
  estimated_additional_vms: number
}

const API_BASE = '/api'

export async function getCapacityOverview(): Promise<CapacityOverview> {
  return apiGet<CapacityOverview>(`${API_BASE}/capacity/overview`)
}

export async function getCapacityFit(cpu: number, memoryGib: number): Promise<CapacityFit> {
  return apiGet<CapacityFit>(`${API_BASE}/capacity/fit?cpu=${cpu}&memory_gib=${memoryGib}`)
}
