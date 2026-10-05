// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router'
import { CloudDownload, FileJson, Hammer, Layers, RefreshCw, Rocket, X } from 'lucide-react'
import { PageHeader, StatusBadge, EmptyState, DataTable } from '../components/ui'
import ErrorBanner from '../components/ErrorBanner'
import { formatUserError } from '../utils/apiError'
import {
  bootstrapKrytonGolden,
  getKrytonCapabilities,
  getKrytonGoldenPassport,
  getKrytonImages,
  getKrytonStatus,
  listKrytonGolden,
  startKrytonGolden,
  type KrytonCapabilities,
  type KrytonGoldenBuild,
  type KrytonImage,
  type KrytonStatus,
} from '../api/kryton'
import { listDownloads, type DownloadStatus } from '../api/images'
import { listOperations, type Operation } from '../api/operations'
import { canBootstrap, isGoldenBuildBusy, windowsBuilderState, windowsGoldenImages } from '../utils/goldenImages'

const CAPTURE_KIND = 'golden-image-convert'
const POLL_MS = 5000

function shortSha(sha?: string): string {
  return sha ? `${sha.slice(0, 12)}…` : '—'
}

function Section({ title, aside, children }: { title: string; aside?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
      <div className="px-4 py-3 border-b border-[var(--zf-hairline)] flex items-center justify-between gap-3">
        <h2 className="font-semibold">{title}</h2>
        {aside && <span className="text-xs text-[var(--zf-muted)]">{aside}</span>}
      </div>
      {children}
    </div>
  )
}

export default function GoldenImages() {
  const [status, setStatus] = useState<KrytonStatus | null>(null)
  const [caps, setCaps] = useState<KrytonCapabilities | null>(null)
  const [images, setImages] = useState<KrytonImage[]>([])
  const [builds, setBuilds] = useState<KrytonGoldenBuild[]>([])
  const [downloads, setDownloads] = useState<DownloadStatus[]>([])
  const [captures, setCaptures] = useState<Operation[] | null>(null)
  const [form, setForm] = useState({ imageId: '', version: '' })
  const [passport, setPassport] = useState<{ id: string; data: unknown } | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextDownloads, nextCaptures] = await Promise.all([
        getKrytonStatus(),
        listDownloads().catch(() => [] as DownloadStatus[]),
        // Durable operations are cluster-admin only; other roles just don't see captures here.
        listOperations(CAPTURE_KIND).catch(() => null),
      ])
      setStatus(nextStatus)
      setDownloads(nextDownloads)
      setCaptures(nextCaptures)
      if (nextStatus.enabled && nextStatus.connected) {
        const [nextCaps, imagePage, buildPage] = await Promise.all([
          getKrytonCapabilities(),
          getKrytonImages(),
          listKrytonGolden(),
        ])
        setCaps(nextCaps)
        const windows = windowsGoldenImages(imagePage.items)
        setImages(windows)
        setBuilds(buildPage.items)
        setForm((current) => ({ ...current, imageId: current.imageId || windows[0]?.id || '' }))
      } else {
        setCaps(null)
        setImages([])
        setBuilds([])
      }
      setError(null)
    } catch (e) {
      setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { void refresh() }, [refresh])

  const polling = builds.some(isGoldenBuildBusy)
  useEffect(() => {
    if (!polling) return
    const timer = window.setInterval(() => { void refresh() }, POLL_MS)
    return () => window.clearInterval(timer)
  }, [polling, refresh])

  const builder = useMemo(() => windowsBuilderState(status, caps), [status, caps])
  const project = builder.kind === 'ready' ? builder.project : undefined

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

  const start = async (e: FormEvent) => {
    e.preventDefault()
    if (!project) return
    await run('start', () => startKrytonGolden({ imageId: form.imageId, version: form.version || undefined, auto: true }, project))
    setForm((current) => ({ ...current, version: '' }))
  }

  const showPassport = (build: KrytonGoldenBuild) =>
    run(`${build.id}:passport`, async () => setPassport({ id: build.id, data: await getKrytonGoldenPassport(build.id) }))

  const builderNotice = {
    disabled: 'Kryton integration is disabled. Set KRYTON_URL on the Zorvia server to build Windows golden images.',
    unreachable: `Kryton is not reachable${builder.kind === 'unreachable' && builder.error ? `: ${builder.error}` : '.'}`,
    'no-project': 'Kryton is reachable, but KRYTON_PROJECT is not configured on the Zorvia server.',
    unavailable: `The Kryton host (${builder.kind === 'unavailable' ? builder.provider : ''} provider) has no golden builder. It needs docker and /dev/kvm on the krytond host.`,
    ready: '',
  }[builder.kind]

  return (
    <div className="space-y-6">
      <PageHeader
        title="Golden images"
        description="Build Windows golden images through Kryton, and track Linux imports and VM captures. Every golden image ends up as a CDI DataSource that new VMs clone."
        icon={Layers}
        actions={(
          <button className="zf-btn zf-btn-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} /> Refresh
          </button>
        )}
      />

      {error && <ErrorBanner title="Golden images" headline={error} onDismiss={() => setError(null)} />}

      <Section title="Windows golden builds (Kryton)" aside="dockur install → Sysprep → capture → CDI DataSource">
        {builderNotice && <div className="px-4 py-3 text-sm text-[var(--zf-muted)] border-b border-[var(--zf-hairline)]">{builderNotice}</div>}
        {builder.kind === 'ready' && (
          <form onSubmit={start} className="px-4 py-4 border-b border-[var(--zf-hairline)] grid md:grid-cols-6 gap-3 items-end">
            <label className="md:col-span-3 text-xs text-[var(--zf-muted)]">Windows image
              <select required className="input-field mt-1 w-full" value={form.imageId} onChange={(e) => setForm({ ...form, imageId: e.target.value })}>
                <option value="">Select image</option>
                {images.map((image) => <option key={image.id} value={image.id}>{image.name} · {image.version}</option>)}
              </select>
            </label>
            <label className="md:col-span-2 text-xs text-[var(--zf-muted)]">Version label (optional)
              <input className="input-field mt-1 w-full" maxLength={63} value={form.version} onChange={(e) => setForm({ ...form, version: e.target.value })} placeholder="2026.10" />
            </label>
            <button className="zf-btn zf-btn-primary" disabled={busy === 'start' || !form.imageId}>
              <Hammer className="w-3.5 h-3.5" /> {busy === 'start' ? 'Starting…' : 'Build'}
            </button>
          </form>
        )}
        <DataTable
          columns={[
            {
              key: 'build',
              header: 'Build',
              render: (b) => (
                <>
                  <div className="font-medium">{b.imageId}{b.version ? ` · ${b.version}` : ''}</div>
                  <div className="text-[11px] text-[var(--zf-muted)] font-mono">{b.id}</div>
                </>
              ),
            },
            {
              key: 'state',
              header: 'State',
              render: (b) => (
                <div className="space-y-1">
                  <StatusBadge status={b.state} />
                  <div className="text-[11px] text-[var(--zf-muted)]">{b.error || b.message || b.phase || ''}{isGoldenBuildBusy(b) && b.progressPercent != null ? ` · ${b.progressPercent}%` : ''}</div>
                </div>
              ),
            },
            { key: 'sha', header: 'SHA-256', render: (b) => <span className="font-mono text-xs text-[var(--zf-muted)]" title={b.sha256}>{shortSha(b.sha256)}</span> },
            {
              key: 'gate',
              header: 'Boot gate',
              render: (b) => b.validationScore
                ? <StatusBadge status={b.certified ? 'healthy' : 'warning'} label={`${b.certified ? 'Certified' : 'Not certified'} · ${Math.round(b.validationScore)}`} />
                : <span className="text-xs text-[var(--zf-muted)]">—</span>,
            },
            {
              key: 'datasource',
              header: 'DataSource',
              render: (b) => (
                <div className="text-xs text-[var(--zf-muted)]">
                  <div className="font-mono">{b.dataSource || '—'}</div>
                  {b.bootstrapState && <div>{b.bootstrapState}{b.bootstrapMessage ? ` · ${b.bootstrapMessage}` : ''}</div>}
                </div>
              ),
            },
            {
              key: 'actions',
              header: 'Actions',
              render: (b) => (
                <div className="flex gap-1">
                  <button
                    title="Bootstrap into KubeVirt (CDI DataSource)"
                    className="zf-btn zf-btn-ghost zf-btn-sm !px-2"
                    disabled={!project || !canBootstrap(b) || busy !== null}
                    onClick={() => project && void run(`${b.id}:bootstrap`, () => bootstrapKrytonGolden(b.id, project))}
                  ><Rocket className="w-3.5 h-3.5" /></button>
                  <button
                    title="View guestkit passport"
                    className="zf-btn zf-btn-ghost zf-btn-sm !px-2"
                    disabled={!b.validationScore || busy !== null}
                    onClick={() => void showPassport(b)}
                  ><FileJson className="w-3.5 h-3.5" /></button>
                </div>
              ),
            },
          ]}
          rows={builds}
          getRowKey={(b) => b.id}
          loading={loading}
          bordered={false}
          emptyState={<EmptyState icon={<Hammer className="w-10 h-10" />} title="No Windows golden builds" description="Builds started here or on the Kryton host appear in this list." />}
        />
        {passport && (
          <div className="border-t border-[var(--zf-hairline)] p-4 space-y-2">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold">guestkit passport · <span className="font-mono">{passport.id}</span></h3>
              <button className="zf-btn zf-btn-ghost zf-btn-sm !px-2" title="Close" onClick={() => setPassport(null)}><X className="w-3.5 h-3.5" /></button>
            </div>
            <pre className="text-xs bg-[var(--zf-surface-2,var(--zf-surface))] rounded-lg p-3 max-h-96 overflow-auto">{JSON.stringify(passport.data, null, 2)}</pre>
          </div>
        )}
      </Section>

      <Section title="Linux imports" aside={<Link className="underline" to="/app/create">Download an OS image from Create VM</Link>}>
        <DataTable
          columns={[
            { key: 'name', header: 'Image', render: (d) => <span className="font-medium">{d.name}</span> },
            { key: 'state', header: 'State', render: (d) => <StatusBadge status={d.state} /> },
            { key: 'output', header: 'Disk', render: (d) => <span className="font-mono text-xs text-[var(--zf-muted)]">{d.output_path || d.error || '—'}</span> },
            { key: 'started', header: 'Started', render: (d) => <span className="text-xs text-[var(--zf-muted)]">{new Date(d.started).toLocaleString()}</span> },
          ]}
          rows={downloads}
          getRowKey={(d) => d.id}
          loading={loading}
          bordered={false}
          emptyState={<EmptyState icon={<CloudDownload className="w-10 h-10" />} title="No Linux imports yet" description="Importing a cloud image creates a versioned DataVolume and a stable DataSource." />}
        />
      </Section>

      <Section title="Captures from VMs" aside="Start a capture from a VM's details page">
        {captures === null ? (
          <div className="px-4 py-3 text-sm text-[var(--zf-muted)]">Capture history is visible to cluster admins.</div>
        ) : (
          <DataTable
            columns={[
              { key: 'resource', header: 'Source VM', render: (op) => <span className="font-medium">{op.namespace}/{op.resource}</span> },
              { key: 'state', header: 'State', render: (op) => <StatusBadge status={op.state} /> },
              { key: 'progress', header: 'Progress', render: (op) => <span className="text-xs text-[var(--zf-muted)]">{op.error || `${op.phase || ''} ${op.progress}%`}</span> },
              { key: 'created', header: 'Started', render: (op) => <span className="text-xs text-[var(--zf-muted)]">{new Date(op.created).toLocaleString()}</span> },
            ]}
            rows={captures}
            getRowKey={(op) => op.id}
            loading={loading}
            bordered={false}
            emptyState={<EmptyState icon={<Layers className="w-10 h-10" />} title="No captures yet" description="Capturing a VM clones its disk into a standalone golden DataVolume." />}
          />
        )}
      </Section>
    </div>
  )
}
