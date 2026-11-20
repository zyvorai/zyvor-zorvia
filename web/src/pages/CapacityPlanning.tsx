// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { Gauge } from 'lucide-react'
import { getCapacityOverview, getCapacityFit, CapacityOverview } from '../api/capacity'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

function pct(used: number, total: number): number {
  return total > 0 ? Math.min(100, (used / total) * 100) : 0
}

function barColor(p: number): string {
  if (p >= 90) return 'bg-[var(--zf-danger)]'
  if (p >= 70) return 'bg-[var(--zf-warning)]'
  return 'bg-[var(--zf-success)]'
}

export default function CapacityPlanning() {
  const toast = useToastContext()
  const [data, setData] = useState<CapacityOverview | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [fitCpu, setFitCpu] = useState('2')
  const [fitMemory, setFitMemory] = useState('4')
  const [fitResult, setFitResult] = useState<number | null>(null)
  const [fitting, setFitting] = useState(false)

  const fetchOverview = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setData(await getCapacityOverview())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load capacity data', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchOverview() }, [fetchOverview])

  const handleFit = async () => {
    setFitting(true)
    try {
      const result = await getCapacityFit(parseFloat(fitCpu) || 1, parseFloat(fitMemory) || 1)
      setFitResult(result.estimated_additional_vms)
    } catch (err) {
      toastFailure(toast, 'Failed to estimate capacity', err)
    } finally {
      setFitting(false)
    }
  }

  const cpuPct = data ? pct(data.used_cpu, data.total_cpu) : 0
  const memPct = data ? pct(data.used_memory_gib, data.total_memory_gib) : 0

  return (
    <div className="space-y-6">
      <PageHeader
        title="Capacity Planning"
        description="Real cluster headroom, computed from actual Node allocatable capacity minus actual VM usage"
        onRefresh={fetchOverview}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load capacity data" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchOverview} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading…
        </div>
      ) : !loadError && data ? (
        <>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5">
              <div className="flex justify-between text-sm mb-2">
                <span className="text-[var(--zf-muted)]">CPU</span>
                <span className="text-[var(--zf-ink)] font-medium">{data.used_cpu.toFixed(1)} / {data.total_cpu.toFixed(1)} cores</span>
              </div>
              <div className="h-2.5 bg-[var(--zf-canvas)] rounded-full overflow-hidden">
                <div className={`h-full rounded-full ${barColor(cpuPct)}`} style={{ width: `${cpuPct}%` }} />
              </div>
              <p className="text-xs text-[var(--zf-muted)] mt-2">{data.free_cpu.toFixed(1)} cores free</p>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5">
              <div className="flex justify-between text-sm mb-2">
                <span className="text-[var(--zf-muted)]">Memory</span>
                <span className="text-[var(--zf-ink)] font-medium">{data.used_memory_gib.toFixed(1)} / {data.total_memory_gib.toFixed(1)} GiB</span>
              </div>
              <div className="h-2.5 bg-[var(--zf-canvas)] rounded-full overflow-hidden">
                <div className={`h-full rounded-full ${barColor(memPct)}`} style={{ width: `${memPct}%` }} />
              </div>
              <p className="text-xs text-[var(--zf-muted)] mt-2">{data.free_memory_gib.toFixed(1)} GiB free</p>
            </div>
          </div>

          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3">
              <div className="text-2xl font-bold text-[var(--zf-ink)]">{data.node_count}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Nodes</div>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3">
              <div className="text-2xl font-bold text-[var(--zf-ink)]">{data.vm_count}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">VMs</div>
            </div>
          </div>

          <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5">
            <h3 className="text-sm font-semibold text-[var(--zf-ink)] mb-3">How many more VMs would fit?</h3>
            <div className="flex flex-wrap items-end gap-3">
              <div>
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">vCPUs</label>
                <input type="number" min={1} value={fitCpu} onChange={(e) => setFitCpu(e.target.value)} className="input-field text-sm w-24" />
              </div>
              <div>
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Memory (GiB)</label>
                <input type="number" min={1} value={fitMemory} onChange={(e) => setFitMemory(e.target.value)} className="input-field text-sm w-24" />
              </div>
              <button type="button" onClick={handleFit} disabled={fitting} className="zf-btn zf-btn-primary zf-btn-sm">
                {fitting ? 'Estimating…' : 'Estimate'}
              </button>
              {fitResult !== null && (
                <span className="text-sm text-[var(--zf-ink)]">≈ <span className="font-semibold">{fitResult}</span> more VMs of this size</span>
              )}
            </div>
            <p className="text-xs text-[var(--zf-muted)] mt-3">A per-node capacity estimate, not a scheduling guarantee — it doesn't account for taints, affinity, or other pods competing for the same headroom.</p>
          </div>

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <div className="px-5 py-4 border-b border-[var(--zf-hairline)]"><h3 className="text-sm font-semibold text-[var(--zf-ink)]">Per-Node Breakdown</h3></div>
            <DataTable
              columns={[
                { key: 'node', header: 'Node', className: 'px-5', render: (n) => <span className="text-[var(--zf-ink)] font-medium">{n.name}</span> },
                { key: 'cpu', header: 'CPU', render: (n) => <span className="text-xs text-[var(--zf-muted)]">{n.used_cpu.toFixed(1)} / {n.allocatable_cpu.toFixed(1)}</span> },
                { key: 'memory', header: 'Memory', render: (n) => <span className="text-xs text-[var(--zf-muted)]">{n.used_memory_gib.toFixed(1)} / {n.allocatable_memory_gib.toFixed(1)} GiB</span> },
                { key: 'vms', header: 'VMs', render: (n) => <span className="text-xs text-[var(--zf-muted)]">{n.vm_count}</span> },
                {
                  key: 'status',
                  header: 'Status',
                  render: (n) => (
                    <span className="text-xs">
                      {n.unschedulable ? <span className="text-[var(--zf-warning)]">Unschedulable</span> : <span className="text-[var(--zf-success)]">Ready</span>}
                    </span>
                  ),
                },
              ]}
              rows={data.nodes}
              getRowKey={(n) => n.name}
              bordered={false}
            />
          </div>
        </>
      ) : !loadError ? (
        <div className="bg-[var(--zf-surface)] rounded-xl p-10 border border-[var(--zf-hairline)] text-center text-[var(--zf-muted)]">
          <Gauge className="w-10 h-10 mx-auto mb-3 opacity-50" />
          No capacity data available
        </div>
      ) : null}
    </div>
  )
}
