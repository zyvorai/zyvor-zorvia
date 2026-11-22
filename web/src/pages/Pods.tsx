// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  ExternalLink,
  FileCode2,
  Maximize2,
  Minimize2,
  RotateCw,
  ScrollText,
  Search,
  SquareTerminal,
  Trash2,
  X,
} from 'lucide-react'
import { DataTable, EmptyState, PageHeader, StatusBadge, type DataTableColumn } from '../components/ui'
import ErrorBanner from '../components/ErrorBanner'
import ConfirmDialog from '../components/ConfirmDialog'
import { useConfirm } from '../hooks/useConfirm'
import { useToastContext } from '../contexts/ToastContext'
import { toastFailure } from '../utils/toastError'
import {
  ALL_POD_CAPABILITIES,
  deletePod,
  listNamespaces,
  listPods,
  podCapabilities,
  podLogsPagePath,
  restartPod,
  type PodCapabilities,
  type PodSummary,
} from '../api/pods'

const PodLogs = lazy(() => import('../components/PodLogs'))
const PodExec = lazy(() => import('../components/PodExec'))
const PodEvents = lazy(() => import('../components/PodEvents'))
const PodYaml = lazy(() => import('../components/PodYaml'))

const REFRESH_MS = 10_000
type Filter = 'all' | 'running' | 'pending' | 'failed'
type Mode = 'logs' | 'exec' | 'events' | 'yaml'

/** Owners that recreate a deleted pod (must match `restartable_owner` in pod_handlers.rs). */
const RESTARTABLE_OWNERS = new Set(['ReplicaSet', 'StatefulSet', 'DaemonSet', 'ReplicationController'])
const canRestart = (p: PodSummary) => !!p.owner_kind && RESTARTABLE_OWNERS.has(p.owner_kind)

const MODES: { mode: Mode; label: string; icon: typeof ScrollText }[] = [
  { mode: 'logs', label: 'Logs', icon: ScrollText },
  { mode: 'exec', label: 'Terminal', icon: SquareTerminal },
  { mode: 'events', label: 'Events', icon: Activity },
  { mode: 'yaml', label: 'YAML', icon: FileCode2 },
]

function badgeKey(pod: PodSummary): string {
  const s = pod.status
  if (s === 'Running') return 'running'
  if (s === 'Succeeded' || s === 'Completed') return 'completed'
  if (s === 'Terminating') return 'paused'
  if (/BackOff|Err|Fail|OOMKilled|Evicted|Unknown/i.test(s)) return 'failed'
  return 'pending'
}

function matchesFilter(pod: PodSummary, f: Filter): boolean {
  if (f === 'all') return true
  const k = badgeKey(pod)
  if (f === 'running') return k === 'running'
  if (f === 'failed') return k === 'failed'
  return k === 'pending'
}

function formatAge(secs: number | null): string {
  if (secs === null) return '—'
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h`
  return `${Math.floor(secs / 86_400)}d`
}

function containerNames(pod: PodSummary): string[] {
  const main = pod.containers.filter((c) => !c.init).map((c) => c.name)
  const init = pod.containers.filter((c) => c.init).map((c) => c.name)
  return [...main, ...init]
}

const podKey = (p: PodSummary) => `${p.namespace}/${p.name}`

/** ClusterRole rule each capability needs — shown when the service account lacks it. */
const MISSING_RULE: Record<keyof PodCapabilities, { rule: string; feature: string }> = {
  logs: { rule: 'pods/log: get', feature: 'Logs' },
  exec: { rule: 'pods/exec: create, get', feature: 'Terminal' },
  delete: { rule: 'pods: delete', feature: 'Restart and Delete' },
}
const noRbac = (cap: keyof PodCapabilities) => `Zorvia's ClusterRole is missing "${MISSING_RULE[cap].rule}"`

export default function Pods() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [busy, setBusy] = useState<string | null>(null)
  const [caps, setCaps] = useState<PodCapabilities>(ALL_POD_CAPABILITIES)
  const [pods, setPods] = useState<PodSummary[]>([])
  const [namespaces, setNamespaces] = useState<string[]>([])
  const [namespace, setNamespace] = useState('')
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<Filter>('all')
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [active, setActive] = useState<{ key: string; mode: Mode; container: string } | null>(null)
  const [panelHeight, setPanelHeight] = useState(460)
  const [maximized, setMaximized] = useState(false)
  const panelRef = useRef<HTMLDivElement>(null)

  const load = useCallback(
    async (quiet = false) => {
      if (!quiet) setRefreshing(true)
      try {
        setPods(await listPods(namespace || undefined))
        setError(null)
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        setLoading(false)
        setRefreshing(false)
      }
    },
    [namespace],
  )

  useEffect(() => {
    void load()
    const t = window.setInterval(() => void load(true), REFRESH_MS)
    return () => window.clearInterval(t)
  }, [load])

  useEffect(() => {
    listNamespaces().then(setNamespaces).catch(() => setNamespaces([]))
    podCapabilities().then(setCaps).catch(() => setCaps(ALL_POD_CAPABILITIES))
  }, [])

  const missing = (Object.keys(MISSING_RULE) as (keyof PodCapabilities)[]).filter((k) => !caps[k])
  const restartBlock = (p: PodSummary) =>
    !caps.delete ? noRbac('delete') : !canRestart(p) ? 'Not controller-managed' : null

  useEffect(() => {
    if (!maximized) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMaximized(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [maximized])

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase()
    return pods.filter(
      (p) =>
        matchesFilter(p, filter) &&
        (!q ||
          p.name.toLowerCase().includes(q) ||
          p.namespace.toLowerCase().includes(q) ||
          (p.node ?? '').toLowerCase().includes(q)),
    )
  }, [pods, query, filter])

  const counts = useMemo(() => {
    const c = { running: 0, pending: 0, failed: 0 }
    for (const p of pods) {
      const k = badgeKey(p)
      if (k === 'running') c.running++
      else if (k === 'failed') c.failed++
      else if (k === 'pending') c.pending++
    }
    return c
  }, [pods])

  const activePod = active ? pods.find((p) => podKey(p) === active.key) : undefined

  const open = (pod: PodSummary, mode: Mode) => {
    const names = containerNames(pod)
    const keep = active && active.key === podKey(pod) && names.includes(active.container)
    setActive({ key: podKey(pod), mode, container: keep ? active!.container : names[0] ?? '' })
    requestAnimationFrame(() => panelRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }))
  }

  const removePod = async (pod: PodSummary) => {
    const managed = canRestart(pod)
    const ok = await confirm(
      `Delete pod ${pod.name}?`,
      managed
        ? `${pod.namespace}/${pod.name} will be deleted. Its ${pod.owner_kind} will create a replacement.`
        : `${pod.namespace}/${pod.name} is not managed by a controller — it will be gone for good.`,
      { confirmLabel: 'Delete pod', variant: 'danger' },
    )
    if (!ok) return
    setBusy(podKey(pod))
    try {
      await deletePod(pod.namespace, pod.name)
      toast.success(`Deleted ${pod.name}`)
      if (active?.key === podKey(pod)) {
        setActive(null)
        setMaximized(false)
      }
      await load(true)
    } catch (e) {
      toastFailure(toast, `Could not delete ${pod.name}`, e)
    } finally {
      setBusy(null)
    }
  }

  const restart = async (pod: PodSummary) => {
    const ok = await confirm(
      `Restart pod ${pod.name}?`,
      `The pod is deleted and its ${pod.owner_kind} schedules a fresh replacement (new name, same spec). Running sessions and in-memory state are lost.`,
      { confirmLabel: 'Restart', variant: 'warning' },
    )
    if (!ok) return
    setBusy(podKey(pod))
    try {
      const res = await restartPod(pod.namespace, pod.name)
      toast.success(`Restarting ${pod.name} — ${res.owner_kind} ${res.owner} will recreate it`)
      if (active?.key === podKey(pod)) setActive(null)
      await load(true)
    } catch (e) {
      toastFailure(toast, `Could not restart ${pod.name}`, e)
    } finally {
      setBusy(null)
    }
  }

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault()
    const startY = e.clientY
    const startH = panelHeight
    const move = (ev: PointerEvent) =>
      setPanelHeight(Math.min(Math.max(240, startH + ev.clientY - startY), window.innerHeight - 120))
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  const columns: DataTableColumn<PodSummary>[] = [
    {
      key: 'name',
      header: 'Name',
      sortable: true,
      sortValue: (p) => p.name,
      render: (p) => (
        <div className="min-w-0">
          <div className="font-medium text-[var(--zf-ink)] truncate max-w-[28rem]" title={p.name}>
            {p.name}
          </div>
          {p.owner_kind && <div className="text-xs text-[var(--zf-tertiary)]">{p.owner_kind}</div>}
        </div>
      ),
    },
    {
      key: 'namespace',
      header: 'Namespace',
      sortable: true,
      sortValue: (p) => p.namespace,
      render: (p) => <span className="text-[var(--zf-secondary)]">{p.namespace}</span>,
    },
    {
      key: 'status',
      header: 'Status',
      sortable: true,
      sortValue: (p) => p.status,
      render: (p) => <StatusBadge status={badgeKey(p)} label={p.status} />,
    },
    { key: 'ready', header: 'Ready', render: (p) => <span className="tabular-nums">{p.ready}</span> },
    {
      key: 'restarts',
      header: 'Restarts',
      sortable: true,
      sortValue: (p) => p.restarts,
      render: (p) => (
        <span className={`tabular-nums ${p.restarts > 0 ? 'text-[var(--zf-warning-text)]' : ''}`}>{p.restarts}</span>
      ),
    },
    {
      key: 'age',
      header: 'Age',
      sortable: true,
      sortValue: (p) => p.age_seconds ?? 0,
      render: (p) => <span className="tabular-nums text-[var(--zf-secondary)]">{formatAge(p.age_seconds)}</span>,
    },
    {
      key: 'node',
      header: 'Node',
      render: (p) => <span className="text-[var(--zf-secondary)]">{p.node ?? '—'}</span>,
    },
    {
      key: 'actions',
      header: '',
      className: 'text-right whitespace-nowrap',
      render: (p) => (
        <div className="inline-flex gap-2" onClick={(e) => e.stopPropagation()}>
          <button
            type="button"
            className="zf-btn zf-btn-secondary zf-btn-xs"
            onClick={() => open(p, 'logs')}
            disabled={!caps.logs}
            title={caps.logs ? 'Stream logs' : noRbac('logs')}
          >
            <ScrollText className="w-3.5 h-3.5" />
            Logs
          </button>
          <button
            type="button"
            className="zf-btn zf-btn-secondary zf-btn-xs"
            onClick={() => open(p, 'exec')}
            disabled={!caps.exec || p.phase !== 'Running'}
            title={!caps.exec ? noRbac('exec') : p.phase !== 'Running' ? 'Pod is not running' : 'Open a shell'}
          >
            <SquareTerminal className="w-3.5 h-3.5" />
            Terminal
          </button>
          <button
            type="button"
            className="console-icon-btn"
            onClick={() => void restart(p)}
            disabled={!!restartBlock(p) || busy === podKey(p)}
            aria-label={`Restart ${p.name}`}
            title={restartBlock(p) ?? 'Restart (delete and let the controller recreate it)'}
          >
            <RotateCw className="w-4 h-4" />
          </button>
          <button
            type="button"
            className="console-icon-btn"
            onClick={() => void removePod(p)}
            disabled={!caps.delete || busy === podKey(p)}
            aria-label={`Delete ${p.name}`}
            title={caps.delete ? 'Delete pod' : noRbac('delete')}
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      ),
    },
  ]

  const names = activePod ? containerNames(activePod) : []

  return (
    <div className="space-y-6">
      <PageHeader
        eyebrow="Ops"
        title="Pods"
        description="Every pod in the cluster. Stream colorized logs, open a shell, inspect events and YAML — Terminal.app style."
        onRefresh={() => void load()}
        refreshing={refreshing}
      />

      {error && (
        <ErrorBanner
          title="Could not load pods"
          headline={error}
          hints={['Pods requires the cluster.admin permission and pods list access for the Zorvia service account.']}
          onRetry={() => void load()}
        />
      )}

      {missing.length > 0 && (
        <div
          role="status"
          data-testid="pods-rbac-notice"
          className="rounded-xl border px-4 py-3 text-sm bg-[var(--zf-warning-bg)] border-[var(--zf-warning-border)] text-[var(--zf-warning-text)]"
        >
          <strong>{missing.map((k) => MISSING_RULE[k].feature).join(', ')} unavailable</strong> — the Zorvia service
          account lacks {missing.map((k) => `"${MISSING_RULE[k].rule}"`).join(', ')}. Re-apply the Zorvia ClusterRole
          (Helm upgrade or <code>kubectl apply -f deploy/k8s.yaml</code>); see{' '}
          <a
            className="underline"
            href="https://github.com/zyvorai/zorvia/blob/main/docs/UPGRADE.md"
            target="_blank"
            rel="noopener"
          >
            UPGRADE.md
          </a>
          .
        </div>
      )}

      <div className="toolbar-pill">
        <select
          aria-label="Namespace"
          className="input-field"
          style={{ flex: '0 0 220px', borderRadius: 980, minHeight: 36, padding: '6px 14px' }}
          value={namespace}
          onChange={(e) => setNamespace(e.target.value)}
        >
          <option value="">All namespaces</option>
          {namespaces.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
        <div className="relative flex-1 min-w-[220px]">
          <Search className="w-4 h-4 absolute left-3.5 top-1/2 -translate-y-1/2 text-[var(--zf-tertiary)] pointer-events-none" />
          <input
            aria-label="Search pods"
            className="input-field"
            style={{ borderRadius: 980, minHeight: 36, padding: '6px 14px 6px 36px' }}
            placeholder="Search pods, namespaces, nodes"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="toolbar-pill__trailing">
          {(
            [
              ['all', `All ${pods.length}`],
              ['running', `Running ${counts.running}`],
              ['pending', `Pending ${counts.pending}`],
              ['failed', `Failed ${counts.failed}`],
            ] as [Filter, string][]
          ).map(([f, label]) => (
            <button
              key={f}
              type="button"
              className={`zf-chip ${filter === f ? 'zf-chip-active' : ''}`}
              aria-pressed={filter === f}
              onClick={() => setFilter(f)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <DataTable
        columns={columns}
        rows={visible}
        getRowKey={podKey}
        loading={loading}
        onRowClick={(p) => open(p, caps.logs ? 'logs' : 'events')}
        rowClassName={(p) => (active?.key === podKey(p) ? 'selected' : '')}
        emptyState={
          <EmptyState
            title={pods.length ? 'No pods match' : 'No pods'}
            description={pods.length ? 'Try another namespace, search or status filter.' : 'Nothing is scheduled here yet.'}
          />
        }
      />

      {active && activePod && (
        <div
          ref={panelRef}
          className={maximized ? 'fixed inset-x-3 bottom-3 z-[60] flex flex-col' : 'flex flex-col'}
          style={maximized ? { top: 'calc(var(--console-topbar-height) + 12px)' } : { height: panelHeight }}
          data-testid="pod-panel"
        >
          <div className="flex items-center gap-2 mb-2 flex-wrap">
            {MODES.map(({ mode, label, icon: Icon }) => (
              <button
                key={mode}
                type="button"
                className={`zf-chip ${active.mode === mode ? 'zf-chip-active' : ''}`}
                aria-pressed={active.mode === mode}
                onClick={() => setActive({ ...active, mode })}
                disabled={
                  (mode === 'exec' && (!caps.exec || activePod.phase !== 'Running')) || (mode === 'logs' && !caps.logs)
                }
                title={
                  mode === 'exec' && !caps.exec ? noRbac('exec') : mode === 'logs' && !caps.logs ? noRbac('logs') : undefined
                }
              >
                <Icon className="w-3.5 h-3.5" /> {label}
              </button>
            ))}
            <span className="ml-2 text-sm text-[var(--zf-secondary)] truncate min-w-0">
              {activePod.namespace}/{activePod.name}
            </span>
            <div className="ml-auto flex items-center gap-1">
              <a
                className="console-icon-btn"
                href={podLogsPagePath(activePod.namespace, activePod.name, active.container)}
                target="_blank"
                rel="noopener"
                aria-label="Open logs in new tab"
                title="Open logs in new tab"
              >
                <ExternalLink className="w-4 h-4" />
              </a>
              <button
                type="button"
                className="console-icon-btn"
                onClick={() => void restart(activePod)}
                disabled={!!restartBlock(activePod) || busy === active.key}
                aria-label="Restart pod"
                title={restartBlock(activePod) ?? 'Restart pod'}
              >
                <RotateCw className="w-4 h-4" />
              </button>
              <button
                type="button"
                className="console-icon-btn"
                onClick={() => void removePod(activePod)}
                disabled={!caps.delete || busy === active.key}
                aria-label="Delete pod"
                title={caps.delete ? 'Delete pod' : noRbac('delete')}
              >
                <Trash2 className="w-4 h-4" />
              </button>
              <span className="w-px h-5 bg-[var(--zf-hairline)] mx-1" aria-hidden />
              <button
                type="button"
                className="console-icon-btn"
                onClick={() => setMaximized((v) => !v)}
                aria-label={maximized ? 'Restore' : 'Maximize'}
              >
                {maximized ? <Minimize2 className="w-4 h-4" /> : <Maximize2 className="w-4 h-4" />}
              </button>
              <button
                type="button"
                className="console-icon-btn"
                onClick={() => {
                  setActive(null)
                  setMaximized(false)
                }}
                aria-label="Close panel"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
          </div>
          <div className="flex-1 min-h-0">
            <Suspense fallback={<div className="zf-terminal zf-terminal-pro rounded-xl h-full" />}>
              {active.mode === 'logs' && (
                <PodLogs
                  namespace={activePod.namespace}
                  pod={activePod.name}
                  containers={names}
                  container={active.container}
                  onContainerChange={(c) => setActive({ ...active, container: c })}
                />
              )}
              {active.mode === 'exec' && (
                <PodExec
                  namespace={activePod.namespace}
                  pod={activePod.name}
                  containers={names}
                  container={active.container}
                  onContainerChange={(c) => setActive({ ...active, container: c })}
                />
              )}
              {active.mode === 'events' && <PodEvents namespace={activePod.namespace} pod={activePod.name} />}
              {active.mode === 'yaml' && <PodYaml namespace={activePod.namespace} pod={activePod.name} />}
            </Suspense>
          </div>
          {!maximized && (
            <div
              role="separator"
              aria-orientation="horizontal"
              aria-label="Resize terminal"
              className="h-3 mt-1 cursor-row-resize flex items-center justify-center"
              onPointerDown={startResize}
            >
              <span className="w-10 h-1 rounded-full bg-[var(--zf-hairline-strong)]" />
            </div>
          )}
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
