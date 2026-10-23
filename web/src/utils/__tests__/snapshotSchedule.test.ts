// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import type { SnapshotSchedule } from '../../api/snapshots'
import {
  affectsOtherVMs,
  buildScheduleRequest,
  retentionSummary,
  scheduleScope,
  validateScheduleName,
} from '../snapshotSchedule'

const sched = (over: Partial<SnapshotSchedule> = {}): SnapshotSchedule => ({
  id: 'nightly',
  name: 'nightly',
  vm_names: ['web1'],
  schedule_type: 'daily',
  retention: { max_snapshots: 10, max_age_days: 30, keep_last_n: 5 },
  enabled: true,
  last_run: null,
  next_run: null,
  last_error: null,
  ...over,
})

const form = {
  name: 'nightly',
  frequency: 'daily' as const,
  hour: 2,
  minute: 30,
  weekday: 'sunday',
  dayOfMonth: 1,
  keepLast: 7,
}

describe('validateScheduleName', () => {
  it('accepts names the backend accepts', () => {
    expect(validateScheduleName('nightly')).toBeNull()
    expect(validateScheduleName('db-6h')).toBeNull()
  })
  it('rejects what the backend rejects', () => {
    expect(validateScheduleName('')).not.toBeNull()
    expect(validateScheduleName('Nightly')).not.toBeNull()
    expect(validateScheduleName('-x')).not.toBeNull()
    expect(validateScheduleName('x-')).not.toBeNull()
    expect(validateScheduleName('a b')).not.toBeNull()
    expect(validateScheduleName('a'.repeat(41))).not.toBeNull()
    expect(validateScheduleName('a'.repeat(40))).toBeNull()
  })
})

describe('buildScheduleRequest', () => {
  it('always targets only the given VM', () => {
    expect(buildScheduleRequest('web1', form).vm_names).toEqual(['web1'])
  })
  it('daily sends hour and minute but no weekday or day of month', () => {
    const r = buildScheduleRequest('web1', form)
    expect(r).toMatchObject({ schedule_type: 'daily', hour: 2, minute: 30, keep_last_n: 7, max_snapshots: 7 })
    expect(r.weekday).toBeUndefined()
    expect(r.day_of_month).toBeUndefined()
  })
  it('hourly omits the hour', () => {
    const r = buildScheduleRequest('web1', { ...form, frequency: 'hourly' })
    expect(r.hour).toBeUndefined()
    expect(r.minute).toBe(30)
  })
  it('weekly adds the weekday, monthly adds the day', () => {
    expect(buildScheduleRequest('web1', { ...form, frequency: 'weekly' }).weekday).toBe('sunday')
    expect(buildScheduleRequest('web1', { ...form, frequency: 'monthly', dayOfMonth: 15 }).day_of_month).toBe(15)
  })
  it('never sends a max below one', () => {
    expect(buildScheduleRequest('web1', { ...form, keepLast: 0 }).max_snapshots).toBe(1)
  })
})

describe('scope helpers', () => {
  it('labels the scope', () => {
    expect(scheduleScope(sched({ vm_names: [] }))).toBe('All VMs')
    expect(scheduleScope(sched())).toBe('web1')
    expect(scheduleScope(sched({ vm_names: ['a', 'b', 'c'] }))).toBe('3 VMs')
  })
  it('flags schedules shared with other VMs', () => {
    expect(affectsOtherVMs(sched(), 'web1')).toBe(false)
    expect(affectsOtherVMs(sched({ vm_names: [] }), 'web1')).toBe(true)
    expect(affectsOtherVMs(sched({ vm_names: ['web1', 'db1'] }), 'web1')).toBe(true)
  })
})

describe('retentionSummary', () => {
  it('lists the set limits', () => {
    expect(retentionSummary(sched())).toBe('keep last 5 · max 10 · 30 days')
  })
  it('says so when nothing is set', () => {
    expect(
      retentionSummary(sched({ retention: { max_snapshots: null, max_age_days: null, keep_last_n: null } })),
    ).toBe('no limit')
  })
})
