// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useState } from 'react'
import { ShieldAlert, Plus, Trash2, ArrowUpCircle, ArrowDownCircle } from 'lucide-react'
import {
  listAtlasDrPeers,
  registerAtlasDrPeer,
  deleteAtlasDrPeer,
  listAtlasDrMirrors,
  getAtlasDrStatus,
  getAtlasDrPreflight,
  promoteAtlasMirror,
  demoteAtlasMirror,
  atlasDrFailover,
  pollAtlasJob,
  isAtlasJobTerminal,
  AtlasDrPeer,
  AtlasDrMirror,
  AtlasDrStatus,
  AtlasDrPreflight,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { useConfirm } from '../../hooks/useConfirm'
import ConfirmDialog from '../../components/ConfirmDialog'

/** Disaster recovery (RBD mirroring). Atlas's own source labels this
    "scaffolding -- real ops UNVERIFIED without a 2nd cluster": the control-
    plane catalog (peers, mirror role/state bookkeeping, preflight checks)
    is real and tested, but promote/demote/failover only exercise the real
    `rbd mirror` CLI when Atlas's Ceph driver is in `real` mode against an
    actual second cluster -- this deployment's `dataplane_verified` flag
    (surfaced below) says whether that live drill has ever been run. Treat
    every failover here as a control-plane-only drill until it's true. */
export default function AtlasDrSection() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [status, setStatus] = useState<AtlasDrStatus | null>(null)
  const [preflight, setPreflight] = useState<AtlasDrPreflight | null>(null)
  const [peers, setPeers] = useState<AtlasDrPeer[]>([])
  const [mirrors, setMirrors] = useState<AtlasDrMirror[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  const load = async () => {
    try {
      const [s, pf, p, m] = await Promise.all([
        getAtlasDrStatus(),
        getAtlasDrPreflight(),
        listAtlasDrPeers(),
        listAtlasDrMirrors(),
      ])
      setStatus(s)
      setPreflight(pf)
      setPeers(p)
      setMirrors(m)
    } catch (e) {
      toastFailure(toast, 'Failed to load DR status', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    const interval = setInterval(() => void load(), 15000)
    return () => clearInterval(interval)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const runWrite = async (busyKey: string, actionLabel: string, name: string, op: () => Promise<{ job_id: string }>) => {
    setBusy(busyKey)
    try {
      const envelope = await op()
      toast.success(`${actionLabel} requested for '${name}'`)
      const job = await pollAtlasJob(envelope.job_id)
      if (isAtlasJobTerminal(job)) {
        if (job.state === 'succeeded') {
          toast.success(`${actionLabel} succeeded for '${name}'`)
        } else {
          toast.error(`${actionLabel} failed for '${name}': ${job.error ?? 'unknown error'}`)
        }
      }
      await load()
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  if (loading) return null

  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
        <ShieldAlert className="w-4 h-4 text-[var(--zf-warning)]" />
        Disaster recovery
      </h2>

      {/* Persistent, non-dismissable -- this is not a one-time notice. */}
      <p className="text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/25 rounded px-3 py-2">
        Cross-cluster RBD mirroring is scaffolding on Atlas's own side: real promote/demote/
        failover are <strong>unverified without a second real Ceph cluster</strong>.
        {status && !status.dataplane_verified && (
          <> This deployment has not completed a live two-site mirror drill — treat every failover here as a control-plane-only drill.</>
        )}
        {status?.dataplane_verified && <> This deployment has completed a live two-site mirror drill.</>}
      </p>

      {status && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <StatTile label="Peers" value={status.peers} />
          <StatTile label="Mirrors" value={status.mirrors} />
          <StatTile label="Primary" value={status.primary} />
          <StatTile label="Secondary" value={status.secondary} />
        </div>
      )}

      {preflight && !preflight.ready && (
        <div className="text-xs text-[var(--zf-muted)] space-y-1">
          <div className="font-medium text-[var(--zf-warning)]">Failover preflight not ready:</div>
          {preflight.blockers.map((b) => (
            <div key={b}>• {b}</div>
          ))}
        </div>
      )}

      {peers.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Peers</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {peers.map((p) => (
              <div key={p.id} className="flex items-center justify-between py-2 gap-3">
                <div>
                  <div className="font-medium text-sm text-[var(--zf-ink)]">{p.name}</div>
                  <div className="text-xs text-[var(--zf-muted)]">
                    {p.direction} · {p.state}
                  </div>
                </div>
                <button
                  type="button"
                  disabled={busy !== null}
                  onClick={async () => {
                    if (
                      !(await confirm(`Remove peer '${p.name}'`, 'Removes this peer and any mirrors that reference it.', {
                        variant: 'danger',
                        confirmLabel: 'Remove',
                      }))
                    ) {
                      return
                    }
                    setBusy(`delete-peer-${p.id}`)
                    try {
                      await deleteAtlasDrPeer(p.id)
                      toast.success(`Peer '${p.name}' removed`)
                      await load()
                    } catch (e) {
                      toastFailure(toast, 'Failed to remove peer', e)
                    } finally {
                      setBusy(null)
                    }
                  }}
                  className="zf-btn zf-btn-danger zf-btn-sm"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <RegisterPeerForm
        disabled={busy !== null}
        onRegister={(name) => {
          setBusy('register-peer')
          registerAtlasDrPeer({ name })
            .then(() => {
              toast.success(`Peer '${name}' registered`)
              return load()
            })
            .catch((e) => toastFailure(toast, 'Failed to register peer', e))
            .finally(() => setBusy(null))
        }}
      />

      {mirrors.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Mirrors</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {mirrors.map((m) => (
              <MirrorRow
                key={m.id}
                mirror={m}
                busy={busy}
                onPromote={async (force) => {
                  if (
                    !(await confirm(
                      `Promote mirror to primary`,
                      `Promote rbd:${m.pool}/${m.image} to primary${force ? ' (force -- split-brain recovery)' : ''}? This is a failover action and only verified as a control-plane operation without a second real cluster.`,
                      { variant: 'danger', confirmLabel: force ? 'Force promote' : 'Promote' },
                    ))
                  ) {
                    return
                  }
                  void runWrite(`promote-${m.id}`, 'Promote', m.id, () => promoteAtlasMirror(m.id, force))
                }}
                onDemote={async () => {
                  if (
                    !(await confirm(`Demote mirror to secondary`, `Demote rbd:${m.pool}/${m.image} to secondary?`, {
                      variant: 'warning',
                      confirmLabel: 'Demote',
                    }))
                  ) {
                    return
                  }
                  void runWrite(`demote-${m.id}`, 'Demote', m.id, () => demoteAtlasMirror(m.id))
                }}
                onFailover={async () => {
                  if (
                    !(await confirm(
                      'Run failover runbook',
                      `Run the one-click failover runbook for rbd:${m.pool}/${m.image}? This promotes it to primary and is only verified as a control-plane operation without a second real cluster.${preflight && !preflight.ready ? ' Preflight is NOT ready -- this will force past the blockers listed above.' : ''}`,
                      { variant: 'danger', confirmLabel: 'Run failover' },
                    ))
                  ) {
                    return
                  }
                  void runWrite(`failover-${m.id}`, 'Failover', m.id, () =>
                    atlasDrFailover(m.id, !!(preflight && !preflight.ready)),
                  )
                }}
              />
            ))}
          </div>
        </div>
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
  )
}

function MirrorRow({
  mirror,
  busy,
  onPromote,
  onDemote,
  onFailover,
}: {
  mirror: AtlasDrMirror
  busy: string | null
  onPromote: (force: boolean) => void
  onDemote: () => void
  onFailover: () => void
}) {
  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">
          rbd:{mirror.pool}/{mirror.image}
        </div>
        <div className="text-xs text-[var(--zf-muted)]">
          {mirror.role} · {mirror.state}
          {mirror.rpo_seconds != null ? ` · RPO ${mirror.rpo_seconds}s` : ''}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        {mirror.role === 'secondary' ? (
          <button type="button" disabled={busy !== null} onClick={() => onPromote(false)} className="zf-btn zf-btn-ghost zf-btn-sm">
            <ArrowUpCircle className="w-3.5 h-3.5" />
            Promote
          </button>
        ) : (
          <button type="button" disabled={busy !== null} onClick={() => onPromote(true)} className="zf-btn zf-btn-ghost zf-btn-sm">
            <ArrowUpCircle className="w-3.5 h-3.5" />
            Force promote
          </button>
        )}
        {mirror.role === 'primary' && (
          <button type="button" disabled={busy !== null} onClick={onDemote} className="zf-btn zf-btn-ghost zf-btn-sm">
            <ArrowDownCircle className="w-3.5 h-3.5" />
            Demote
          </button>
        )}
        <button type="button" disabled={busy !== null} onClick={onFailover} className="zf-btn zf-btn-danger zf-btn-sm">
          Failover
        </button>
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

function RegisterPeerForm({ disabled, onRegister }: { disabled: boolean; onRegister: (name: string) => void }) {
  const [name, setName] = useState('')
  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Peer cluster name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="dr-site-2"
          className="w-48 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim()}
        onClick={() => {
          onRegister(name.trim())
          setName('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Register peer
      </button>
    </div>
  )
}
