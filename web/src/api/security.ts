// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet } from './client'

/** Mirrors the backend's real security-dashboard aggregation
 * (src/api/http_server/web/security_dashboard_handlers.rs) -- every field
 * is sourced from real data this project shipped elsewhere (real exposed
 * Services, the real audit trail), except `risk_score`, which is an
 * explicit heuristic over those two signals, not a certified metric. */
export interface SecurityAlert {
  severity: 'critical' | 'warning' | 'info'
  message: string
  source: string
  timestamp: string
}

export interface FailedLogin {
  timestamp: string
  user: string
}

export interface ListeningPort {
  port: number | null
  protocol: string
  service_name?: string
  vm_name?: string | null
}

export interface SecuritySummary {
  risk_score: number
  alerts: SecurityAlert[]
  failed_logins: FailedLogin[]
  listening_ports: ListeningPort[]
}

const API_BASE = '/api'

export async function getSecuritySummary(): Promise<SecuritySummary> {
  return apiGet<SecuritySummary>(`${API_BASE}/system/security`)
}
