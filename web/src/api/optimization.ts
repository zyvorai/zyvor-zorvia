// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet } from './client'

/** Mirrors src/api/http_server/web/resource_optimizer_handlers.rs, backed
 * by the real crate::cost::optimization::OptimizationEngine fed real VM
 * specs and real usage. `potential_savings` is estimated from allocated
 * CPU/memory at a configurable rate, not real billing data -- see
 * `cost_basis` on the response. */
export interface OptimizationRecommendation {
  id: string
  vm_name: string
  recommendation_type: 'RightSize' | 'Shutdown' | 'Schedule' | string
  priority: 'Low' | 'Medium' | 'High' | 'Critical'
  potential_savings: number
  savings_percent: number
  description: string
  action_items: string[]
  impact: 'Low' | 'Medium' | 'High'
  generated_at: string
}

export interface CostBasis {
  cpu_per_core_hour: number
  memory_per_gb_hour: number
  note: string
}

export interface OptimizationResult {
  recommendations: OptimizationRecommendation[]
  cost_basis: CostBasis
}

const API_BASE = '/api'

export async function getOptimizationRecommendations(): Promise<OptimizationResult> {
  return apiGet<OptimizationResult>(`${API_BASE}/optimization/recommendations`)
}
