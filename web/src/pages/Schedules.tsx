// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { Plus, Trash2, Power, PowerOff, Clock, Loader2 } from 'lucide-react'
import {
  listPowerSchedules,
  createPowerSchedule,
  deletePowerSchedule,
  enablePowerSchedule,
  disablePowerSchedule,
  PowerSchedule,
} from '../api/powerSchedules'
import { listVMs, VM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

const WEEKDAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday']

export default function Schedules() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [schedules, setSchedules] = useState<PowerSchedule[]>([])
  const [vms, setVms] = useState<VM[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [busyName, setBusyName] = useState<string | null>(null)

  const [name, setName] = useState('')
  const [vmName, setVmName] = useState('')
  const [action, setAction] = useState<'start' | 'stop' | 'restart'>('start')
  const [scheduleType, setScheduleType] = useState<'hourly' | 'daily' | 'weekly' | 'monthly'>('daily')
  const [hour, setHour] = useState('8')
  const [minute, setMinute] = useState('0')
  const [weekday, setWeekday] = useState('monday')
  const [dayOfMonth, setDayOfMonth] = useState('1')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState('')

  const fetchAll = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [scheduleList, vmList] = await Promise.all([listPowerSchedules(), listVMs()])
      setSchedules(scheduleList)
      setVms(vmList)
      if (!vmName && vmList[0]) setVmName(vmList[0].name)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load power schedules', err)
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { fetchAll() }, [fetchAll])

  const handleCreate = async () => {
    if (!name.trim()) { setCreateError('Name is required'); return }
    if (!vmName) { setCreateError('Select a VM'); return }
    setCreating(true)
    setCreateError('')
    try {
      await createPowerSchedule({
        name: name.trim(),
        vm_name: vmName,
        action,
        schedule_type: scheduleType,
        hour: parseInt(hour, 10) || 0,
        minute: parseInt(minute, 10) || 0,
        weekday: scheduleType === 'weekly' ? weekday : undefined,
        day_of_month: scheduleType === 'monthly' ? parseInt(dayOfMonth, 10) || 1 : undefined,
      })
      toast.success(`Schedule "${name}" created`)
      setName(''); setShowCreate(false)
      fetchAll()
    } catch (err) {
      setCreateError(formatUserError(err))
      toastFailure(toast, 'Failed to create schedule', err)
    } finally {
      setCreating(false)
    }
  }

  const handleToggle = async (schedule: PowerSchedule) => {
    setBusyName(schedule.name)
    try {
      if (schedule.enabled) await disablePowerSchedule(schedule.name)
      else await enablePowerSchedule(schedule.name)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to update schedule', err)
    } finally {
      setBusyName(null)
    }
  }

  const handleDelete = async (schedule: PowerSchedule) => {
    if (!await confirm('Delete Schedule', `Delete power schedule "${schedule.name}"?`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setBusyName(schedule.name)
    try {
      await deletePowerSchedule(schedule.name)
      setSchedules(prev => prev.filter(s => s.name !== schedule.name))
      toast.success('Schedule deleted')
    } catch (err) {
      toastFailure(toast, 'Failed to delete schedule', err)
    } finally {
      setBusyName(null)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="VM Power Schedules"
        description="Start, stop, or restart a VM on a recurring schedule — checked and run by the server every minute"
        onRefresh={fetchAll}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowCreate(!showCreate)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> New Schedule
          </button>
        }
      />

      {loadError && (
        <ErrorBanner title="Could not load schedules" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchAll} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading schedules…
        </div>
      ) : !loadError ? (
        <>
          {showCreate && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Schedule</h3>
              <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Name</label>
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="business-hours" className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">VM</label>
                  <select value={vmName} onChange={(e) => setVmName(e.target.value)} className="input-field text-sm w-full">
                    {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Action</label>
                  <select value={action} onChange={(e) => setAction(e.target.value as typeof action)} className="input-field text-sm w-full">
                    <option value="start">Start</option>
                    <option value="stop">Stop</option>
                    <option value="restart">Restart</option>
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Frequency</label>
                  <select value={scheduleType} onChange={(e) => setScheduleType(e.target.value as typeof scheduleType)} className="input-field text-sm w-full">
                    <option value="hourly">Hourly</option>
                    <option value="daily">Daily</option>
                    <option value="weekly">Weekly</option>
                    <option value="monthly">Monthly</option>
                  </select>
                </div>
                {scheduleType !== 'hourly' ? (
                  <div>
                    <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Time (UTC)</label>
                    <div className="flex gap-2">
                      <input type="number" min={0} max={23} value={hour} onChange={(e) => setHour(e.target.value)} className="input-field text-sm w-full" placeholder="HH" />
                      <input type="number" min={0} max={59} value={minute} onChange={(e) => setMinute(e.target.value)} className="input-field text-sm w-full" placeholder="MM" />
                    </div>
                  </div>
                ) : (
                  <div>
                    <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Minute</label>
                    <input type="number" min={0} max={59} value={minute} onChange={(e) => setMinute(e.target.value)} className="input-field text-sm w-full" />
                  </div>
                )}
                {scheduleType === 'weekly' && (
                  <div>
                    <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Weekday</label>
                    <select value={weekday} onChange={(e) => setWeekday(e.target.value)} className="input-field text-sm w-full capitalize">
                      {WEEKDAYS.map(d => <option key={d} value={d}>{d}</option>)}
                    </select>
                  </div>
                )}
                {scheduleType === 'monthly' && (
                  <div>
                    <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Day of month</label>
                    <input type="number" min={1} max={31} value={dayOfMonth} onChange={(e) => setDayOfMonth(e.target.value)} className="input-field text-sm w-full" />
                  </div>
                )}
              </div>
              {createError && <p className="text-sm text-[var(--zf-danger)]">{createError}</p>}
              <div className="flex gap-2">
                <button onClick={handleCreate} disabled={creating} className="zf-btn zf-btn-primary zf-btn-sm">
                  {creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                  {creating ? 'Creating...' : 'Create Schedule'}
                </button>
                <button onClick={() => { setShowCreate(false); setCreateError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <DataTable
              columns={[
                { key: 'name', header: 'Name', className: 'px-5', render: (s) => <span className="font-medium text-[var(--zf-ink)]">{s.name}</span> },
                { key: 'vm', header: 'VM', render: (s) => <span className="text-[var(--zf-muted)]">{s.vm_name}</span> },
                { key: 'action', header: 'Action', render: (s) => <span className="text-[var(--zf-muted)] capitalize">{s.action}</span> },
                { key: 'frequency', header: 'Frequency', render: (s) => <span className="text-[var(--zf-muted)] capitalize">{s.schedule_type}</span> },
                { key: 'next_run', header: 'Next Run', render: (s) => <span className="text-xs text-[var(--zf-muted)]">{s.next_run ? new Date(s.next_run).toLocaleString() : '—'}</span> },
                {
                  key: 'status',
                  header: 'Status',
                  render: (s) => (
                    <span className={`px-2.5 py-0.5 rounded-full text-xs font-medium border ${s.enabled ? 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25' : 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]'}`}>
                      {s.enabled ? 'Enabled' : 'Disabled'}
                    </span>
                  ),
                },
                {
                  key: 'actions',
                  header: 'Actions',
                  className: 'text-right',
                  render: (s) => (
                    <div className="flex items-center justify-end gap-1">
                      <button onClick={() => handleToggle(s)} disabled={busyName === s.name} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] hover:bg-[var(--zf-hover-tint)] rounded-lg transition-colors disabled:opacity-50" title={s.enabled ? 'Disable' : 'Enable'}>
                        {s.enabled ? <PowerOff className="w-4 h-4" /> : <Power className="w-4 h-4" />}
                      </button>
                      <button onClick={() => handleDelete(s)} disabled={busyName === s.name} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete">
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  ),
                },
              ]}
              rows={schedules}
              getRowKey={(s) => s.name}
              bordered={false}
              emptyState={<EmptyState icon={<Clock className="w-8 h-8" />} title="No power schedules" description="Create one to automatically start, stop, or restart a VM on a schedule." />}
            />
          </div>
        </>
      ) : null}

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel ?? 'Delete'}
          variant={confirmState.variant ?? 'danger'}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}
