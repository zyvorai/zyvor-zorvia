// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet } from './client'

/** Mirrors crate::placement::advisor -- a real scoring algorithm, fed real
 * Node/VM data by src/api/http_server/web/placement_handlers.rs.
 * Recommendation-only: nothing here moves a VM. Act on a suggestion via
 * the real live-migration route (`startMigration` in `./migrations`). */
export interface CandidateScore {
  node: string
  eligible: boolean
  score: number
  reasons: string[]
}

export interface PlacementRecommendation {
  workload: string
  selected_node: string | null
  candidates: CandidateScore[]
}

export interface RebalanceMove {
  workload: string
  namespace: string
  from_node: string
  to_node: string
  expected_improvement: number
  reason: string
}

const API_BASE = '/api'

export async function getPlacementRecommendation(vmName: string): Promise<PlacementRecommendation> {
  return apiGet<PlacementRecommendation>(`${API_BASE}/placement/${encodeURIComponent(vmName)}`)
}

export async function getRebalanceSuggestions(): Promise<RebalanceMove[]> {
  return apiGet<RebalanceMove[]>(`${API_BASE}/placement/rebalance`)
}
