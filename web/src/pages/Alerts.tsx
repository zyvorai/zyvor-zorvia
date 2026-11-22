// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { Bell, Plus, Trash2, CheckCircle, BellOff, Loader2 } from 'lucide-react'
import {
  listActiveAlerts,
  listAlertRules,
  createAlertRule,
  deleteAlertRule,
  resolveAlert,
  silenceAlert,
  Alert,
  AlertRule,
  AlertConditionType,
} from '../api/alerts'
import { getAtlasStatus, listAtlasAlerts, ackAtlasAlert, silenceAtlasAlert, resolveAtlasAlert, AtlasAlert } from '../api/atlas'
import { listVMs, VM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function severityBadge(s: string): string {
  switch (s) {
    case 'critical': return 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'
    case 'warning': return 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25'
    default: return 'text-[var(--zf-link)] bg-[var(--zf-link)]/10 border-[var(--zf-link)]/25'
  }
}

export default function Alerts() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [tab, setTab] = useState<'active' | 'rules'>('active')
  const [alerts, setAlerts] = useState<Alert[]>([])
  const [atlasAlerts, setAtlasAlerts] = useState<AtlasAlert[]>([])
  const [rules, setRules] = useState<AlertRule[]>([])
  const [vms, setVms] = useState<VM[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)

  const [name, setName] = useState('')
  const [conditionType, setConditionType] = useState<AlertConditionType>('metric_threshold')
  const [metric, setMetric] = useState('cpu_usage')
  const [threshold, setThreshold] = useState('90')
  const [severity, setSeverity] = useState<'info' | 'warning' | 'critical'>('warning')
  const [vmName, setVmName] = useState('')
  const [vmState, setVmState] = useState('Failed')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState('')

  const fetchAll = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [alertList, ruleList, vmList] = await Promise.all([listActiveAlerts(), listAlertRules(), listVMs()])
      setAlerts(alertList)
      setRules(ruleList)
      setVms(vmList)
      if (!vmName && vmList[0]) setVmName(vmList[0].name)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load alerts', err)
    } finally {
      setLoading(false)
    }
    // Atlas is optional -- a disabled/unreachable integration degrades to an
    // empty list rather than surfacing as a page-level error.
    try {
      const status = await getAtlasStatus()
      if (status.enabled && status.connected) {
        setAtlasAlerts(await listAtlasAlerts('open'))
      } else {
        setAtlasAlerts([])
      }
    } catch {
      setAtlasAlerts([])
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { fetchAll() }, [fetchAll])

  const handleCreate = async () => {
    if (!name.trim()) { setCreateError('Name is required'); return }
    setCreating(true)
    setCreateError('')
    try {
      await createAlertRule({
        name: name.trim(),
        severity,
        condition_type: conditionType,
        metric_or_resource: conditionType !== 'vm_state' ? metric : undefined,
        threshold: conditionType !== 'vm_state' ? parseFloat(threshold) || 0 : undefined,
        vm_name: conditionType === 'vm_state' ? vmName : undefined,
        state: conditionType === 'vm_state' ? vmState : undefined,
      })
      toast.success(`Alert rule "${name}" created`)
      setName(''); setShowCreate(false)
      fetchAll()
    } catch (err) {
      setCreateError(formatUserError(err))
      toastFailure(toast, 'Failed to create alert rule', err)
    } finally {
      setCreating(false)
    }
  }

  const handleDeleteRule = async (rule: AlertRule) => {
    if (!await confirm('Delete Rule', `Delete alert rule "${rule.name}"?`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setBusyId(rule.id)
    try {
      await deleteAlertRule(rule.id)
      setRules(prev => prev.filter(r => r.id !== rule.id))
      toast.success('Rule deleted')
    } catch (err) {
      toastFailure(toast, 'Failed to delete rule', err)
    } finally {
      setBusyId(null)
    }
  }

  const handleResolve = async (alert: Alert) => {
    setBusyId(alert.id)
    try {
      await resolveAlert(alert.id)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to resolve alert', err)
    } finally {
      setBusyId(null)
    }
  }

  const handleSilence = async (alert: Alert) => {
    setBusyId(alert.id)
    try {
      await silenceAlert(alert.id)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to silence alert', err)
    } finally {
      setBusyId(null)
    }
  }

  const handleAtlasAck = async (alert: AtlasAlert) => {
    setBusyId(alert.id)
    try {
      await ackAtlasAlert(alert.id)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to acknowledge Atlas alert', err)
    } finally {
      setBusyId(null)
    }
  }

  const handleAtlasSilence = async (alert: AtlasAlert) => {
    setBusyId(alert.id)
    try {
      await silenceAtlasAlert(alert.id)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to silence Atlas alert', err)
    } finally {
      setBusyId(null)
    }
  }

  const handleAtlasResolve = async (alert: AtlasAlert) => {
    setBusyId(alert.id)
    try {
      await resolveAtlasAlert(alert.id)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to resolve Atlas alert', err)
    } finally {
      setBusyId(null)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Alerts"
        description="Rules evaluated every minute against real VM CPU/memory usage and real KubeVirt phase"
        onRefresh={fetchAll}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowCreate(!showCreate)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> New Rule
          </button>
        }
      />

      <div className="flex bg-[var(--zf-canvas)] rounded-lg p-0.5 w-fit">
        {(['active', 'rules'] as const).map(t => (
          <button key={t} onClick={() => setTab(t)} className={`px-3 py-1.5 rounded text-sm capitalize transition ${tab === t ? 'bg-[var(--zf-surface)] text-[var(--zf-ink)] shadow-sm' : 'text-[var(--zf-muted)]'}`}>
            {t === 'active' ? `Active (${alerts.length + atlasAlerts.length})` : `Rules (${rules.length})`}
          </button>
        ))}
      </div>

      {loadError && (
        <ErrorBanner title="Could not load alerts" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchAll} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading…
        </div>
      ) : !loadError ? (
        <>
          {showCreate && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Alert Rule</h3>
              <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Name</label>
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="high-cpu" className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Type</label>
                  <select value={conditionType} onChange={(e) => setConditionType(e.target.value as AlertConditionType)} className="input-field text-sm w-full">
                    <option value="metric_threshold">Metric threshold (any VM)</option>
                    <option value="resource_usage">Resource usage (any VM)</option>
                    <option value="vm_state">Specific VM state</option>
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Severity</label>
                  <select value={severity} onChange={(e) => setSeverity(e.target.value as typeof severity)} className="input-field text-sm w-full">
                    <option value="info">Info</option>
                    <option value="warning">Warning</option>
                    <option value="critical">Critical</option>
                  </select>
                </div>
                {conditionType !== 'vm_state' ? (
                  <>
                    <div>
                      <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Metric</label>
                      <select value={metric} onChange={(e) => setMetric(e.target.value)} className="input-field text-sm w-full">
                        <option value="cpu_usage">CPU %</option>
                        <option value="memory_usage_percent">Memory %</option>
                      </select>
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Threshold (%)</label>
                      <input type="number" value={threshold} onChange={(e) => setThreshold(e.target.value)} className="input-field text-sm w-full" />
                    </div>
                  </>
                ) : (
                  <>
                    <div>
                      <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">VM</label>
                      <select value={vmName} onChange={(e) => setVmName(e.target.value)} className="input-field text-sm w-full">
                        {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                      </select>
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Phase</label>
                      <input type="text" value={vmState} onChange={(e) => setVmState(e.target.value)} placeholder="Failed" className="input-field text-sm w-full" />
                    </div>
                  </>
                )}
              </div>
              {createError && <p className="text-sm text-[var(--zf-danger)]">{createError}</p>}
              <div className="flex gap-2">
                <button onClick={handleCreate} disabled={creating} className="zf-btn zf-btn-primary zf-btn-sm">
                  {creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                  {creating ? 'Creating...' : 'Create'}
                </button>
                <button onClick={() => { setShowCreate(false); setCreateError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          {tab === 'active' ? (
            alerts.length === 0 && atlasAlerts.length === 0 ? (
              <EmptyState icon={<Bell className="w-8 h-8" />} title="No active alerts" description="Nothing is currently firing." />
            ) : (
              <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] divide-y divide-[var(--zf-hairline)]/30">
                {alerts.map(a => (
                  <div key={a.id} className="px-5 py-3 flex items-start gap-3">
                    <span className={`px-2 py-0.5 rounded-full text-xs font-medium border shrink-0 mt-0.5 ${severityBadge(a.severity)}`}>{a.severity}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-[var(--zf-ink)]">{a.message}</div>
                      <div className="text-[10px] text-[var(--zf-muted)] mt-0.5">{a.rule_name} · started {new Date(a.started_at).toLocaleString()}</div>
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      <button onClick={() => handleResolve(a)} disabled={busyId === a.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-success)] hover:bg-[var(--zf-success)]/10 rounded-lg transition-colors disabled:opacity-50" title="Resolve">
                        <CheckCircle className="w-4 h-4" />
                      </button>
                      <button onClick={() => handleSilence(a)} disabled={busyId === a.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] hover:bg-[var(--zf-hover-tint)] rounded-lg transition-colors disabled:opacity-50" title="Silence">
                        <BellOff className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                ))}
                {atlasAlerts.map(a => (
                  <div key={a.id} className="px-5 py-3 flex items-start gap-3">
                    <span className={`px-2 py-0.5 rounded-full text-xs font-medium border shrink-0 mt-0.5 ${severityBadge(a.severity)}`}>{a.severity}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-[var(--zf-ink)]">
                        {a.title}
                        <span className="ml-2 px-1.5 py-0.5 rounded text-[10px] font-medium border border-[var(--zf-hairline)] text-[var(--zf-muted)]">Atlas</span>
                      </div>
                      <div className="text-[10px] text-[var(--zf-muted)] mt-0.5">
                        {a.description} · {a.resource_type}/{a.resource_id}
                        {a.created_at ? ` · started ${new Date(a.created_at).toLocaleString()}` : ''}
                        {a.acknowledged_by ? ` · acked by ${a.acknowledged_by}` : ''}
                      </div>
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      {!a.acknowledged_at && (
                        <button onClick={() => handleAtlasAck(a)} disabled={busyId === a.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] hover:bg-[var(--zf-hover-tint)] rounded-lg transition-colors disabled:opacity-50" title="Acknowledge">
                        <Bell className="w-4 h-4" />
                      </button>
                      )}
                      <button onClick={() => handleAtlasResolve(a)} disabled={busyId === a.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-success)] hover:bg-[var(--zf-success)]/10 rounded-lg transition-colors disabled:opacity-50" title="Resolve">
                        <CheckCircle className="w-4 h-4" />
                      </button>
                      <button onClick={() => handleAtlasSilence(a)} disabled={busyId === a.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] hover:bg-[var(--zf-hover-tint)] rounded-lg transition-colors disabled:opacity-50" title="Silence 1h">
                        <BellOff className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )
          ) : (
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
              <DataTable
                columns={[
                  { key: 'name', header: 'Name', className: 'px-5', render: (r) => <span className="font-medium text-[var(--zf-ink)]">{r.name}</span> },
                  {
                    key: 'severity',
                    header: 'Severity',
                    render: (r) => <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${severityBadge(r.severity)}`}>{r.severity}</span>,
                  },
                  { key: 'condition', header: 'Condition', render: (r) => <span className="text-xs text-[var(--zf-muted)] font-mono">{JSON.stringify(r.condition)}</span> },
                  {
                    key: 'actions',
                    header: 'Actions',
                    className: 'text-right',
                    render: (r) => (
                      <button onClick={() => handleDeleteRule(r)} disabled={busyId === r.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete">
                        <Trash2 className="w-4 h-4" />
                      </button>
                    ),
                  },
                ]}
                rows={rules}
                getRowKey={(r) => r.id}
                bordered={false}
                emptyState={<EmptyState icon={<Bell className="w-8 h-8" />} title="No alert rules" description="Create one to get notified when a VM breaches a threshold." />}
              />
            </div>
          )}
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
