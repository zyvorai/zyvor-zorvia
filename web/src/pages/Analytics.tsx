// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { BarChart3 } from 'lucide-react'
import { getTopVms, TopVmEntry } from '../api/analytics'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

type Metric = 'cpu_usage' | 'memory_usage' | 'disk_usage'

const METRIC_LABEL: Record<Metric, string> = {
  cpu_usage: 'CPU %',
  memory_usage: 'Memory (bytes)',
  disk_usage: 'Disk (bytes)',
}

function formatValue(metric: Metric, value: number): string {
  if (metric === 'cpu_usage') return `${value.toFixed(1)}%`
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MB`
  return `${value.toFixed(0)} B`
}

export default function Analytics() {
  const toast = useToastContext()
  const [metric, setMetric] = useState<Metric>('cpu_usage')
  const [rows, setRows] = useState<TopVmEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const fetchTop = useCallback(async (m: Metric) => {
    setLoading(true)
    setLoadError(null)
    try {
      setRows(await getTopVms(m, 10))
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load analytics', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchTop(metric) }, [fetchTop, metric])

  const maxValue = Math.max(1, ...rows.map(r => r[metric]))

  return (
    <div className="space-y-6">
      <PageHeader
        title="Analytics"
        description="Top VMs by real, live usage — the same numbers each VM's own metrics panel reports, ranked across the fleet"
        onRefresh={() => fetchTop(metric)}
        refreshing={loading}
      />

      <div className="flex gap-2">
        {(Object.keys(METRIC_LABEL) as Metric[]).map(m => (
          <button key={m} onClick={() => setMetric(m)} className={`zf-chip ${metric === m? 'zf-chip-active' : ''}`}>
            {METRIC_LABEL[m]}
          </button>
        ))}
      </div>

      {loadError && (
        <ErrorBanner title="Could not load analytics" headline={loadError} hints={hintsForError(loadError)} onRetry={() => fetchTop(metric)} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading…
        </div>
      ) : !loadError && rows.length === 0 ? (
        <EmptyState icon={<BarChart3 className="w-8 h-8" />} title="No VMs found" description="Create a VM to see usage rankings here." />
      ) : !loadError ? (
        <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5 space-y-3">
          {rows.map(row => (
            <div key={row.vm_name} className="flex items-center gap-3">
              <span className="text-sm text-[var(--zf-ink)] w-32 shrink-0 truncate">{row.vm_name}</span>
              <div className="flex-1 bg-[var(--zf-canvas)] rounded-full h-6 overflow-hidden">
                <div
                  className="bg-[var(--zf-link)] h-full rounded-full flex items-center justify-end pr-2 transition-all"
                  style={{ width: `${Math.max(5, (row[metric] / maxValue) * 100)}%` }}
                >
                  <span className="text-xs text-white font-medium whitespace-nowrap">{formatValue(metric, row[metric])}</span>
                </div>
              </div>
              <span className="text-[10px] text-[var(--zf-muted)] w-20 shrink-0">{row.source}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
