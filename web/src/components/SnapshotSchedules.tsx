// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useState } from 'react'
import { AlertCircle, CalendarClock, Loader2, Plus } from 'lucide-react'
import {
  createSnapshotSchedule,
  deleteSnapshotSchedule,
  listVMSnapshotSchedules,
  setSnapshotScheduleEnabled,
  type SnapshotSchedule,
  type SnapshotScheduleFrequency,
} from '../api/snapshots'
import { useToastContext } from '../contexts/ToastContext'
import { usePermissions } from '../hooks/usePermissions'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { formatRelativeTime } from '../utils/format'
import {
  WEEKDAYS,
  affectsOtherVMs,
  buildScheduleRequest,
  retentionSummary,
  scheduleScope,
  validateScheduleName,
} from '../utils/snapshotSchedule'
import ConfirmDialog from './ConfirmDialog'

const FREQUENCIES: SnapshotScheduleFrequency[] = ['hourly', 'daily', 'weekly', 'monthly']

/** Scheduled snapshots for one VM: lists the schedules that apply to it and, for admins, manages them. */
export default function SnapshotSchedules({ vmName, onChanged }: { vmName: string; onChanged?: () => void }) {
  const toast = useToastContext()
  const { canAdmin } = usePermissions()
  const [schedules, setSchedules] = useState<SnapshotSchedule[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showForm, setShowForm] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<SnapshotSchedule | null>(null)

  const [name, setName] = useState('')
  const [frequency, setFrequency] = useState<SnapshotScheduleFrequency>('daily')
  const [hour, setHour] = useState(2)
  const [minute, setMinute] = useState(0)
  const [weekday, setWeekday] = useState<string>('sunday')
  const [dayOfMonth, setDayOfMonth] = useState(1)
  const [keepLast, setKeepLast] = useState(7)

  const load = useCallback(async () => {
    try {
      setSchedules(await listVMSnapshotSchedules(vmName))
      setError(null)
    } catch (err) {
      setError(formatUserError(err))
    } finally {
      setLoading(false)
    }
  }, [vmName])

  useEffect(() => {
    load()
  }, [load])

  const nameError = name ? validateScheduleName(name) : null

  const handleCreate = async () => {
    const invalid = validateScheduleName(name)
    if (invalid) return
    setBusy('create')
    try {
      await createSnapshotSchedule(
        buildScheduleRequest(vmName, { name, frequency, hour, minute, weekday, dayOfMonth, keepLast }),
      )
      toast.success(`Schedule '${name}' created`)
      setShowForm(false)
      setName('')
      await load()
    } catch (err) {
      toastFailure(toast, 'Failed to create schedule', err)
    } finally {
      setBusy(null)
    }
  }

  const handleToggle = async (s: SnapshotSchedule) => {
    setBusy(s.id)
    try {
      await setSnapshotScheduleEnabled(s.name, !s.enabled)
      await load()
    } catch (err) {
      toastFailure(toast, `Failed to ${s.enabled ? 'pause' : 'resume'} schedule`, err)
    } finally {
      setBusy(null)
    }
  }

  const handleDelete = async (s: SnapshotSchedule) => {
    setPendingDelete(null)
    setBusy(s.id)
    try {
      await deleteSnapshotSchedule(s.name)
      toast.success(`Schedule '${s.name}' deleted`)
      await load()
      onChanged?.()
    } catch (err) {
      toastFailure(toast, 'Failed to delete schedule', err)
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="zf-panel p-5 space-y-3" aria-label="Scheduled snapshots">
      <div className="flex items-center justify-between">
        <h3 className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)]">
          <CalendarClock className="w-4 h-4 text-[var(--zf-muted)]" />
          Scheduled snapshots
        </h3>
        {canAdmin && !showForm && (
          <button onClick={() => setShowForm(true)} className="zf-btn zf-btn-soft zf-btn-sm">
            <Plus className="w-3.5 h-3.5" />
            Add schedule
          </button>
        )}
      </div>

      {showForm && (
        <div className="space-y-3 border border-[var(--zf-hairline)] rounded-lg p-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <div>
              <label htmlFor="ss-name" className="block text-xs text-[var(--zf-muted)] mb-1">Name</label>
              <input
                id="ss-name"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="nightly"
                className="input-field py-1.5"
                aria-invalid={nameError ? true : undefined}
              />
              {nameError && <p className="text-xs text-[var(--zf-danger)] mt-1">{nameError}</p>}
            </div>
            <div>
              <label htmlFor="ss-freq" className="block text-xs text-[var(--zf-muted)] mb-1">Frequency</label>
              <select
                id="ss-freq"
                value={frequency}
                onChange={(e) => setFrequency(e.target.value as SnapshotScheduleFrequency)}
                className="input-field py-1.5"
              >
                {FREQUENCIES.map((f) => (
                  <option key={f} value={f}>{f[0].toUpperCase() + f.slice(1)}</option>
                ))}
              </select>
            </div>
            {frequency === 'weekly' && (
              <div>
                <label htmlFor="ss-weekday" className="block text-xs text-[var(--zf-muted)] mb-1">Day of week</label>
                <select id="ss-weekday" value={weekday} onChange={(e) => setWeekday(e.target.value)} className="input-field py-1.5">
                  {WEEKDAYS.map((d) => (
                    <option key={d} value={d}>{d[0].toUpperCase() + d.slice(1)}</option>
                  ))}
                </select>
              </div>
            )}
            {frequency === 'monthly' && (
              <div>
                <label htmlFor="ss-dom" className="block text-xs text-[var(--zf-muted)] mb-1">Day of month</label>
                <input id="ss-dom" type="number" min={1} max={31} value={dayOfMonth}
                  onChange={(e) => setDayOfMonth(Math.min(31, Math.max(1, Number(e.target.value) || 1)))}
                  className="input-field py-1.5" />
              </div>
            )}
            {frequency !== 'hourly' && (
              <div>
                <label htmlFor="ss-hour" className="block text-xs text-[var(--zf-muted)] mb-1">Hour (UTC, 0-23)</label>
                <input id="ss-hour" type="number" min={0} max={23} value={hour}
                  onChange={(e) => setHour(Math.min(23, Math.max(0, Number(e.target.value) || 0)))}
                  className="input-field py-1.5" />
              </div>
            )}
            <div>
              <label htmlFor="ss-min" className="block text-xs text-[var(--zf-muted)] mb-1">Minute (0-59)</label>
              <input id="ss-min" type="number" min={0} max={59} value={minute}
                onChange={(e) => setMinute(Math.min(59, Math.max(0, Number(e.target.value) || 0)))}
                className="input-field py-1.5" />
            </div>
            <div>
              <label htmlFor="ss-keep" className="block text-xs text-[var(--zf-muted)] mb-1">Keep the last</label>
              <input id="ss-keep" type="number" min={1} max={100} value={keepLast}
                onChange={(e) => setKeepLast(Math.min(100, Math.max(1, Number(e.target.value) || 1)))}
                className="input-field py-1.5" />
            </div>
          </div>
          <p className="text-xs text-[var(--zf-muted)]">
            Only snapshots this schedule creates are pruned; snapshots you take by hand are never deleted.
          </p>
          <div className="flex justify-end gap-2">
            <button onClick={() => { setShowForm(false); setName('') }} className="zf-btn zf-btn-ghost zf-btn-sm">
              Cancel
            </button>
            <button
              onClick={handleCreate}
              disabled={busy === 'create' || !name || !!nameError}
              className="zf-btn zf-btn-primary zf-btn-sm"
            >
              {busy === 'create' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              Create
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <p className="text-sm text-[var(--zf-muted)]">Loading schedules...</p>
      ) : error ? (
        <p className="flex items-center gap-2 text-sm text-[var(--zf-danger)]">
          <AlertCircle className="w-4 h-4" />
          {error}
        </p>
      ) : schedules.length === 0 ? (
        <p className="text-sm text-[var(--zf-muted)]">
          No schedules. {canAdmin ? 'Add one to snapshot this VM automatically.' : 'An admin can add one.'}
        </p>
      ) : (
        <ul className="divide-y divide-[var(--zf-hairline)]">
          {schedules.map((s) => (
            <li key={s.id} className="py-3 flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-[var(--zf-ink)]">{s.name}</span>
                  <span className="px-2 py-0.5 text-[11px] rounded text-[var(--zf-muted)] bg-[var(--zf-canvas)] border border-[var(--zf-hairline)]">
                    {s.schedule_text ?? s.schedule_type}
                  </span>
                  <span className="px-2 py-0.5 text-[11px] rounded text-[var(--zf-muted)] bg-[var(--zf-canvas)] border border-[var(--zf-hairline)]">
                    {scheduleScope(s)}
                  </span>
                  {!s.enabled && <span className="text-[11px] text-[var(--zf-muted)]">paused</span>}
                </div>
                <div className="text-xs text-[var(--zf-muted)] mt-1">
                  {retentionSummary(s)}
                  {s.enabled && s.next_run && <> · next {formatRelativeTime(s.next_run)}</>}
                  {s.last_run && <> · last run {formatRelativeTime(s.last_run)}</>}
                </div>
                {s.last_error && (
                  <div className="text-xs text-[var(--zf-danger)] mt-1 break-words">Last run failed: {s.last_error}</div>
                )}
              </div>
              {canAdmin && (
                <div className="flex gap-2">
                  <button onClick={() => handleToggle(s)} disabled={busy === s.id} className="zf-btn zf-btn-soft zf-btn-xs">
                    {s.enabled ? 'Pause' : 'Resume'}
                  </button>
                  <button
                    onClick={() => setPendingDelete(s)}
                    disabled={busy === s.id}
                    className="px-2.5 py-1 bg-[var(--zf-danger)]/10 text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/20 disabled:opacity-50 rounded text-xs font-medium transition-colors"
                  >
                    Delete
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {pendingDelete && (
        <ConfirmDialog
          title={`Delete schedule '${pendingDelete.name}'?`}
          message={
            affectsOtherVMs(pendingDelete, vmName)
              ? `This schedule also covers ${pendingDelete.vm_names.length === 0 ? 'every VM' : 'other VMs'}, so they stop being snapshotted too. Existing snapshots are kept.`
              : 'Existing snapshots are kept.'
          }
          confirmLabel="Delete"
          variant="danger"
          onConfirm={() => handleDelete(pendingDelete)}
          onCancel={() => setPendingDelete(null)}
        />
      )}
    </section>
  )
}
