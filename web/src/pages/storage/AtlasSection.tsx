// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useState } from 'react'
import { Layers, Plus, Trash2 } from 'lucide-react'
import {
  getAtlasStatus,
  listAtlasBackends,
  listAtlasClusters,
  listAtlasPools,
  listAtlasVolumes,
  listAtlasJobs,
  createAtlasVolume,
  expandAtlasVolume,
  deleteAtlasVolume,
  cancelAtlasJob,
  pollAtlasJob,
  isAtlasJobTerminal,
  atlasVolumeOwnerForVm,
  createAtlasBackend,
  deleteAtlasBackend,
  discoverAtlasBackend,
  cordonAtlasBackend,
  uncordonAtlasBackend,
  AtlasStatus,
  AtlasBackend,
  AtlasCluster,
  AtlasPool,
  AtlasVolume,
  AtlasJob,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { formatBytes } from '../../utils/format'
import { useConfirm } from '../../hooks/useConfirm'
import ConfirmDialog from '../../components/ConfirmDialog'
import AtlasRbdSection from './AtlasRbdSection'
import AtlasBucketsSection from './AtlasBucketsSection'
import AtlasDrSection from './AtlasDrSection'
import AtlasAiSection from './AtlasAiSection'
import AtlasObservabilitySection from './AtlasObservabilitySection'
import AtlasGovernanceSection from './AtlasGovernanceSection'

export const ATLAS_HEALTH_STYLES: Record<string, string> = {
  ok: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  warn: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
  critical: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
  unknown: 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
}

export default function AtlasSection() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [status, setStatus] = useState<AtlasStatus | null>(null)
  const [backends, setBackends] = useState<AtlasBackend[]>([])
  const [clusters, setClusters] = useState<AtlasCluster[]>([])
  const [pools, setPools] = useState<AtlasPool[]>([])
  const [volumes, setVolumes] = useState<AtlasVolume[]>([])
  const [jobs, setJobs] = useState<AtlasJob[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  const load = async () => {
    try {
      const s = await getAtlasStatus()
      setStatus(s)
      if (!s.enabled || !s.connected) {
        setBackends([])
        setClusters([])
        setPools([])
        setVolumes([])
        setJobs([])
        return
      }
      const [b, c, p, v, j] = await Promise.all([
        listAtlasBackends().catch(() => []),
        listAtlasClusters().catch(() => []),
        listAtlasPools().catch(() => []),
        listAtlasVolumes().catch(() => []),
        listAtlasJobs().catch(() => []),
      ])
      setBackends(b)
      setClusters(c)
      setPools(p)
      setVolumes(v)
      setJobs(j)
    } catch {
      // Status fetch itself failing (network/auth) leaves status null; the
      // section below just shows "Not configured" rather than an alarming
      // banner — Atlas is an optional secondary integration on this page.
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    const interval = setInterval(() => void load(), 15000)
    return () => clearInterval(interval)
  }, [])

  /** Every Atlas write only ever returns "requested" synchronously -- this
      polls the real job to a terminal state in the background and toasts
      the actual outcome, instead of leaving the user to guess from the next
      15s refresh whether it actually succeeded. */
  const runAtlasWrite = async (busyKey: string, actionLabel: string, resourceName: string, op: () => Promise<{ job_id: string }>) => {
    setBusy(busyKey)
    try {
      const envelope = await op()
      toast.success(`${actionLabel} requested for '${resourceName}'`)
      await load()
      const job = await pollAtlasJob(envelope.job_id)
      if (isAtlasJobTerminal(job)) {
        if (job.state === 'succeeded') {
          toast.success(`${actionLabel} succeeded for '${resourceName}'`)
        } else {
          toast.error(`${actionLabel} failed for '${resourceName}': ${job.error ?? 'unknown error'}`)
        }
      }
      await load()
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  /** Backend lifecycle (create/delete/discover/cordon/uncordon) is
      synchronous on Atlas's side -- no job to poll, just refresh. */
  const runAtlasSync = async (busyKey: string, actionLabel: string, resourceName: string, op: () => Promise<unknown>) => {
    setBusy(busyKey)
    try {
      await op()
      toast.success(`${actionLabel} '${resourceName}'`)
      await load()
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  if (loading) return null

  const enabled = status?.enabled ?? false
  const connected = status?.connected ?? false

  return (
    <>
      <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
        <div className="flex items-center justify-between flex-wrap gap-3">
          <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
            <Layers className="w-4 h-4 text-[var(--zf-muted)]" />
            Atlas
          </h2>
          <span
            className={`px-3 py-1 rounded-full text-xs font-medium border ${
              !enabled
                ? ATLAS_HEALTH_STYLES.unknown
                : connected
                  ? ATLAS_HEALTH_STYLES.ok
                  : ATLAS_HEALTH_STYLES.critical
            }`}
          >
            {!enabled ? 'Not configured' : connected ? 'Connected' : 'Unreachable'}
          </span>
        </div>

        {!enabled && (
          <p className="text-sm text-[var(--zf-muted)]">
            Optional pluggable storage control plane for Ceph/NFS/ZFS with multi-backend governance
            and capacity forecasting. Set <code>ATLAS_URL</code> (and <code>ATLAS_TOKEN</code>) on
            the Zorvia server to enable — see{' '}
            <a href="/docs/ATLAS_INTEGRATION.md" className="underline">
              docs/ATLAS_INTEGRATION.md
            </a>
            .
          </p>
        )}

        {enabled && !connected && (
          <p className="text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/25 rounded px-3 py-2">
            {status?.error || 'Could not reach the configured Atlas control plane.'}
          </p>
        )}

        {enabled && connected && (
          <>
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
              <StatTile label="Backends" value={backends.length} />
              <StatTile label="Clusters" value={clusters.length} />
              <StatTile label="Pools" value={pools.length} />
              <StatTile label="Volumes" value={volumes.length} />
            </div>

            {backends.length > 0 && (
              <div>
                <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Backends</h3>
                <div className="divide-y divide-[var(--zf-hairline)]">
                  {backends.map((b) => (
                    <AtlasBackendRow
                      key={b.id}
                      backend={b}
                      busy={busy}
                      onDiscover={() => void runAtlasSync(`discover-${b.id}`, 'Discovery started for', b.name, () => discoverAtlasBackend(b.id))}
                      onToggleCordon={() =>
                        void runAtlasSync(
                          `cordon-${b.id}`,
                          b.cordoned ? 'Uncordoned' : 'Cordoned',
                          b.name,
                          () => (b.cordoned ? uncordonAtlasBackend(b.id) : cordonAtlasBackend(b.id)),
                        )
                      }
                      onDelete={async () => {
                        if (
                          !(await confirm(`Delete backend '${b.name}'`, 'Delete this backend? Refused if volumes still reference it unless purged.', {
                            variant: 'danger',
                            confirmLabel: 'Delete',
                          }))
                        ) {
                          return
                        }
                        void runAtlasSync(`delete-backend-${b.id}`, 'Deleted', b.name, () => deleteAtlasBackend(b.id, false))
                      }}
                    />
                  ))}
                </div>
              </div>
            )}

            <CreateAtlasBackendForm
              disabled={busy !== null}
              onCreate={(name) => void runAtlasSync('create-backend', 'Created', name, () => createAtlasBackend({ name }))}
            />

            {volumes.length > 0 && (
              <div>
                <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Volumes</h3>
                <div className="divide-y divide-[var(--zf-hairline)]">
                  {volumes.map((v) => (
                    <AtlasVolumeRow
                      key={v.id}
                      volume={v}
                      busy={busy}
                      onExpand={(newSizeBytes) =>
                        void runAtlasWrite(`expand-${v.id}`, 'Expand', v.name, () => expandAtlasVolume(v.id, newSizeBytes))
                      }
                      onDelete={async () => {
                        if (
                          !(await confirm(`Delete volume '${v.name}'`, 'Delete this volume? Any data on it is destroyed.', {
                            variant: 'danger',
                            confirmLabel: 'Delete',
                          }))
                        ) {
                          return
                        }
                        // Always pass confirm=true -- Atlas itself decides (based on
                        // storage class / protection tier) whether that was required;
                        // Zorvia doesn't try to duplicate that judgment.
                        void runAtlasWrite(`delete-${v.id}`, 'Delete', v.name, () => deleteAtlasVolume(v.id, true))
                      }}
                    />
                  ))}
                </div>
              </div>
            )}

            <CreateAtlasVolumeForm
              disabled={busy !== null}
              onCreate={(req) => void runAtlasWrite('create-volume', 'Create', req.name, () => createAtlasVolume(req))}
            />

            {jobs.length > 0 && (
              <div>
                <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Recent jobs</h3>
                <div className="divide-y divide-[var(--zf-hairline)]">
                  {jobs.slice(0, 10).map((j) => (
                    <AtlasJobRow
                      key={j.id}
                      job={j}
                      busy={busy === `cancel-job-${j.id}`}
                      onCancel={() => {
                        setBusy(`cancel-job-${j.id}`)
                        cancelAtlasJob(j.id)
                          .then(() => {
                            toast.success(`Job '${j.id}' cancelled`)
                            return load()
                          })
                          .catch((e) => toastFailure(toast, 'Failed to cancel job', e))
                          .finally(() => setBusy(null))
                      }}
                    />
                  ))}
                </div>
              </div>
            )}
          </>
        )}

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

      {enabled && connected && <AtlasRbdSection />}
      {enabled && connected && <AtlasBucketsSection />}
      {enabled && connected && <AtlasDrSection />}
      {enabled && connected && <AtlasAiSection />}
      {enabled && connected && <AtlasObservabilitySection />}
      {enabled && connected && <AtlasGovernanceSection />}
    </>
  )
}

function AtlasBackendRow({
  backend,
  busy,
  onDiscover,
  onToggleCordon,
  onDelete,
}: {
  backend: AtlasBackend
  busy: string | null
  onDiscover: () => void
  onToggleCordon: () => void
  onDelete: () => void
}) {
  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{backend.name}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          {backend.backend_type} · {backend.mode}
          {backend.cordoned ? ' · cordoned' : ''}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <span
          className={`px-2 py-0.5 rounded-full text-xs font-medium border ${
            backend.cordoned ? ATLAS_HEALTH_STYLES.warn : ATLAS_HEALTH_STYLES.ok
          }`}
        >
          {backend.status}
        </span>
        <button type="button" disabled={busy !== null} onClick={onDiscover} className="zf-btn zf-btn-ghost zf-btn-sm">
          Discover
        </button>
        <button type="button" disabled={busy !== null} onClick={onToggleCordon} className="zf-btn zf-btn-ghost zf-btn-sm">
          {backend.cordoned ? 'Uncordon' : 'Cordon'}
        </button>
        <button type="button" disabled={busy !== null} onClick={onDelete} className="zf-btn zf-btn-danger zf-btn-sm">
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  )
}

function CreateAtlasBackendForm({ disabled, onCreate }: { disabled: boolean; onCreate: (name: string) => void }) {
  const [name, setName] = useState('')
  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Backend name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="secondary-ceph"
          className="w-48 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim()}
        onClick={() => {
          onCreate(name.trim())
          setName('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Add backend
      </button>
    </div>
  )
}

function AtlasVolumeRow({
  volume,
  busy,
  onExpand,
  onDelete,
}: {
  volume: AtlasVolume
  busy: string | null
  onExpand: (newSizeBytes: number) => void
  onDelete: () => void
}) {
  const currentGiB = Math.ceil(volume.size_bytes / (1024 * 1024 * 1024))
  const [newSizeGiB, setNewSizeGiB] = useState(currentGiB + 10)
  const rowBusy = busy === `expand-${volume.id}` || busy === `delete-${volume.id}`

  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{volume.name}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          {volume.kind} · {formatBytes(volume.size_bytes)}
          {volume.storage_class_name ? ` · ${volume.storage_class_name}` : ''}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <span
          className={`px-2 py-0.5 rounded-full text-xs font-medium border ${ATLAS_HEALTH_STYLES[volume.health] ?? ATLAS_HEALTH_STYLES.unknown}`}
        >
          {volume.state}
        </span>
        <input
          type="number"
          min={currentGiB + 1}
          value={newSizeGiB}
          onChange={(e) => setNewSizeGiB(parseInt(e.target.value) || currentGiB + 1)}
          className="w-20 px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-xs"
          title="New size (GiB)"
        />
        <button
          type="button"
          disabled={busy !== null || newSizeGiB <= currentGiB}
          onClick={() => onExpand(newSizeGiB * 1024 * 1024 * 1024)}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          {rowBusy && busy === `expand-${volume.id}` ? 'Expanding…' : 'Expand'}
        </button>
        <button
          type="button"
          disabled={busy !== null}
          onClick={onDelete}
          className="zf-btn zf-btn-danger zf-btn-sm"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  )
}

const ATLAS_JOB_STATE_STYLES: Record<string, string> = {
  succeeded: ATLAS_HEALTH_STYLES.ok,
  failed: ATLAS_HEALTH_STYLES.critical,
  running: ATLAS_HEALTH_STYLES.warn,
  verifying: ATLAS_HEALTH_STYLES.warn,
  queued: ATLAS_HEALTH_STYLES.unknown,
  pending: ATLAS_HEALTH_STYLES.unknown,
}

function AtlasJobRow({ job, busy, onCancel }: { job: AtlasJob; busy: boolean; onCancel: () => void }) {
  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{job.job_type}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          {job.id}
          {job.error ? ` · ${job.error}` : ''}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        {!isAtlasJobTerminal(job) && (
          <span className="text-xs text-[var(--zf-muted)]">{job.progress_percent}%</span>
        )}
        <span
          className={`px-2 py-0.5 rounded-full text-xs font-medium border ${ATLAS_JOB_STATE_STYLES[job.state] ?? ATLAS_HEALTH_STYLES.unknown}`}
        >
          {job.state}
        </span>
        {!isAtlasJobTerminal(job) && (
          <button type="button" disabled={busy} onClick={onCancel} className="zf-btn zf-btn-ghost zf-btn-sm">
            {busy ? 'Cancelling…' : 'Cancel'}
          </button>
        )}
      </div>
    </div>
  )
}

function StatTile({ label, value }: { label: string; value: number }) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-md border border-[var(--zf-hairline)] px-3 py-2">
      <div className="text-lg font-semibold text-[var(--zf-ink)]">{value}</div>
      <div className="text-xs text-[var(--zf-muted)]">{label}</div>
    </div>
  )
}

function CreateAtlasVolumeForm({
  disabled,
  onCreate,
}: {
  disabled: boolean
  onCreate: (req: ReturnType<typeof buildCreateAtlasVolumeRequest>) => void
}) {
  const [name, setName] = useState('')
  const [sizeGiB, setSizeGiB] = useState(20)
  const [vmName, setVmName] = useState('')

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="app-data"
          className="w-36 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Size (GiB)</label>
        <input
          type="number"
          min={1}
          value={sizeGiB}
          onChange={(e) => setSizeGiB(parseInt(e.target.value) || 1)}
          className="w-24 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Attribute to VM (optional)</label>
        <input
          value={vmName}
          onChange={(e) => setVmName(e.target.value)}
          placeholder="prod-db"
          className="w-40 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim() || sizeGiB < 1}
        onClick={() => onCreate(buildCreateAtlasVolumeRequest(name.trim(), sizeGiB, vmName.trim()))}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Create volume
      </button>
    </div>
  )
}

function buildCreateAtlasVolumeRequest(name: string, sizeGiB: number, vmName: string) {
  return {
    name,
    size_bytes: sizeGiB * 1024 * 1024 * 1024,
    ...(vmName ? { owner: atlasVolumeOwnerForVm(vmName) } : {}),
  }
}
