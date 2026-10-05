// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router'
import { MonitorCog, MonitorPlay, Play, RefreshCw, Square, Trash2, Camera, Plus, Layers } from 'lucide-react'
import { PageHeader, StatusBadge, EmptyState, DataTable } from '../components/ui'
import ErrorBanner from '../components/ErrorBanner'
import ConfirmDialog from '../components/ConfirmDialog'
import { useConfirm } from '../hooks/useConfirm'
import { formatUserError } from '../utils/apiError'
import { downloadRdpFile } from '../utils/rdp'
import {
  createKrytonMachine,
  deleteKrytonMachine,
  getKrytonImages,
  getKrytonStatus,
  getKrytonSummary,
  listKrytonMachines,
  snapshotKrytonMachine,
  startKrytonMachine,
  stopKrytonMachine,
  type KrytonImage,
  type KrytonMachine,
  type KrytonStatus,
  type KrytonSummary,
} from '../api/kryton'

const initialForm = { name: '', image: '', cpu: 4, memoryMiB: 8192, diskGiB: 80, ttlMinutes: 0 }

export default function KrytonWindows() {
  const { confirmState, confirm, cancel } = useConfirm()
  const [status, setStatus] = useState<KrytonStatus | null>(null)
  const [summary, setSummary] = useState<KrytonSummary | null>(null)
  const [machines, setMachines] = useState<KrytonMachine[]>([])
  const [images, setImages] = useState<KrytonImage[]>([])
  const [form, setForm] = useState(initialForm)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const project = status?.project || undefined

  const refresh = useCallback(async () => {
    try {
      const nextStatus = await getKrytonStatus()
      setStatus(nextStatus)
      if (!nextStatus.enabled || !nextStatus.connected) {
        setMachines([])
        setSummary(null)
        setImages([])
        return
      }
      const effectiveProject = nextStatus.project || undefined
      if (!effectiveProject) {
        const imagePage = await getKrytonImages()
        setMachines([])
        setSummary(null)
        setImages(imagePage.items.filter((image) => image.ready))
        setForm((current) => ({ ...current, image: current.image || imagePage.items.find((image) => image.ready)?.id || '' }))
        setError('Kryton is reachable, but KRYTON_PROJECT is not configured on the Zorvia server.')
        return
      }
      const [machinePage, imagePage, nextSummary] = await Promise.all([
        listKrytonMachines(effectiveProject),
        getKrytonImages(),
        getKrytonSummary(effectiveProject),
      ])
      setMachines(machinePage.items)
      setImages(imagePage.items.filter((image) => image.ready))
      setSummary(nextSummary)
      setForm((current) => ({ ...current, image: current.image || imagePage.items.find((image) => image.ready)?.id || '' }))
      setError(null)
    } catch (e) {
      setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  const readyImages = useMemo(() => images.filter((image) => image.ready), [images])

  const run = async (key: string, operation: () => Promise<unknown>) => {
    setBusy(key)
    try {
      await operation()
      await refresh()
    } catch (e) {
      setError(formatUserError(e))
    } finally {
      setBusy(null)
    }
  }

  const handleDelete = async (machine: KrytonMachine) => {
    if (!await confirm('Delete Machine', `Delete "${machine.spec.name}"?`, { variant: 'danger', confirmLabel: 'Delete' })) return
    await run(`${machine.id}:delete`, () => deleteKrytonMachine(machine.id, machine.project))
  }

  const create = async (e: FormEvent) => {
    e.preventDefault()
    if (!project) {
      setError('Configure KRYTON_PROJECT on the Zorvia server before creating Windows machines.')
      return
    }
    await run('create', () => createKrytonMachine({
      project,
      name: form.name,
      image: form.image,
      compute: { cpu: form.cpu, memoryMiB: form.memoryMiB },
      disk: { sizeGiB: form.diskGiB },
      ttlMinutes: form.ttlMinutes || undefined,
    }))
    setForm((current) => ({ ...current, name: '' }))
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Windows"
        description="Kryton-backed Windows virtualization inside Zorvia. KubeVirt, dockur and provider details stay behind the control-plane boundary."
        icon={MonitorCog}
        actions={(
          <>
            <Link className="zf-btn zf-btn-secondary" to="/app/golden-images">
              <Layers className="w-3.5 h-3.5" /> Build golden image
            </Link>
            <button className="zf-btn zf-btn-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} /> Refresh
            </button>
          </>
        )}
      />

      {error && <ErrorBanner title="Kryton error" headline={error} onDismiss={() => setError(null)} />}

      <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
        {[
          ['Connection', status?.connected ? 'Connected' : status?.enabled ? 'Degraded' : 'Disabled'],
          ['Provider', summary?.provider || '—'],
          ['Machines', summary?.machines ?? machines.length],
          ['Running', summary?.running ?? machines.filter((m) => m.state === 'running').length],
          ['Project', status?.project || 'Not configured'],
        ].map(([label, value]) => (
          <div key={String(label)} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-4">
            <div className="text-xs text-[var(--zf-muted)]">{label}</div>
            <div className="mt-1 text-lg font-semibold text-[var(--zf-ink)] truncate">{String(value)}</div>
          </div>
        ))}
      </div>

      {status?.enabled && status.connected && project && (
        <form onSubmit={create} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5 space-y-4">
          <div className="flex items-center gap-2">
            <Plus className="w-4 h-4" />
            <h2 className="font-semibold">Create Windows machine</h2>
          </div>
          <div className="grid md:grid-cols-6 gap-3">
            <label className="md:col-span-2 text-xs text-[var(--zf-muted)]">Name
              <input required pattern="[a-z0-9]([-a-z0-9]*[a-z0-9])?" maxLength={63} className="input-field mt-1 w-full" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="win11-finance-01" />
            </label>
            <label className="md:col-span-2 text-xs text-[var(--zf-muted)]">Image
              <select required className="input-field mt-1 w-full" value={form.image} onChange={(e) => setForm({ ...form, image: e.target.value })}>
                <option value="">Select image</option>
                {readyImages.map((image) => <option key={image.id} value={image.id}>{image.name} · {image.version}</option>)}
              </select>
            </label>
            <label className="text-xs text-[var(--zf-muted)]">CPU
              <input type="number" min={1} max={256} className="input-field mt-1 w-full" value={form.cpu} onChange={(e) => setForm({ ...form, cpu: Number(e.target.value) })} />
            </label>
            <label className="text-xs text-[var(--zf-muted)]">Memory MiB
              <input type="number" min={512} className="input-field mt-1 w-full" value={form.memoryMiB} onChange={(e) => setForm({ ...form, memoryMiB: Number(e.target.value) })} />
            </label>
            <label className="text-xs text-[var(--zf-muted)]">Disk GiB
              <input type="number" min={16} className="input-field mt-1 w-full" value={form.diskGiB} onChange={(e) => setForm({ ...form, diskGiB: Number(e.target.value) })} />
            </label>
            <label className="text-xs text-[var(--zf-muted)]">TTL minutes
              <input type="number" min={0} max={43200} className="input-field mt-1 w-full" value={form.ttlMinutes} onChange={(e) => setForm({ ...form, ttlMinutes: Number(e.target.value) })} />
            </label>
          </div>
          <button className="zf-btn zf-btn-primary" disabled={busy === 'create' || !form.image || !form.name}>
            {busy === 'create' ? 'Creating…' : 'Create in Kryton'}
          </button>
        </form>
      )}

      <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
        <div className="px-4 py-3 border-b border-[var(--zf-hairline)] flex items-center justify-between">
          <h2 className="font-semibold">Windows machines</h2>
          <span className="text-xs text-[var(--zf-muted)]">Stable Kryton UUIDs</span>
        </div>
        <DataTable
          columns={[
            {
              key: 'machine',
              header: 'Machine',
              render: (machine) => (
                <>
                  <div className="font-medium">{machine.spec.name}</div>
                  <div className="text-[11px] text-[var(--zf-muted)] font-mono">{machine.id}</div>
                </>
              ),
            },
            { key: 'state', header: 'State', render: (machine) => <StatusBadge status={machine.state} /> },
            {
              key: 'compute',
              header: 'Compute',
              render: (machine) => <span className="text-sm text-[var(--zf-muted)]">{machine.spec.compute.cpu} vCPU · {(machine.spec.compute.memoryMiB / 1024).toFixed(1)} GiB</span>,
            },
            {
              key: 'ip',
              header: 'IP / RDP',
              render: (machine) => (
                <div className="text-sm text-[var(--zf-muted)]">
                  <div>{machine.ipAddresses?.[0] || '—'}</div>
                  <div className="text-[11px]">{machine.rdpHost ? `RDP ${machine.rdpHost}:${machine.rdpPort || 3389}` : ''}</div>
                </div>
              ),
            },
            { key: 'provider', header: 'Provider', render: (machine) => <span className="text-sm text-[var(--zf-muted)]">{machine.provider}</span> },
            {
              key: 'actions',
              header: 'Actions',
              render: (machine) => {
                const key = machine.id
                const actionBusy = busy?.startsWith(`${key}:`) === true
                return (
                  <div className="flex gap-1">
                    {machine.state === 'stopped' || machine.state === 'failed' ? (
                      <button title="Start" className="zf-btn zf-btn-ghost zf-btn-sm !px-2" disabled={actionBusy} onClick={() => void run(`${key}:start`, () => startKrytonMachine(key, machine.project))}><Play className="w-3.5 h-3.5" /></button>
                    ) : (
                      <button title="Stop" className="zf-btn zf-btn-ghost zf-btn-sm !px-2" disabled={actionBusy} onClick={() => void run(`${key}:stop`, () => stopKrytonMachine(key, machine.project))}><Square className="w-3.5 h-3.5" /></button>
                    )}
                    <button title="Snapshot" className="zf-btn zf-btn-ghost zf-btn-sm !px-2" disabled={actionBusy} onClick={() => void run(`${key}:snapshot`, () => snapshotKrytonMachine(key, machine.project))}><Camera className="w-3.5 h-3.5" /></button>
                    {machine.rdpHost && (
                      <button title="Download .rdp file" className="zf-btn zf-btn-ghost zf-btn-sm !px-2" onClick={() => downloadRdpFile(machine.rdpHost!, machine.rdpPort || 3389, machine.rdpUsername)}><MonitorPlay className="w-3.5 h-3.5" /></button>
                    )}
                    <button title="Delete" className="zf-btn zf-btn-ghost zf-btn-sm !px-2 text-[var(--zf-danger)]" disabled={actionBusy} onClick={() => void handleDelete(machine)}><Trash2 className="w-3.5 h-3.5" /></button>
                  </div>
                )
              },
            },
          ]}
          rows={machines}
          getRowKey={(machine) => machine.id}
          loading={loading}
          bordered={false}
          emptyState={<EmptyState icon={<MonitorCog className="w-10 h-10" />} title="No Windows machines" description="No Windows machines are visible in this Kryton project." />}
        />
      </div>

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
