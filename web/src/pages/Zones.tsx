// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { MapPin } from 'lucide-react'
import { listZones, Zone } from '../api/zones'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

export default function Zones() {
  const toast = useToastContext()
  const [zones, setZones] = useState<Zone[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const fetchZones = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setZones(await listZones())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load zones', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchZones() }, [fetchZones])

  return (
    <div className="space-y-6">
      <PageHeader
        title="Zones"
        description="Real cluster nodes grouped by their topology.kubernetes.io/zone label — a single-cluster view, not multi-datacenter federation"
        onRefresh={fetchZones}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load zones" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchZones} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading zones…
        </div>
      ) : !loadError && zones.length === 0 ? (
        <EmptyState icon={<MapPin className="w-8 h-8" />} title="No zone data" description="No nodes were found in this cluster." />
      ) : !loadError ? (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {zones.map(z => (
            <div key={z.zone} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5">
              <div className="flex items-center gap-2 mb-3">
                <MapPin className="w-4 h-4 text-[var(--zf-muted)]" />
                <h3 className="text-sm font-semibold text-[var(--zf-ink)]">{z.zone}</h3>
              </div>
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div><div className="text-lg font-bold text-[var(--zf-ink)]">{z.node_count}</div><div className="text-xs text-[var(--zf-muted)]">Nodes</div></div>
                <div><div className="text-lg font-bold text-[var(--zf-ink)]">{z.vm_count}</div><div className="text-xs text-[var(--zf-muted)]">VMs</div></div>
                <div><div className="text-lg font-bold text-[var(--zf-ink)]">{z.allocatable_cpu.toFixed(1)}</div><div className="text-xs text-[var(--zf-muted)]">vCPUs</div></div>
                <div><div className="text-lg font-bold text-[var(--zf-ink)]">{z.allocatable_memory_gib.toFixed(1)}</div><div className="text-xs text-[var(--zf-muted)]">GiB</div></div>
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
