// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet, apiPost, apiDelete } from './client'

/** Mirrors src/api/http_server/web/power_schedule_handlers.rs, backed by
 * the real, disk-persisted crate::power_schedule::PowerScheduleManager --
 * a background loop actually starts/stops/restarts the VM when due. */
export interface PowerSchedule {
  name: string
  vm_name: string
  action: 'start' | 'stop' | 'restart'
  schedule_type: 'hourly' | 'daily' | 'weekly' | 'monthly' | 'cron'
  enabled: boolean
  last_run?: string | null
  next_run?: string | null
}

export interface CreatePowerScheduleRequest {
  name: string
  vm_name: string
  action: 'start' | 'stop' | 'restart'
  schedule_type: 'hourly' | 'daily' | 'weekly' | 'monthly'
  hour?: number
  minute?: number
  weekday?: string
  day_of_month?: number
  enabled?: boolean
}

const API_BASE = '/api'

export async function listPowerSchedules(): Promise<PowerSchedule[]> {
  return apiGet<PowerSchedule[]>(`${API_BASE}/schedules/power`)
}

export async function createPowerSchedule(req: CreatePowerScheduleRequest): Promise<PowerSchedule> {
  return apiPost<PowerSchedule>(`${API_BASE}/schedules/power`, req)
}

export async function deletePowerSchedule(name: string): Promise<void> {
  return apiDelete(`${API_BASE}/schedules/power/${encodeURIComponent(name)}`)
}

export async function enablePowerSchedule(name: string): Promise<PowerSchedule> {
  return apiPost<PowerSchedule>(`${API_BASE}/schedules/power/${encodeURIComponent(name)}/enable`)
}

export async function disablePowerSchedule(name: string): Promise<PowerSchedule> {
  return apiPost<PowerSchedule>(`${API_BASE}/schedules/power/${encodeURIComponent(name)}/disable`)
}
