// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import type { CreateSnapshotScheduleRequest, SnapshotSchedule, SnapshotScheduleFrequency } from '../api/snapshots'

/** Mirrors the backend rule: lowercase letters, digits and '-', 1-40 chars, alphanumeric at both ends. */
const NAME_RE = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/
export const MAX_SCHEDULE_NAME_LEN = 40

export const WEEKDAYS = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'] as const

export function validateScheduleName(name: string): string | null {
  if (!name) return 'Name is required'
  if (name.length > MAX_SCHEDULE_NAME_LEN) return `Name must be ${MAX_SCHEDULE_NAME_LEN} characters or fewer`
  if (!NAME_RE.test(name)) return "Use lowercase letters, digits and '-', starting and ending with a letter or digit"
  return null
}

export interface ScheduleFormValues {
  name: string
  frequency: SnapshotScheduleFrequency
  hour: number
  minute: number
  weekday: string
  dayOfMonth: number
  keepLast: number
}

/** Builds the create request for one VM. Never produces an all-VM schedule. */
export function buildScheduleRequest(vmName: string, v: ScheduleFormValues): CreateSnapshotScheduleRequest {
  const req: CreateSnapshotScheduleRequest = {
    name: v.name,
    schedule_type: v.frequency,
    vm_names: [vmName],
    keep_last_n: v.keepLast,
    max_snapshots: Math.max(v.keepLast, 1),
  }
  if (v.frequency !== 'hourly') req.hour = v.hour
  req.minute = v.minute
  if (v.frequency === 'weekly') req.weekday = v.weekday
  if (v.frequency === 'monthly') req.day_of_month = v.dayOfMonth
  return req
}

export function scheduleScope(s: SnapshotSchedule): string {
  if (s.vm_names.length === 0) return 'All VMs'
  if (s.vm_names.length === 1) return s.vm_names[0]
  return `${s.vm_names.length} VMs`
}

/** True when deleting or disabling the schedule would also affect VMs other than `vmName`. */
export function affectsOtherVMs(s: SnapshotSchedule, vmName: string): boolean {
  return s.vm_names.length === 0 || s.vm_names.some((n) => n !== vmName)
}

export function retentionSummary(s: SnapshotSchedule): string {
  const { keep_last_n, max_snapshots, max_age_days } = s.retention
  const parts: string[] = []
  if (keep_last_n != null) parts.push(`keep last ${keep_last_n}`)
  if (max_snapshots != null) parts.push(`max ${max_snapshots}`)
  if (max_age_days != null) parts.push(`${max_age_days} days`)
  return parts.length ? parts.join(' · ') : 'no limit'
}
