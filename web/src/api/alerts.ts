// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost, apiDelete } from './client'

/** Mirrors src/api/http_server/web/alert_handlers.rs, backed by the real
 * crate::observability::alerts::AlertManager. A background loop evaluates
 * rules against real VM CPU/memory usage and real KubeVirt phase every
 * 60s -- only metric_threshold/resource_usage/vm_state conditions are
 * evaluated for real; error_rate/custom have no real data source here. */
export type AlertConditionType = 'metric_threshold' | 'resource_usage' | 'vm_state'
export type AlertOperator = 'gt' | 'lt' | 'eq' | 'gte' | 'lte'

export interface AlertRule {
  id: string
  name: string
  description: string
  severity: 'info' | 'warning' | 'critical'
  condition: Record<string, unknown>
  enabled: boolean
  created_at: string
}

export interface CreateAlertRuleRequest {
  name: string
  description?: string
  severity?: 'info' | 'warning' | 'critical'
  condition_type: AlertConditionType
  metric_or_resource?: string
  operator?: AlertOperator
  threshold?: number
  vm_name?: string
  state?: string
}

export interface Alert {
  id: string
  rule_id: string
  rule_name: string
  severity: 'info' | 'warning' | 'critical'
  state: 'Pending' | 'Firing' | 'Resolved' | 'Silenced'
  message: string
  labels: Record<string, string>
  started_at: string
  resolved_at?: string | null
}

const API_BASE = '/api'

export async function listActiveAlerts(): Promise<Alert[]> {
  return apiGet<Alert[]>(`${API_BASE}/alerts`)
}

export async function listAlertRules(): Promise<AlertRule[]> {
  return apiGet<AlertRule[]>(`${API_BASE}/alerts/rules`)
}

export async function createAlertRule(req: CreateAlertRuleRequest): Promise<AlertRule> {
  return apiPost<AlertRule>(`${API_BASE}/alerts/rules`, req)
}

export async function deleteAlertRule(id: string): Promise<void> {
  return apiDelete(`${API_BASE}/alerts/rules/${encodeURIComponent(id)}`)
}

export async function resolveAlert(id: string): Promise<void> {
  await apiPost(`${API_BASE}/alerts/${encodeURIComponent(id)}/resolve`)
}

export async function silenceAlert(id: string): Promise<void> {
  await apiPost(`${API_BASE}/alerts/${encodeURIComponent(id)}/silence`)
}
