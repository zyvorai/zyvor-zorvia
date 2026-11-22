// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { HardDrive } from 'lucide-react'
import { listStorageVolumes, StorageVolume } from '../api/storage'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

function statusBadge(status: string): string {
  switch (status.toLowerCase()) {
    case 'bound': return 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25'
    case 'pending': return 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25'
    case 'lost': return 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'
    default: return 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]'
  }
}

export default function Storage() {
  const toast = useToastContext()
  const [volumes, setVolumes] = useState<StorageVolume[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const fetchVolumes = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setVolumes(await listStorageVolumes())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load storage volumes', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchVolumes() }, [fetchVolumes])

  return (
    <div className="space-y-6">
      <PageHeader
        title="Storage"
        description="Every PersistentVolumeClaim in the namespace, and which VM (if any) each one is attached to"
        onRefresh={fetchVolumes}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load storage volumes" headline={loadError} hints={hintsForError(loadError, 'storage')} onRetry={fetchVolumes} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading volumes…
        </div>
      ) : !loadError && volumes.length === 0 ? (
        <EmptyState icon={<HardDrive className="w-8 h-8" />} title="No volumes found" description="PersistentVolumeClaims created by VM disks will appear here." />
      ) : !loadError ? (
        <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
          <DataTable
            columns={[
              { key: 'name', header: 'Name', className: 'px-5', render: (v) => <span className="font-mono text-xs text-[var(--zf-ink)]">{v.name}</span> },
              {
                key: 'status',
                header: 'Status',
                render: (v) => <span className={`px-2.5 py-0.5 rounded-full text-xs font-medium border ${statusBadge(v.status)}`}>{v.status}</span>,
              },
              { key: 'capacity', header: 'Capacity', render: (v) => <span className="text-[var(--zf-muted)]">{v.capacity || '—'}</span> },
              { key: 'storage_class', header: 'Storage Class', render: (v) => <span className="text-[var(--zf-muted)]">{v.storage_class || '—'}</span> },
              {
                key: 'attached_vms',
                header: 'Attached VMs',
                render: (v) => <span className="text-[var(--zf-muted)]">{v.attached_vms.length > 0 ? v.attached_vms.join(', ') : '—'}</span>,
              },
            ]}
            rows={volumes}
            getRowKey={(v) => v.name}
            bordered={false}
          />
        </div>
      ) : null}
    </div>
  )
}
