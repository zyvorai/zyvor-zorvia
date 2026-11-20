// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useState } from 'react'
import { ShieldCheck, Plus, Trash2 } from 'lucide-react'
import {
  listAtlasTenants,
  putAtlasTenantQuota,
  listAtlasSchedules,
  createAtlasSchedule,
  deleteAtlasSchedule,
  AtlasTenant,
  AtlasSchedule,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { formatBytes } from '../../utils/format'
import { useConfirm } from '../../hooks/useConfirm'
import ConfirmDialog from '../../components/ConfirmDialog'

/** Tenant quotas and protection schedules. Tenant policy overrides and
    volume labels/bindings are proxied on the backend but deliberately have
    no UI here yet -- lower-frequency day-2 operations than quota/schedule
    management, better suited to a future pass if there's real demand. */
export default function AtlasGovernanceSection() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [tenants, setTenants] = useState<AtlasTenant[]>([])
  const [schedules, setSchedules] = useState<AtlasSchedule[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  const load = async () => {
    try {
      const [t, s] = await Promise.all([listAtlasTenants(), listAtlasSchedules()])
      setTenants(t)
      setSchedules(s)
    } catch (e) {
      toastFailure(toast, 'Failed to load governance data', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const saveQuota = (tenantId: string, maxGiB: number, maxVolumes: number) => {
    setBusy(`quota-${tenantId}`)
    putAtlasTenantQuota(tenantId, maxGiB * 1024 * 1024 * 1024, maxVolumes)
      .then(() => {
        toast.success(`Quota updated for '${tenantId}'`)
        return load()
      })
      .catch((e) => toastFailure(toast, 'Failed to update quota', e))
      .finally(() => setBusy(null))
  }

  const removeSchedule = async (schedule: AtlasSchedule) => {
    if (
      !(await confirm(`Delete schedule`, `Delete this ${schedule.kind} schedule for volume '${schedule.volume_id}'?`, {
        variant: 'danger',
        confirmLabel: 'Delete',
      }))
    ) {
      return
    }
    setBusy(`delete-sched-${schedule.id}`)
    try {
      await deleteAtlasSchedule(schedule.id)
      toast.success('Schedule deleted')
      await load()
    } catch (e) {
      toastFailure(toast, 'Failed to delete schedule', e)
    } finally {
      setBusy(null)
    }
  }

  if (loading) return null

  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
        <ShieldCheck className="w-4 h-4 text-[var(--zf-muted)]" />
        Governance
      </h2>

      {tenants.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Tenant quotas</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {tenants.map((t) => (
              <TenantRow key={t.tenant_id} tenant={t} busy={busy} onSave={saveQuota} />
            ))}
          </div>
        </div>
      )}

      {schedules.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Protection schedules</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {schedules.map((s) => (
              <div key={s.id} className="flex items-center justify-between py-2 gap-3">
                <div>
                  <div className="font-medium text-sm text-[var(--zf-ink)]">{s.volume_id}</div>
                  <div className="text-xs text-[var(--zf-muted)]">
                    {s.kind} · every {Math.round(s.interval_secs / 60)}min · keep {s.keep || 'all'}
                    {s.next_run_at ? ` · next ${new Date(s.next_run_at).toLocaleString()}` : ''}
                  </div>
                </div>
                <button
                  type="button"
                  disabled={busy !== null}
                  onClick={() => void removeSchedule(s)}
                  className="zf-btn zf-btn-danger zf-btn-sm"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <CreateScheduleForm
        disabled={busy !== null}
        onCreate={(volumeId, intervalSecs, keep) => {
          setBusy('create-sched')
          createAtlasSchedule(volumeId, { interval_secs: intervalSecs, keep, kind: 'snapshot' })
            .then(() => {
              toast.success(`Schedule created for '${volumeId}'`)
              return load()
            })
            .catch((e) => toastFailure(toast, 'Failed to create schedule', e))
            .finally(() => setBusy(null))
        }}
      />

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

function TenantRow({
  tenant,
  busy,
  onSave,
}: {
  tenant: AtlasTenant
  busy: string | null
  onSave: (tenantId: string, maxGiB: number, maxVolumes: number) => void
}) {
  const [maxGiB, setMaxGiB] = useState(Math.round(tenant.max_bytes / (1024 * 1024 * 1024)))
  const [maxVolumes, setMaxVolumes] = useState(tenant.max_volumes)

  return (
    <div className="flex items-center justify-between py-2 gap-3 flex-wrap">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{tenant.tenant_id}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          {formatBytes(tenant.used_bytes)} used · {tenant.volume_count} volumes
        </div>
      </div>
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={0}
          value={maxGiB}
          onChange={(e) => setMaxGiB(parseInt(e.target.value) || 0)}
          className="w-24 px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-xs"
          title="Max GiB (0 = unlimited)"
        />
        <input
          type="number"
          min={0}
          value={maxVolumes}
          onChange={(e) => setMaxVolumes(parseInt(e.target.value) || 0)}
          className="w-20 px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-xs"
          title="Max volumes (0 = unlimited)"
        />
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => onSave(tenant.tenant_id, maxGiB, maxVolumes)}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Save
        </button>
      </div>
    </div>
  )
}

function CreateScheduleForm({
  disabled,
  onCreate,
}: {
  disabled: boolean
  onCreate: (volumeId: string, intervalSecs: number, keep: number) => void
}) {
  const [volumeId, setVolumeId] = useState('')
  const [intervalHours, setIntervalHours] = useState(24)
  const [keep, setKeep] = useState(7)

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Volume id</label>
        <input
          value={volumeId}
          onChange={(e) => setVolumeId(e.target.value)}
          placeholder="vol_..."
          className="w-56 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Every (hours)</label>
        <input
          type="number"
          min={1}
          value={intervalHours}
          onChange={(e) => setIntervalHours(parseInt(e.target.value) || 1)}
          className="w-24 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Keep (0 = all)</label>
        <input
          type="number"
          min={0}
          value={keep}
          onChange={(e) => setKeep(parseInt(e.target.value) || 0)}
          className="w-20 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !volumeId.trim()}
        onClick={() => {
          onCreate(volumeId.trim(), intervalHours * 3600, keep)
          setVolumeId('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Add snapshot schedule
      </button>
    </div>
  )
}
