// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useCallback, useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { PageHeader, Card, CardBody, StatusBadge } from '../components/ui'
import ConfirmDialog from '../components/ConfirmDialog'
import ExperimentalDisabledNotice, { ExperimentalBadge, INPUT_CLASS } from '../components/ExperimentalDisabledNotice'
import {
  applyEncryption,
  getVolumeEncryptionStatus,
  planEncryption,
  rotateKey,
  type EncryptPlan,
  type VolumeEncryptionStatus,
} from '../api/volumeEncryption'
import { isExperimentalDisabled } from '../utils/experimental'
import {
  dataVolumeName,
  encryptFormProblem,
  rotateFormProblem,
  rotateSummary,
  toEncryptRequest,
  toRotateRequest,
  type EncryptForm,
} from '../utils/volumeEncryption'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { useToastContext } from '../contexts/ToastContext'
import { usePermissions } from '../hooks/usePermissions'

const EMPTY: EncryptForm = { namespace: '', vm: '', volume: '', size: '20Gi', storageClass: '', acknowledge: false }

export default function VolumeEncryption() {
  const toast = useToastContext()
  const { canAdmin } = usePermissions()
  const [status, setStatus] = useState<VolumeEncryptionStatus | null>(null)
  const [disabled, setDisabled] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [form, setForm] = useState<EncryptForm>(EMPTY)
  const [plan, setPlan] = useState<EncryptPlan | null>(null)
  const [busy, setBusy] = useState<'plan' | 'apply' | 'rotate' | null>(null)
  const [confirmApply, setConfirmApply] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setStatus(await getVolumeEncryptionStatus())
      setDisabled(null)
    } catch (e) {
      if (isExperimentalDisabled(e)) setDisabled(e.message)
      else setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  const edit = (patch: Partial<EncryptForm>) => {
    setForm({ ...form, ...patch })
    setPlan(null) // a plan is only valid for the exact request it was made for
    setMessage(null)
  }
  const text = (k: 'namespace' | 'vm' | 'volume' | 'size' | 'storageClass') => (e: React.ChangeEvent<HTMLInputElement>) =>
    edit({ [k]: e.target.value })

  const encryptProblem = encryptFormProblem(form)
  const rotateProblem = rotateFormProblem(form)

  const guard = async (kind: 'plan' | 'apply' | 'rotate', label: string, fn: () => Promise<void>) => {
    setBusy(kind)
    try {
      await fn()
    } catch (e) {
      if (isExperimentalDisabled(e)) setDisabled(e.message)
      else toastFailure(toast, label, e)
    } finally {
      setBusy(null)
    }
  }

  const onPlan = () =>
    guard('plan', 'Plan failed', async () => {
      setPlan(await planEncryption(toEncryptRequest(form)))
    })

  const onApply = () => {
    setConfirmApply(false)
    return guard('apply', 'Apply failed', async () => {
      const r = await applyEncryption(toEncryptRequest(form))
      setMessage(`Creating encrypted DataVolume ${r.data_volume}; wrapped key stored in Secret ${r.key_secret}.`)
      toast.success(`Encrypted volume ${r.data_volume} requested`)
      setPlan(null)
    })
  }

  const onRotate = () =>
    guard('rotate', 'Rotate failed', async () => {
      const r = await rotateKey(toRotateRequest(form))
      setMessage(rotateSummary(r))
      toast.success(r.rewrapped ? 'Data key re-wrapped' : 'Key already current')
    })

  const header = (
    <PageHeader
      title="Volume Encryption"
      description="Envelope-encrypted data keys for VM volumes: status, plan, apply and rotate."
      eyebrow="Experimental"
      onRefresh={load}
      refreshing={loading}
    />
  )

  if (disabled !== null) {
    return (
      <div className="space-y-6">
        {header}
        <ExperimentalDisabledNotice feature="Volume encryption" message={disabled} />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {header}

      {!canAdmin && (
        <p className="text-sm text-[var(--zf-warning)]" role="status">
          Volume encryption needs the admin role (cluster.admin). Controls are hidden.
        </p>
      )}
      {error && <p className="text-sm text-[var(--zf-danger)]" role="alert">{error}</p>}

      <Card>
        <CardBody className="space-y-2">
          <div className="text-sm font-medium text-[var(--zf-ink)] flex items-center gap-2">
            Key provider <ExperimentalBadge />
          </div>
          {loading ? (
            <p className="text-sm text-[var(--zf-muted)]">Loading…</p>
          ) : status ? (
            <div className="flex items-center gap-3 text-sm">
              <StatusBadge status={status.ready ? 'healthy' : 'failed'} label={status.ready ? 'Ready' : 'No key provider'} />
              <span className="text-[var(--zf-muted)]">{status.provider ?? 'none configured (ZORVIA_KEY_FILE or ZORVIA_VAULT_*)'}</span>
            </div>
          ) : null}
          <p className="text-xs text-[var(--zf-muted)]">
            With the storage_class mode the CSI driver encrypts with its own KMS configuration. The Zorvia-wrapped data key is
            recorded for key custody, audit and rotation, but the driver does not use it. Treat this as key custody plus an
            encrypted-volume provisioning flow, not end-to-end KMS-backed encryption. Unit-tested only; not run against a live cluster.
          </p>
        </CardBody>
      </Card>

      {canAdmin && (
        <Card>
          <CardBody className="space-y-4">
            <div className="text-sm font-medium text-[var(--zf-ink)]">Volume</div>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              <input aria-label="Namespace" placeholder="Namespace" value={form.namespace} onChange={text('namespace')} className={INPUT_CLASS} />
              <input aria-label="VM" placeholder="VM" value={form.vm} onChange={text('vm')} className={INPUT_CLASS} />
              <input aria-label="Volume" placeholder="Volume (disk name)" value={form.volume} onChange={text('volume')} className={INPUT_CLASS} />
              <input aria-label="Size" placeholder="Size (20Gi)" value={form.size} onChange={text('size')} className={INPUT_CLASS} />
              <input aria-label="StorageClass" placeholder="Encrypting StorageClass" value={form.storageClass} onChange={text('storageClass')} className={`${INPUT_CLASS} md:col-span-2`} />
            </div>
            <label className="flex items-center gap-2 text-xs text-[var(--zf-muted)]">
              <input type="checkbox" checked={form.acknowledge} onChange={(e) => edit({ acknowledge: e.target.checked })} />
              Acknowledge unverified encryption (use only if you know this StorageClass encrypts; Zorvia recognises Ceph-CSI RBD only)
            </label>
            {form.vm && encryptProblem && <p className="text-xs text-[var(--zf-warning)]" role="alert">{encryptProblem}</p>}

            <div className="flex flex-wrap gap-2">
              <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" disabled={!!encryptProblem || busy !== null} onClick={onPlan}>
                {busy === 'plan' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                Plan
              </button>
              <button
                type="button"
                className="zf-btn zf-btn-primary zf-btn-sm"
                disabled={!plan || !plan.ready || busy !== null}
                title={plan ? undefined : 'Run a plan first'}
                onClick={() => setConfirmApply(true)}
              >
                {busy === 'apply' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                Apply
              </button>
              <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" disabled={!!rotateProblem || busy !== null} onClick={onRotate}>
                {busy === 'rotate' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                Rotate key
              </button>
            </div>

            {plan && (
              <div className="space-y-2 text-sm" data-testid="encrypt-plan">
                <StatusBadge status={plan.ready ? 'healthy' : 'failed'} label={plan.ready ? 'Ready to apply' : 'Blocked'} />
                {plan.blockers.length > 0 && (
                  <ul className="text-[var(--zf-danger)] list-disc pl-5" aria-label="Blockers">
                    {plan.blockers.map((b) => <li key={b}>{b}</li>)}
                  </ul>
                )}
                {plan.warnings.length > 0 && (
                  <ul className="text-[var(--zf-warning)] list-disc pl-5" aria-label="Warnings">
                    {plan.warnings.map((w) => <li key={w}>{w}</li>)}
                  </ul>
                )}
                <ol className="text-[var(--zf-muted)] list-decimal pl-5">
                  {plan.steps.map((s) => <li key={s}>{s}</li>)}
                </ol>
              </div>
            )}
            {message && <p className="text-sm text-[var(--zf-ink)]" role="status">{message}</p>}
          </CardBody>
        </Card>
      )}

      {confirmApply && (
        <ConfirmDialog
          title="Create encrypted volume?"
          message={`This stores a wrapped data key in Secret zorvia-dek-${form.vm}-${form.volume} and creates DataVolume ${dataVolumeName(form.vm, form.volume)} on StorageClass ${form.storageClass}. The action is audited.`}
          confirmLabel="Apply"
          variant="warning"
          onConfirm={onApply}
          onCancel={() => setConfirmApply(false)}
        />
      )}
    </div>
  )
}
