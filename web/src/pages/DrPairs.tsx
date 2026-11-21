// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useCallback, useEffect, useState } from 'react'
import { PageHeader, Card, CardBody } from '../components/ui'
import ErrorBanner from '../components/ErrorBanner'
import ConfirmDialog from '../components/ConfirmDialog'
import DrPairsTable from '../components/DrPairsTable'
import FailoverPanel from '../components/FailoverPanel'
import ExperimentalDisabledNotice, { ExperimentalBadge, INPUT_CLASS } from '../components/ExperimentalDisabledNotice'
import {
  createDrPair,
  deleteDrPair,
  failoverDrPair,
  getFailoverPlan,
  listDrPairs,
  type DrPairStatus,
  type FailoverPlan,
} from '../api/drPairs'
import { isExperimentalDisabled } from '../utils/experimental'
import { pairFormProblem, RPO_MAX_SECONDS, RPO_MIN_SECONDS, type PairForm } from '../utils/drPairs'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { useToastContext } from '../contexts/ToastContext'
import { usePermissions } from '../hooks/usePermissions'

const EMPTY: PairForm = { name: '', vm: '', namespace: '', drNamespace: '', rpoSeconds: '3600', rtoSeconds: '' }

export default function DrPairs() {
  const toast = useToastContext()
  const { canAdmin } = usePermissions()
  const [pairs, setPairs] = useState<DrPairStatus[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [disabled, setDisabled] = useState<string | null>(null)
  const [form, setForm] = useState<PairForm>(EMPTY)
  const [creating, setCreating] = useState(false)
  const [plan, setPlan] = useState<FailoverPlan | null>(null)
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const [removing, setRemoving] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setPairs(await listDrPairs())
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

  const problem = pairFormProblem(form)
  const set = (k: keyof PairForm) => (e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, [k]: e.target.value })

  const onCreate = async () => {
    if (problem) return
    setCreating(true)
    try {
      await createDrPair({
        name: form.name,
        vm: form.vm,
        namespace: form.namespace || undefined,
        dr_namespace: form.drNamespace || undefined,
        rpo_seconds: Number(form.rpoSeconds),
        rto_seconds: form.rtoSeconds ? Number(form.rtoSeconds) : undefined,
      })
      toast.success(`Pair ${form.name} created`)
      setForm(EMPTY)
      await load()
    } catch (e) {
      toastFailure(toast, 'Could not create pair', e)
    } finally {
      setCreating(false)
    }
  }

  const openFailover = async (name: string) => {
    setTyped('')
    try {
      setPlan(await getFailoverPlan(name))
    } catch (e) {
      toastFailure(toast, 'Could not load failover plan', e)
    }
  }

  const runFailover = async (dryRun: boolean) => {
    if (!plan) return
    setBusy(true)
    try {
      const r = await failoverDrPair(plan.pair, { dryRun, confirm: dryRun ? undefined : typed })
      if (r.dry_run) {
        setPlan(r.plan)
        toast.success(r.plan.allowed ? 'Dry run: failover would proceed' : 'Dry run: failover is blocked, see the plan')
      } else {
        toast.success(`Failover started (restore ${r.restore_operation_id ?? 'queued'})`)
        setPlan(null)
        await load()
      }
    } catch (e) {
      toastFailure(toast, dryRun ? 'Dry run failed' : 'Failover failed', e)
    } finally {
      setBusy(false)
    }
  }

  const onRemove = async () => {
    const name = removing
    setRemoving(null)
    if (!name) return
    try {
      await deleteDrPair(name)
      toast.success(`Pair ${name} removed`)
      await load()
    } catch (e) {
      toastFailure(toast, 'Could not remove pair', e)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Disaster Recovery"
        description="Replication pairs keep a verified off-cluster backup of a VM within its RPO."
        eyebrow="Experimental"
        onRefresh={load}
        refreshing={loading}
      />

      {!canAdmin && (
        <p className="text-sm text-[var(--zf-warning)]" role="status">
          Disaster recovery needs the admin role (cluster.admin). Actions are hidden.
        </p>
      )}

      {disabled !== null ? (
        <ExperimentalDisabledNotice feature="DR replication" message={disabled} hint="and ZORVIA_FEATURE_DR_REPLICATION=1 (plus an off-cluster backup target)" />
      ) : (
        <>
          {error && <ErrorBanner title="Could not load DR pairs" headline={error} hints={[]} onRetry={load} />}

          <Card>
            <CardBody className="space-y-3">
              <div className="text-sm font-medium text-[var(--zf-ink)] flex items-center gap-2">
                Replication pairs <ExperimentalBadge />
              </div>
              {loading ? (
                <p className="text-sm text-[var(--zf-muted)]">Loading…</p>
              ) : (
                <DrPairsTable pairs={pairs} canAdmin={canAdmin} onFailover={openFailover} onDelete={setRemoving} />
              )}
              <p className="text-xs text-[var(--zf-muted)]">
                Same-cluster object-store replication, not site failover: it does not protect against losing the cluster that runs Zorvia.
              </p>
            </CardBody>
          </Card>

          {plan && canAdmin && (
            <Card>
              <CardBody>
                <FailoverPanel
                  plan={plan}
                  typed={typed}
                  busy={busy}
                  onTyped={setTyped}
                  onDryRun={() => runFailover(true)}
                  onConfirm={() => runFailover(false)}
                  onClose={() => setPlan(null)}
                />
              </CardBody>
            </Card>
          )}

          {canAdmin && (
            <Card>
              <CardBody className="space-y-3">
                <div className="text-sm font-medium text-[var(--zf-ink)]">New pair</div>
                <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                  <input aria-label="Pair name" placeholder="Pair name" value={form.name} onChange={set('name')} className={INPUT_CLASS} />
                  <input aria-label="VM" placeholder="VM" value={form.vm} onChange={set('vm')} className={INPUT_CLASS} />
                  <input aria-label="Namespace" placeholder="Namespace (default)" value={form.namespace} onChange={set('namespace')} className={INPUT_CLASS} />
                  <input aria-label="DR namespace" placeholder="DR namespace (zorvia-dr)" value={form.drNamespace} onChange={set('drNamespace')} className={INPUT_CLASS} />
                  <input aria-label="RPO seconds" placeholder={`RPO seconds (${RPO_MIN_SECONDS}..${RPO_MAX_SECONDS})`} value={form.rpoSeconds} onChange={set('rpoSeconds')} className={INPUT_CLASS} />
                  <input aria-label="RTO seconds" placeholder="RTO seconds (3600)" value={form.rtoSeconds} onChange={set('rtoSeconds')} className={INPUT_CLASS} />
                </div>
                {problem && form.name && <p className="text-xs text-[var(--zf-warning)]" role="alert">{problem}</p>}
                <button type="button" className="zf-btn zf-btn-primary zf-btn-sm" disabled={!!problem || creating} onClick={onCreate}>
                  Create pair
                </button>
              </CardBody>
            </Card>
          )}
        </>
      )}

      {removing && (
        <ConfirmDialog
          title={`Remove pair ${removing}?`}
          message="The pair stops replicating. Existing off-cluster backups and any DR VM are not deleted."
          confirmLabel="Remove"
          onConfirm={onRemove}
          onCancel={() => setRemoving(null)}
        />
      )}
    </div>
  )
}
