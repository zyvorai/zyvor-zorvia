// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useState } from 'react'
import { ArrowRightLeft, Plus, XCircle } from 'lucide-react'
import {
  listVmMigrations,
  startMigration,
  cancelMigration,
  MigrationStatus,
  MigrationPhase,
} from '../api/migrations'
import { listVMs, VM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import ErrorBanner from '../components/ErrorBanner'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import { PageHeader, EmptyState, DataTable, type DataTableColumn } from '../components/ui'

const TERMINAL_PHASES: MigrationPhase[] = ['Succeeded', 'Failed']

export default function Migrations() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [vms, setVms] = useState<VM[]>([])
  const [migrations, setMigrations] = useState<MigrationStatus[]>([])
  const [loading, setLoading] = useState(true)
  const [showStartDialog, setShowStartDialog] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    void loadAll(false)
    const interval = setInterval(() => void loadAll(true), 5000)
    return () => clearInterval(interval)
  }, [])

  const loadAll = async (silent = false) => {
    if (!silent) setLoadError(null)
    try {
      const vmList = await listVMs()
      setVms(vmList)
      const results = await Promise.all(
        vmList.map((vm) => listVmMigrations(vm.name).catch(() => [] as MigrationStatus[])),
      )
      setMigrations(results.flat())
      setLoadError(null)
    } catch (error) {
      const msg = formatUserError(error)
      if (!silent || migrations.length === 0) {
        setLoadError(msg)
        if (!silent) toastFailure(toast, 'Failed to load migrations', error)
      }
    } finally {
      if (!silent) setLoading(false)
    }
  }

  const handleCancel = async (id: string) => {
    if (!await confirm('Cancel Migration', 'Cancel this migration in progress? It may leave the VM in a partially-migrated state.', { variant: 'danger', confirmLabel: 'Cancel Migration' })) return
    try {
      await cancelMigration(id)
      toast.success('Migration cancelled')
      void loadAll(true)
    } catch (error) {
      toastFailure(toast, 'Failed to cancel migration', error)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-[var(--zf-ink)]"></div>
      </div>
    )
  }

  const activeMigrations = migrations.filter(m => !TERMINAL_PHASES.includes(m.phase))
  const completedMigrations = migrations.filter(m => TERMINAL_PHASES.includes(m.phase))

  const historyColumns: DataTableColumn<MigrationStatus>[] = [
    { key: 'vm', header: 'VM', render: (m) => <span className="font-medium">{m.vm_name}</span> },
    { key: 'source', header: 'Source', render: (m) => <span className="font-mono text-sm text-[var(--zf-muted)]">{m.source_node || '—'}</span> },
    { key: 'target', header: 'Target', render: (m) => <span className="font-mono text-sm text-[var(--zf-muted)]">{m.target_node || '—'}</span> },
    { key: 'phase', header: 'Phase', render: (m) => <StatusBadge phase={m.phase} /> },
    {
      key: 'started',
      header: 'Started',
      render: (m) => <span className="text-sm text-[var(--zf-muted)]">{m.start_timestamp ? new Date(m.start_timestamp).toLocaleString() : '—'}</span>,
    },
  ]

  return (
    <div className="space-y-6">
      {loadError && (
        <ErrorBanner
          title="Could not load migrations"
          headline={loadError}
          hints={hintsForError(loadError)}
          onRetry={() => void loadAll()}
        />
      )}
      <PageHeader
        title="Migrations"
        description="Live-migrate VMs between nodes for load balancing or maintenance. KubeVirt's scheduler selects the target node automatically."
        onRefresh={() => void loadAll()}
        primaryAction={
          <button
            onClick={() => setShowStartDialog(true)}
            className="zf-btn zf-btn-primary zf-btn-sm"
          >
            <Plus className="w-4 h-4" />
            Start Migration
          </button>
        }
      />

      {showStartDialog && (
        <StartMigrationDialog
          vms={vms}
          onClose={() => setShowStartDialog(false)}
          onSuccess={() => {
            toast.success('Migration started')
            setShowStartDialog(false)
            void loadAll(true)
          }}
        />
      )}

      {activeMigrations.length > 0 && (
        <div className="space-y-4">
          <h2 className="text-xl font-semibold">Active Migrations</h2>
          {activeMigrations.map(migration => (
            <MigrationCard
              key={migration.id}
              migration={migration}
              onCancel={() => void handleCancel(migration.id)}
            />
          ))}
        </div>
      )}

      <div className="space-y-4">
        <h2 className="text-xl font-semibold">Migration History</h2>
        {completedMigrations.length === 0 && activeMigrations.length === 0 ? (
          <div className="text-center py-12 bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)]">
            <ArrowRightLeft className="w-16 h-16 mx-auto mb-4 text-[var(--zf-muted)]" />
            <p className="text-xl text-[var(--zf-muted)] mb-4">No migrations yet</p>
            <p className="text-[var(--zf-muted)] mb-6">Live-migrate a VM to move it between nodes for load balancing or maintenance</p>
            <button
              onClick={() => setShowStartDialog(true)}
              className="zf-btn zf-btn-primary"
            >
              Start Migration
            </button>
          </div>
        ) : (
          <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] overflow-hidden">
            <DataTable
              columns={historyColumns}
              rows={completedMigrations}
              getRowKey={(migration) => migration.id}
              bordered={false}
              emptyState={<EmptyState icon={<ArrowRightLeft className="w-10 h-10" />} title="No completed migrations yet" />}
            />
          </div>
        )}
      </div>

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel}
          variant={confirmState.variant}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}

function StatusBadge({ phase }: { phase: string }) {
  const styles: Record<string, string> = {
    Pending: 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
    Scheduling: 'text-[var(--zf-link)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
    Scheduled: 'text-[var(--zf-link)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
    PreparingTarget: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
    TargetReady: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
    Running: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
    Succeeded: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
    Failed: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
  }

  return (
    <span className={`px-3 py-1 rounded-full text-xs font-medium border ${styles[phase] || styles.Pending}`}>
      {phase}
    </span>
  )
}

function MigrationCard({
  migration,
  onCancel,
}: {
  migration: MigrationStatus
  onCancel: () => void
}) {
  return (
    <div className="bg-[var(--zf-surface)] rounded-lg p-6 border border-[var(--zf-hairline)]">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-lg font-bold">{migration.vm_name}</h3>
          <p className="text-sm text-[var(--zf-muted)]">
            {migration.source_node && migration.target_node ? (
              <>
                <span className="font-mono">{migration.source_node}</span>
                {' → '}
                <span className="font-mono">{migration.target_node}</span>
              </>
            ) : (
              'Target node selected by the KubeVirt scheduler'
            )}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <StatusBadge phase={migration.phase} />
          <button
            onClick={onCancel}
            className="zf-btn zf-btn-danger zf-btn-sm"
          >
            <XCircle className="w-4 h-4" />
            Cancel
          </button>
        </div>
      </div>
    </div>
  )
}

function StartMigrationDialog({
  vms,
  onClose,
  onSuccess,
}: {
  vms: VM[]
  onClose: () => void
  onSuccess: () => void
}) {
  const toast = useToastContext()
  const [vmName, setVmName] = useState('')
  const [isStarting, setIsStarting] = useState(false)

  const handleStart = async () => {
    if (!vmName) {
      toast.error('Please select a VM')
      return
    }
    setIsStarting(true)
    try {
      await startMigration(vmName)
      onSuccess()
    } catch (error) {
      toastFailure(toast, 'Failed to start migration', error)
    } finally {
      setIsStarting(false)
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Start Migration</h3>

      <div>
        <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">VM</label>
        <select
          value={vmName}
          onChange={(e) => setVmName(e.target.value)}
          className="input-field text-sm"
        >
          <option value="">Select a VM</option>
          {vms.map(vm => (
            <option key={vm.name} value={vm.name}>{vm.name}</option>
          ))}
        </select>
      </div>
      <p className="text-sm text-[var(--zf-muted)]">
        KubeVirt live-migrates the VM to a node its scheduler selects; there is no target host to configure.
      </p>

      <div className="flex gap-2">
        <button
          onClick={() => void handleStart()}
          disabled={isStarting || !vmName}
          className="zf-btn zf-btn-primary zf-btn-sm"
        >
          {isStarting ? 'Starting...' : 'Start Migration'}
        </button>
        <button
          onClick={onClose}
          disabled={isStarting}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
