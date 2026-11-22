// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { Layers3, Plus, Trash2, Zap, Loader2 } from 'lucide-react'
import { listWarmPools, createWarmPool, deleteWarmPool, claimWarmPoolMember, WarmPool } from '../api/warmPools'
import { listTemplatesByFamily } from '../api/templates'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function statusBadge(status: string): string {
  switch (status) {
    case 'ready': return 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25'
    case 'claimed': return 'text-[var(--zf-link)] bg-[var(--zf-link)]/10 border-[var(--zf-link)]/25'
    case 'failed': return 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'
    default: return 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25'
  }
}

export default function WarmPools() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [pools, setPools] = useState<WarmPool[]>([])
  const [templates, setTemplates] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [name, setName] = useState('')
  const [template, setTemplate] = useState('')
  const [size, setSize] = useState('2')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState('')
  const [busyName, setBusyName] = useState<string | null>(null)

  const fetchAll = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [poolList, families] = await Promise.all([listWarmPools(), listTemplatesByFamily()])
      setPools(poolList)
      const allTemplates = Object.values(families).flat()
      setTemplates(allTemplates)
      if (!template && allTemplates[0]) setTemplate(allTemplates[0])
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load warm pools', err)
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { fetchAll() }, [fetchAll])

  const handleCreate = async () => {
    if (!name.trim()) { setCreateError('Name is required'); return }
    if (!template) { setCreateError('Select a template'); return }
    setCreating(true)
    setCreateError('')
    try {
      await createWarmPool({ name: name.trim(), template, size: parseInt(size, 10) || 1 })
      toast.success(`Pool "${name}" created — standbys will provision within a minute`)
      setName(''); setShowCreate(false)
      fetchAll()
    } catch (err) {
      setCreateError(formatUserError(err))
      toastFailure(toast, 'Failed to create pool', err)
    } finally {
      setCreating(false)
    }
  }

  const handleClaim = async (pool: WarmPool) => {
    setBusyName(pool.name)
    try {
      const result = await claimWarmPoolMember(pool.name)
      toast.success(`Claimed and started "${result.vm_name}"`)
      fetchAll()
    } catch (err) {
      toastFailure(toast, 'Failed to claim a standby VM', err)
    } finally {
      setBusyName(null)
    }
  }

  const handleDelete = async (pool: WarmPool) => {
    if (!await confirm('Delete Pool', `Delete warm pool "${pool.name}"? Already-provisioned standby VMs are left in place.`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setBusyName(pool.name)
    try {
      await deleteWarmPool(pool.name)
      setPools(prev => prev.filter(p => p.name !== pool.name))
      toast.success('Pool deleted')
    } catch (err) {
      toastFailure(toast, 'Failed to delete pool', err)
    } finally {
      setBusyName(null)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Warm Pools"
        description="Pre-provisioned standby VMs from the real template catalog, kept stopped until claimed — saves create-time work, not boot time"
        onRefresh={fetchAll}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowCreate(!showCreate)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> New Pool
          </button>
        }
      />

      {loadError && (
        <ErrorBanner title="Could not load warm pools" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchAll} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading pools…
        </div>
      ) : !loadError ? (
        <>
          {showCreate && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Pool</h3>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Name</label>
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="ubuntu-standby" className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Template</label>
                  <select value={template} onChange={(e) => setTemplate(e.target.value)} className="input-field text-sm w-full">
                    {templates.map(t => <option key={t} value={t}>{t}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Standby count</label>
                  <input type="number" min={1} max={20} value={size} onChange={(e) => setSize(e.target.value)} className="input-field text-sm w-full" />
                </div>
              </div>
              {createError && <p className="text-sm text-[var(--zf-danger)]">{createError}</p>}
              <div className="flex gap-2">
                <button onClick={handleCreate} disabled={creating} className="zf-btn zf-btn-primary zf-btn-sm">
                  {creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                  {creating ? 'Creating...' : 'Create Pool'}
                </button>
                <button onClick={() => { setShowCreate(false); setCreateError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          {pools.length === 0 ? (
            <EmptyState icon={<Layers3 className="w-8 h-8" />} title="No warm pools" description="Create one to keep pre-provisioned VMs ready to claim." />
          ) : (
            <div className="space-y-4">
              {pools.map(pool => (
                <div key={pool.name} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
                  <div className="px-5 py-3 border-b border-[var(--zf-hairline)] flex items-center justify-between">
                    <div>
                      <span className="font-medium text-[var(--zf-ink)]">{pool.name}</span>
                      <span className="text-xs text-[var(--zf-muted)] ml-2">from {pool.template} · {pool.ready_count}/{pool.size} ready</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <button onClick={() => handleClaim(pool)} disabled={busyName === pool.name || pool.ready_count === 0} className="zf-btn zf-btn-primary zf-btn-sm">
                        {busyName === pool.name ? <Loader2 className="w-4 h-4 animate-spin" /> : <Zap className="w-4 h-4" />}
                        Claim
                      </button>
                      <button onClick={() => handleDelete(pool)} disabled={busyName === pool.name} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete pool">
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                  {pool.members.length > 0 && (
                    <div className="divide-y divide-[var(--zf-hairline)]/30">
                      {pool.members.map(m => (
                        <div key={m.vm_name} className="px-5 py-2 flex items-center justify-between text-sm">
                          <span className="font-mono text-xs text-[var(--zf-ink)]">{m.vm_name}</span>
                          <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${statusBadge(m.status)}`}>{m.status}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
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
