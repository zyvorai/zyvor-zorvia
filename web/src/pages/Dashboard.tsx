// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState, useCallback } from 'react'
import { listVMs, VM } from '../api/vm'
import { ArrowUpRight } from 'lucide-react'
import { useWebSocketContext } from '../contexts/WebSocketContext'
import { useToastContext } from '../contexts/ToastContext'
import { SkeletonDashboard } from '../components/Skeleton'
import { StatusBadge, DataTable, type DataTableColumn } from '../components/ui'
import { Link } from 'react-router'
import ErrorBanner from '../components/ErrorBanner'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { GettingStarted } from '../components/GettingStarted'
import OnboardingChecklist from '../components/OnboardingChecklist'
import { useCountUp } from '../hooks/useCountUp'

const recentVMColumns: DataTableColumn<VM>[] = [
  {
    key: 'name',
    header: 'Name',
    render: (vm) => (
      <Link
        to={`/app/vms/${vm.name}`}
        className="font-medium text-[var(--zf-ink)] hover:text-[var(--zf-link)] transition-colors"
      >
        {vm.name}
      </Link>
    ),
  },
  { key: 'status', header: 'Status', render: (vm) => <StatusBadge status={vm.state} /> },
  {
    key: 'cpu',
    header: 'CPU',
    render: (vm) => <span className="text-[var(--zf-muted)] tabular-nums">{vm.cpus} vCPU</span>,
  },
  {
    key: 'memory',
    header: 'Memory',
    render: (vm) => (
      <span className="text-[var(--zf-muted)] tabular-nums">
        {vm.memory >= 1024 ? `${(vm.memory / 1024).toFixed(1)} GB` : `${vm.memory} MB`}
      </span>
    ),
  },
  {
    key: 'ip',
    header: 'IP',
    render: (vm) => (
      <span className="text-[var(--zf-muted)] font-mono text-xs">{vm.ip || '—'}</span>
    ),
  },
]

export default function Dashboard() {
  const [vms, setVMs] = useState<VM[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const { subscribe, vmUpdates } = useWebSocketContext()
  const toast = useToastContext()

  const loadVMs = useCallback(async () => {
    try {
      setVMs(await listVMs())
      setLoadError(null)
      setRefreshError(null)
    } catch (err) {
      const msg = formatUserError(err)
      setVMs((prev) => {
        if (prev.length === 0) {
          setLoadError(msg)
          toastFailure(toast, 'Failed to load virtual machines', err)
        } else {
          setRefreshError(msg)
        }
        return prev
      })
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => {
    void loadVMs()
  }, [loadVMs])
  useEffect(() => {
    const unsub = subscribe((msg) => {
      if (['vm_state_changed', 'vm_created', 'vm_deleted'].includes(msg.type)) loadVMs()
    })
    return unsub
  }, [subscribe, loadVMs])
  useEffect(() => {
    if (vmUpdates.size > 0) {
      setVMs((prev) =>
        prev.map((vm) => {
          const u = vmUpdates.get(vm.name)
          return u ? { ...vm, ...u } : vm
        }),
      )
    }
  }, [vmUpdates])

  const stats = {
    total: vms.length,
    running: vms.filter((v) => v.state === 'running').length,
    stopped: vms.filter((v) => v.state === 'stopped').length,
    totalCPU: vms.reduce((a, v) => a + v.cpus, 0),
    totalMem: vms.reduce((a, v) => a + v.memory, 0),
  }

  const totalCount = useCountUp(stats.total)
  const runningCount = useCountUp(stats.running)
  const stoppedCount = useCountUp(stats.stopped)
  const memCount = useCountUp(
    Math.round(stats.totalMem >= 1024 ? stats.totalMem / 1024 : stats.totalMem),
  )

  if (loading) return <SkeletonDashboard />

  const greeting = (() => {
    const h = new Date().getHours()
    return h < 5
      ? 'Good night'
      : h < 12
        ? 'Good morning'
        : h < 18
          ? 'Good afternoon'
          : 'Good evening'
  })()

  const memLabel = stats.totalMem >= 1024 ? 'GB memory' : 'MB memory'
  const stoppedTone = stats.stopped > 0 ? 'is-warn' : undefined
  const runningTone =
    stats.total > 0 && stats.running === 0 ? 'is-bad' : stats.running > 0 ? 'is-good' : undefined

  return (
    <div className="apple-section apple-section--tight space-y-8">
      <header className="page-hero">
        <p className="apple-eyebrow">{greeting}</p>
        <h1 className="apple-display">Zorvia</h1>
        <p className="apple-lede">
          {stats.total === 0
            ? 'Your private cloud control plane is ready. Create a VM to get started.'
            : `Watching ${stats.total} VM${stats.total === 1 ? '' : 's'}. ${stats.running} running right now.`}
        </p>
        <div className="apple-cta-row">
          <Link to="/app/create" className="zf-btn zf-btn-primary">
            Create VM
          </Link>
          <button type="button" onClick={() => void loadVMs()} className="zf-btn zf-btn-ghost">
            Refresh
          </button>
          {stats.total > 0 && (
            <Link to="/app/vms" className="apple-text-link">
              View all VMs
            </Link>
          )}
        </div>
      </header>

      {!loading && !loadError && <OnboardingChecklist hasVMs={vms.length > 0} />}

      {loadError && (
        <ErrorBanner
          title="Could not load dashboard"
          headline={loadError}
          hints={hintsForError(loadError)}
          onRetry={loadVMs}
        />
      )}
      {!loadError && refreshError && (
        <ErrorBanner
          title="Could not refresh dashboard"
          headline={refreshError}
          onRetry={loadVMs}
          tone="amber"
        />
      )}

      <div className="apple-metric-band" aria-label="Fleet summary">
        <div>
          <b>{totalCount}</b>
          <span>Total VMs</span>
        </div>
        <div>
          <b className={runningTone}>{runningCount}</b>
          <span>Running</span>
        </div>
        <div>
          <b className={stoppedTone}>{stoppedCount}</b>
          <span>Stopped</span>
        </div>
        <div>
          <b>
            {memCount}
            <span style={{ fontSize: '0.45em', fontWeight: 500, marginLeft: 6, color: 'var(--zf-muted)' }}>
              {memLabel.replace(' memory', '')}
            </span>
          </b>
          <span>
            {stats.totalCPU} vCPU · memory
          </span>
        </div>
      </div>

      {vms.length === 0 ? (
        <GettingStarted />
      ) : (
        <section className="space-y-3">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-[19px] font-semibold tracking-[-0.016em] text-[var(--zf-ink)]">
              Virtual Machines
            </h2>
            <Link to="/app/vms" className="apple-text-link inline-flex items-center gap-1 text-[15px]">
              View all <ArrowUpRight className="w-3.5 h-3.5" />
            </Link>
          </div>
          <DataTable
            columns={recentVMColumns}
            rows={vms.slice(0, 8)}
            getRowKey={(vm) => vm.name}
          />
        </section>
      )}
    </div>
  )
}
