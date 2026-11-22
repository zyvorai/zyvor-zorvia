// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { Save, Trash2, Plus, Loader2, HardDrive, AlertTriangle } from 'lucide-react'
import { listBackups, createBackup, deleteBackup, Backup } from '../api/backup'
import { listVMs, VM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable, StatusBadge, type DataTableColumn } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function fmtBytes(n: number): string {
  if (!n) return '—'
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`
  return `${(n / 1024).toFixed(1)} KB`
}

export default function Backups() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [backups, setBackups] = useState<Backup[]>([])
  const [vms, setVms] = useState<VM[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [vmName, setVmName] = useState('')
  const [retentionDays, setRetentionDays] = useState('30')
  const [description, setDescription] = useState('')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState('')
  const [deletingId, setDeletingId] = useState<string | null>(null)

  const fetchAll = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [backupList, vmList] = await Promise.all([listBackups(), listVMs()])
      setBackups(backupList)
      setVms(vmList)
      if (!vmName && vmList[0]) setVmName(vmList[0].name)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load backups', err)
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { fetchAll() }, [fetchAll])

  const handleCreate = async () => {
    if (!vmName) { setCreateError('Select a VM'); return }
    const days = parseInt(retentionDays, 10)
    setCreating(true)
    setCreateError('')
    try {
      await createBackup({
        vm_name: vmName,
        backup_type: 'full',
        retention_days: Number.isFinite(days) && days > 0 ? days : undefined,
        description: description.trim() || undefined,
      })
      toast.success(`Backup started for "${vmName}"`)
      setShowCreate(false)
      setDescription('')
      fetchAll()
    } catch (err) {
      setCreateError(formatUserError(err))
      toastFailure(toast, 'Failed to create backup', err)
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async (backup: Backup) => {
    if (!await confirm('Delete Backup', `Delete backup "${backup.id}" for "${backup.vm_name}"? This cannot be undone.`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setDeletingId(backup.id)
    try {
      await deleteBackup(backup.id)
      setBackups(prev => prev.filter(b => b.id !== backup.id))
      toast.success('Backup deleted')
    } catch (err) {
      toastFailure(toast, 'Failed to delete backup', err)
    } finally {
      setDeletingId(null)
    }
  }

  const backupColumns: DataTableColumn<Backup>[] = [
    {
      key: 'id',
      header: 'Backup',
      className: 'px-5',
      render: (b) => (
        <div className="flex items-center gap-1.5">
          <span className="font-mono text-xs text-[var(--zf-ink)]">{b.id}</span>
          {b.warning && (
            <span title={b.warning}>
              <AlertTriangle className="w-3.5 h-3.5 text-[var(--zf-warning)] shrink-0" />
            </span>
          )}
        </div>
      ),
    },
    { key: 'vm', header: 'VM', render: (b) => <span className="text-[var(--zf-muted)]">{b.vm_name}</span> },
    { key: 'status', header: 'Status', render: (b) => <StatusBadge status={b.status} /> },
    { key: 'size', header: 'Size', render: (b) => <span className="text-[var(--zf-muted)]">{fmtBytes(b.size_bytes)}</span> },
    { key: 'created', header: 'Created', render: (b) => <span className="text-xs text-[var(--zf-muted)]">{b.created ? new Date(b.created).toLocaleString() : '—'}</span> },
    { key: 'expires', header: 'Expires', render: (b) => <span className="text-xs text-[var(--zf-muted)]">{b.expires_at ? new Date(b.expires_at).toLocaleDateString() : '—'}</span> },
    {
      key: 'actions',
      header: 'Actions',
      className: 'text-right',
      render: (b) => (
        <button onClick={() => handleDelete(b)} disabled={deletingId === b.id} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete backup">
          {deletingId === b.id ? <Loader2 className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
        </button>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <PageHeader
        title="Backups"
        description="Point-in-time VM backups, backed by real VolumeSnapshots"
        onRefresh={fetchAll}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowCreate(!showCreate)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> New Backup
          </button>
        }
      />

      {loadError && (
        <ErrorBanner title="Could not load backups" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchAll} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading backups…
        </div>
      ) : !loadError ? (
        <>
          {showCreate && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Backup</h3>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">VM</label>
                  <select value={vmName} onChange={(e) => setVmName(e.target.value)} className="input-field text-sm w-full">
                    {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Retention (days)</label>
                  <input type="number" min={1} value={retentionDays} onChange={(e) => setRetentionDays(e.target.value)} className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Description (optional)</label>
                  <input type="text" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="pre-upgrade backup" className="input-field text-sm w-full" />
                </div>
              </div>
              {createError && <p className="text-sm text-[var(--zf-danger)]">{createError}</p>}
              <div className="flex gap-2">
                <button onClick={handleCreate} disabled={creating} className="zf-btn zf-btn-primary zf-btn-sm">
                  {creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                  {creating ? 'Starting...' : 'Start Backup'}
                </button>
                <button onClick={() => { setShowCreate(false); setCreateError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <DataTable
              columns={backupColumns}
              rows={backups}
              getRowKey={(b) => b.id}
              bordered={false}
              emptyState={
                <EmptyState icon={<HardDrive className="w-8 h-8" />} title="No backups yet" description="Start a backup to capture a point-in-time VolumeSnapshot of a VM." />
              }
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
