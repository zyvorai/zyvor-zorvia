// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { Gauge, Plus, Trash2, Loader2 } from 'lucide-react'
import { listQuotas, createQuota, deleteQuota, ResourceQuota } from '../api/quotas'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

const NAME_REGEX = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/

function formatQuantity(map: Record<string, string>, key: string): string | null {
  return Object.prototype.hasOwnProperty.call(map, key) ? map[key] : null
}

export default function Quotas() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [quotas, setQuotas] = useState<ResourceQuota[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showAdd, setShowAdd] = useState(false)
  const [newName, setNewName] = useState('')
  const [newNamespace, setNewNamespace] = useState('')
  const [newCpu, setNewCpu] = useState('')
  const [newMemory, setNewMemory] = useState('')
  const [newPods, setNewPods] = useState('')
  const [adding, setAdding] = useState(false)
  const [addError, setAddError] = useState('')
  const [deletingKey, setDeletingKey] = useState<string | null>(null)

  const fetchQuotas = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setQuotas(await listQuotas())
    } catch (err) {
      const msg = formatUserError(err)
      setLoadError(msg)
      toastFailure(toast, 'Failed to load quotas', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchQuotas() }, [fetchQuotas])

  const handleAdd = async () => {
    const name = newName.trim()
    if (!name) { setAddError('Quota name is required'); return }
    if (!NAME_REGEX.test(name)) { setAddError('Name must be lowercase alphanumeric with hyphens (RFC 1123)'); return }
    const hard: Record<string, string> = {}
    if (newCpu.trim()) hard['requests.cpu'] = newCpu.trim()
    if (newMemory.trim()) hard['requests.memory'] = newMemory.trim()
    if (newPods.trim()) hard.pods = newPods.trim()
    if (Object.keys(hard).length === 0) { setAddError('Set at least one limit (CPU, memory, or pods)'); return }

    setAdding(true); setAddError('')
    try {
      await createQuota(name, hard, newNamespace.trim() || undefined)
      toast.success(`Quota "${name}" created`)
      setNewName(''); setNewNamespace(''); setNewCpu(''); setNewMemory(''); setNewPods(''); setShowAdd(false)
      fetchQuotas()
    } catch (err) {
      setAddError(formatUserError(err))
      toastFailure(toast, 'Failed to create quota', err)
    } finally { setAdding(false) }
  }

  const handleDelete = async (namespace: string, name: string) => {
    const key = `${namespace}/${name}`
    if (!await confirm('Delete Quota', `Delete quota "${name}" in namespace "${namespace}"? This lifts its resource limits immediately.`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setDeletingKey(key)
    try {
      await deleteQuota(namespace, name)
      setQuotas(prev => prev.filter(q => `${q.namespace}/${q.name}` !== key))
      toast.success(`Quota "${name}" deleted`)
    } catch (err) {
      toastFailure(toast, 'Failed to delete quota', err)
    } finally {
      setDeletingKey(null)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Resource Quotas"
        description="Per-namespace CPU, memory, and pod limits enforced by Kubernetes"
        onRefresh={fetchQuotas}
        refreshing={loading}
        primaryAction={
          <button
            type="button"
            onClick={() => setShowAdd(!showAdd)}
            className="zf-btn zf-btn-primary zf-btn-sm"
          >
            <Plus className="w-4 h-4" /> New Quota
          </button>
        }
      />

      {loadError && (
        <ErrorBanner
          title="Could not load quotas"
          headline={loadError}
          hints={hintsForError(loadError)}
          onRetry={fetchQuotas}
        />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading quotas…
        </div>
      ) : !loadError ? (
        <>
          {showAdd && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Quota</h3>
              <div className="grid grid-cols-1 md:grid-cols-5 gap-4">
                <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Name</label><input type="text" value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="team-a-limits" className="input-field text-sm" /></div>
                <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Namespace</label><input type="text" value={newNamespace} onChange={(e) => setNewNamespace(e.target.value)} placeholder="default" className="input-field text-sm" /></div>
                <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">CPU (requests)</label><input type="text" value={newCpu} onChange={(e) => setNewCpu(e.target.value)} placeholder="16" className="input-field text-sm" /></div>
                <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Memory (requests)</label><input type="text" value={newMemory} onChange={(e) => setNewMemory(e.target.value)} placeholder="32Gi" className="input-field text-sm" /></div>
                <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Max Pods</label><input type="text" value={newPods} onChange={(e) => setNewPods(e.target.value)} placeholder="50" className="input-field text-sm" /></div>
              </div>
              {addError && <p className="text-sm text-[var(--zf-danger)]">{addError}</p>}
              <div className="flex gap-2">
                <button onClick={handleAdd} disabled={adding} className="zf-btn zf-btn-primary zf-btn-sm">{adding ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}{adding ? 'Creating...' : 'Create'}</button>
                <button onClick={() => { setShowAdd(false); setAddError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <DataTable
              columns={[
                { key: 'name', header: 'Name', className: 'px-5', render: (q) => <span className="font-medium text-[var(--zf-ink)]">{q.name}</span> },
                { key: 'namespace', header: 'Namespace', render: (q) => <span className="text-[var(--zf-muted)]">{q.namespace}</span> },
                {
                  key: 'cpu',
                  header: 'CPU (used / limit)',
                  render: (q) => {
                    const cpuLimit = formatQuantity(q.hard, 'requests.cpu') ?? formatQuantity(q.hard, 'cpu')
                    const cpuUsed = formatQuantity(q.used, 'requests.cpu') ?? formatQuantity(q.used, 'cpu')
                    return <span className="text-[var(--zf-muted)]">{cpuLimit ? `${cpuUsed ?? '0'} / ${cpuLimit}` : '—'}</span>
                  },
                },
                {
                  key: 'memory',
                  header: 'Memory (used / limit)',
                  render: (q) => {
                    const memLimit = formatQuantity(q.hard, 'requests.memory') ?? formatQuantity(q.hard, 'memory')
                    const memUsed = formatQuantity(q.used, 'requests.memory') ?? formatQuantity(q.used, 'memory')
                    return <span className="text-[var(--zf-muted)]">{memLimit ? `${memUsed ?? '0'} / ${memLimit}` : '—'}</span>
                  },
                },
                {
                  key: 'pods',
                  header: 'Pods (used / limit)',
                  render: (q) => {
                    const podsLimit = formatQuantity(q.hard, 'pods')
                    const podsUsed = formatQuantity(q.used, 'pods')
                    return <span className="text-[var(--zf-muted)]">{podsLimit ? `${podsUsed ?? '0'} / ${podsLimit}` : '—'}</span>
                  },
                },
                {
                  key: 'actions',
                  header: 'Actions',
                  className: 'text-right',
                  render: (q) => {
                    const key = `${q.namespace}/${q.name}`
                    return (
                      <button onClick={() => handleDelete(q.namespace, q.name)} disabled={deletingKey === key} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete quota">
                        {deletingKey === key ? <Loader2 className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
                      </button>
                    )
                  },
                },
              ]}
              rows={quotas}
              getRowKey={(q) => `${q.namespace}/${q.name}`}
              bordered={false}
              emptyState={<EmptyState icon={<Gauge className="w-10 h-10" />} title="No resource quotas configured" />}
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
