// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost } from './client'

/** Mirrors the backend's real per-VM compliance scan
 * (src/api/http_server/web/compliance_handlers.rs), backed by
 * crate::security::compliance::ComplianceChecker inspecting actual VM
 * specs for PCI-DSS/HIPAA/SOC2 controls. Computed fresh on every request --
 * there is no stored scan history to go stale. */
export interface ComplianceCheck {
  id: string
  category: string
  name: string
  status: 'pass' | 'warning' | 'fail'
  description: string
  remediation?: string | null
}

export interface ComplianceSummary {
  score: number
  total: number
  passed: number
  warnings: number
  failed: number
  categories: string[]
  checks: ComplianceCheck[]
  last_scan: string
}

const API_BASE = '/api'

export async function getComplianceSummary(): Promise<ComplianceSummary> {
  return apiGet<ComplianceSummary>(`${API_BASE}/system/compliance`)
}

export async function rescan(): Promise<ComplianceSummary> {
  return apiPost<ComplianceSummary>(`${API_BASE}/system/compliance/scan`)
}
