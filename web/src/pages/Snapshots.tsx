// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useState, useEffect } from 'react'
import { Camera, Plus, Trash2, RotateCcw, AlertTriangle, List, GitBranch } from 'lucide-react'
import {
  listSnapshots,
  createSnapshotWithRetry,
  deleteSnapshot,
  revertSnapshot,
  type VMSnapshot,
} from '../api/snapshots'
import { useConfirm } from '../hooks/useConfirm'
import { useToastContext } from '../contexts/ToastContext'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import RelativeTime from '../components/RelativeTime'
import { PageHeader, DataTable, type DataTableColumn } from '../components/ui'
import { buildSnapshotChain, type SnapshotNode } from '../utils/snapshotChain'

export default function Snapshots() {
  const { confirmState, confirm, cancel } = useConfirm()
  const toast = useToastContext()
  const [vmName, setVmName] = useState('')
  const [snapshots, setSnapshots] = useState<VMSnapshot[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [view, setView] = useState<'table' | 'chain'>('table')

  const loadSnapshots = async () => {
    if (!vmName) return
    setLoading(true)
    setLoadError(null)
    try {
      const data = await listSnapshots(vmName)
      setSnapshots(data)
    } catch (error) {
      const msg = formatUserError(error)
      setLoadError(msg)
      toastFailure(toast, 'Failed to load snapshots', error)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (vmName) void loadSnapshots()
  }, [vmName])

  const handleDelete = async (id: string) => {
    if (!await confirm('Delete Snapshot', 'Are you sure you want to delete this snapshot?')) return
    try {
      await deleteSnapshot(vmName, id)
      await loadSnapshots()
    } catch (error) {
      toastFailure(toast, 'Failed to delete snapshot', error)
    }
  }

  const handleRevert = async (id: string) => {
    if (!await confirm('Revert Snapshot', 'Revert VM to this snapshot? The VM must be stopped.', { confirmLabel: 'Revert', variant: 'warning' })) return
    try {
      await revertSnapshot(vmName, id)
      toast.success('Successfully reverted to snapshot')
    } catch (error) {
      toastFailure(toast, 'Failed to revert snapshot', error)
    }
  }

  const snapshotColumns: DataTableColumn<VMSnapshot>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (snap) => (
        <div className="flex items-center gap-2">
          <Camera className="w-4 h-4 text-[var(--zf-link)]" />
          <span className="font-medium">{snap.name}</span>
          {snap.warning && (
            <span title={snap.warning}>
              <AlertTriangle className="w-3.5 h-3.5 text-[var(--zf-warning)] shrink-0" />
            </span>
          )}
        </div>
      ),
    },
    { key: 'type', header: 'Type', render: (snap) => <span className="text-sm text-[var(--zf-muted)]">{snap.snapshot_type}</span> },
    { key: 'description', header: 'Description', render: (snap) => <span className="text-sm text-[var(--zf-muted)]">{snap.description || '-'}</span> },
    {
      key: 'created',
      header: 'Created',
      render: (snap) => (
        <span className="text-sm text-[var(--zf-muted)]">
          <RelativeTime date={snap.created} />
        </span>
      ),
    },
    {
      key: 'actions',
      header: 'Actions',
      render: (snap) => (
        <div className="flex items-center gap-2">
          <button type="button" onClick={() => void handleRevert(snap.id)} className="p-2 hover:bg-[var(--zf-hover-tint)] rounded transition" title="Revert to snapshot">
            <RotateCcw className="w-4 h-4 text-[var(--zf-warning)]" />
          </button>
          <button type="button" onClick={() => void handleDelete(snap.id)} className="p-2 hover:bg-[var(--zf-hover-tint)] rounded transition" title="Delete snapshot">
            <Trash2 className="w-4 h-4 text-[var(--zf-danger)]" />
          </button>
        </div>
      ),
    },
  ]

  return (
    <div className="p-8">
      {loadError && vmName && (
        <ErrorBanner
          title="Could not load snapshots"
          headline={loadError}
          onRetry={loadSnapshots}
        />
      )}
      <PageHeader title="VM Snapshots" description="Create and manage VM snapshots" />

      <div className="zf-panel p-6 mb-8">
        <div className="flex items-center gap-4">
          <div className="flex-1">
            <label htmlFor="snapshot-vm-name" className="block text-sm font-medium mb-2">VM Name</label>
            <input
              id="snapshot-vm-name"
              type="text"
              value={vmName}
              onChange={(e) => setVmName(e.target.value)}
              className="w-full bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded px-4 py-2"
              placeholder="Enter VM name"
            />
          </div>
          <button type="button" onClick={() => void loadSnapshots()} disabled={!vmName} className="zf-btn zf-btn-ghost mt-7">
            Load Snapshots
          </button>
          <button type="button" onClick={() => setShowCreateDialog(true)} disabled={!vmName} className="zf-btn zf-btn-primary mt-7">
            <Plus className="w-4 h-4" />
            Create
          </button>
        </div>
      </div>

      {showCreateDialog && (
        <div className="mb-8">
          <CreateSnapshotDialog
            vmName={vmName}
            onClose={() => setShowCreateDialog(false)}
            onCreated={loadSnapshots}
          />
        </div>
      )}

      {vmName && (
        <div className="flex justify-end mb-3" role="group" aria-label="Snapshot view">
          <button
            type="button"
            className={`zf-btn zf-btn-sm ${view === 'table' ? 'zf-btn-primary' : 'zf-btn-ghost'}`}
            aria-pressed={view === 'table'}
            onClick={() => setView('table')}
          >
            <List className="w-4 h-4" /> Table
          </button>
          <button
            type="button"
            className={`zf-btn zf-btn-sm ml-2 ${view === 'chain' ? 'zf-btn-primary' : 'zf-btn-ghost'}`}
            aria-pressed={view === 'chain'}
            onClick={() => setView('chain')}
          >
            <GitBranch className="w-4 h-4" /> Chain
          </button>
        </div>
      )}

      {vmName && view === 'chain' && (
        <div className="zf-panel p-6">
          {snapshots.length === 0 ? (
            <div className="text-center text-[var(--zf-muted)]">No snapshots found for this VM.</div>
          ) : (
            <SnapshotChainView
              nodes={buildSnapshotChain(snapshots)}
              onRevert={(id) => void handleRevert(id)}
              onDelete={(id) => void handleDelete(id)}
            />
          )}
        </div>
      )}

      {vmName && view === 'table' && (
        <div className="zf-panel overflow-hidden">
          <DataTable
            columns={snapshotColumns}
            rows={snapshots}
            getRowKey={(snap) => snap.id}
            loading={loading}
            bordered={false}
            emptyState={
              <div className="p-8 text-center text-[var(--zf-muted)]">No snapshots found for this VM.</div>
            }
          />
        </div>
      )}

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

function CreateSnapshotDialog({
  vmName,
  onClose,
  onCreated,
}: {
  vmName: string
  onClose: () => void
  onCreated: () => void
}) {
  const toast = useToastContext()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [snapshotType, setSnapshotType] = useState<'Disk' | 'Full'>('Disk')
  const [submitting, setSubmitting] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setSubmitting(true)
    try {
      await createSnapshotWithRetry(vmName, {
        name,
        description: description || undefined,
        snapshot_type: snapshotType,
      })
      toast.success(
        snapshotType === 'Full'
          ? `Full snapshot '${name}' created`
          : `Snapshot '${name}' created`,
      )
      onCreated()
      onClose()
    } catch (error) {
      toastFailure(toast, 'Failed to create snapshot', error)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Create Snapshot</h3>
      <form onSubmit={(e) => void handleSubmit(e)} className="space-y-4">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div>
            <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Snapshot Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="input-field text-sm"
              required
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Description</label>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              className="input-field text-sm"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Type</label>
            <select
              value={snapshotType}
              onChange={(e) => setSnapshotType(e.target.value as 'Disk' | 'Full')}
              className="input-field text-sm"
            >
              <option value="Disk">Disk</option>
              <option value="Full">Full</option>
            </select>
          </div>
        </div>
        <div className="flex gap-2">
          <button type="submit" disabled={submitting || !name} className="zf-btn zf-btn-primary zf-btn-sm">
            {submitting ? 'Creating…' : 'Create'}
          </button>
          <button type="button" onClick={onClose} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
        </div>
      </form>
    </div>
  )
}

function SnapshotChainView({
  nodes,
  onRevert,
  onDelete,
}: {
  nodes: SnapshotNode[]
  onRevert: (id: string) => void
  onDelete: (id: string) => void
}) {
  return (
    <ol className="space-y-3" aria-label="Snapshot chain">
      {nodes.map((node) => (
        <li key={node.snapshot.id}>
          <div
            className="flex items-center gap-3 rounded-lg border border-[var(--zf-hairline)] bg-[var(--zf-surface)] px-4 py-3"
            style={{ marginLeft: `${node.depth * 1.5}rem` }}
          >
            <Camera className="w-4 h-4 text-[var(--zf-link)] shrink-0" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <div className="font-medium truncate">
                {node.snapshot.name}
                <span className="ml-2 text-xs text-[var(--zf-muted)]">{node.snapshot.snapshot_type}</span>
              </div>
              <div className="text-xs text-[var(--zf-muted)]">
                <RelativeTime date={node.snapshot.created} />
                {node.snapshot.description ? ` · ${node.snapshot.description}` : ''}
              </div>
            </div>
            <button type="button" onClick={() => onRevert(node.snapshot.id)} className="p-2 hover:bg-[var(--zf-hover-tint)] rounded transition" aria-label={`Revert to ${node.snapshot.name}`}>
              <RotateCcw className="w-4 h-4 text-[var(--zf-warning)]" />
            </button>
            <button type="button" onClick={() => onDelete(node.snapshot.id)} className="p-2 hover:bg-[var(--zf-hover-tint)] rounded transition" aria-label={`Delete ${node.snapshot.name}`}>
              <Trash2 className="w-4 h-4 text-[var(--zf-danger)]" />
            </button>
          </div>
          {node.children.length > 0 && (
            <div className="mt-3 border-l border-[var(--zf-hairline)]" style={{ marginLeft: `${node.depth * 1.5 + 0.75}rem`, paddingLeft: '0.75rem' }}>
              <SnapshotChainView nodes={node.children} onRevert={onRevert} onDelete={onDelete} />
            </div>
          )}
        </li>
      ))}
    </ol>
  )
}
