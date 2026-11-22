// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useCallback, useEffect, useState } from 'react'
import { PageHeader, Card, CardBody } from '../components/ui'
import ErrorBanner from '../components/ErrorBanner'
import ExperimentalDisabledNotice, { ExperimentalBadge, INPUT_CLASS } from '../components/ExperimentalDisabledNotice'
import { AutoscalerTargetsTable, DecisionsTable } from '../components/AutoscalerPanel'
import {
  getAutoscalerDecisions,
  getAutoscalerStatus,
  type AutoscalerDecisions,
  type AutoscalerStatus,
} from '../api/autoscaler'
import { isExperimentalDisabled } from '../utils/experimental'
import { formatUserError } from '../utils/apiError'

export default function Autoscaler() {
  const [status, setStatus] = useState<AutoscalerStatus | null>(null)
  const [decisions, setDecisions] = useState<AutoscalerDecisions | null>(null)
  const [target, setTarget] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [disabled, setDisabled] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const s = await getAutoscalerStatus()
      setStatus(s)
      setDecisions(await getAutoscalerDecisions({ limit: 100, target: target || undefined }))
      setDisabled(null)
    } catch (e) {
      if (isExperimentalDisabled(e)) setDisabled(e.message)
      else setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [target])

  useEffect(() => {
    load()
  }, [load])

  const header = (
    <PageHeader
      title="Autoscaler"
      description="Read-only view of the autoscaler loop: targets, hysteresis state and recent decisions."
      eyebrow="Experimental"
      onRefresh={load}
      refreshing={loading}
    />
  )

  if (disabled !== null) {
    return (
      <div className="space-y-6">
        {header}
        <ExperimentalDisabledNotice feature="Autoscaler" message={disabled} />
      </div>
    )
  }

  const running = status?.enabled === true
  return (
    <div className="space-y-6">
      {header}
      {error && <ErrorBanner title="Could not load autoscaler" headline={error} hints={[]} onRetry={load} />}

      {status && !running && (
        <ExperimentalDisabledNotice
          feature="Autoscaler loop"
          message={status.disabled_reason ?? undefined}
          hint="and ZORVIA_AUTOSCALER_LOOP=1 with ZORVIA_AUTOSCALER_TARGETS"
        />
      )}

      <Card>
        <CardBody className="space-y-2">
          <div className="text-sm font-medium text-[var(--zf-ink)] flex items-center gap-2">
            Loop <ExperimentalBadge />
          </div>
          {loading && !status ? (
            <p className="text-sm text-[var(--zf-muted)]">Loading…</p>
          ) : status && running ? (
            <dl className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
              <div>
                <dt className="text-xs text-[var(--zf-muted)]">Mode</dt>
                <dd>{status.status.mode === 'apply' ? 'Applying changes' : 'Recommendation only'}</dd>
              </div>
              <div>
                <dt className="text-xs text-[var(--zf-muted)]">Interval</dt>
                <dd>{status.status.interval_secs}s</dd>
              </div>
              <div>
                <dt className="text-xs text-[var(--zf-muted)]">This replica</dt>
                <dd>{status.status.leader === null ? 'no tick yet' : status.status.leader ? 'leader' : 'follower (idle)'}</dd>
              </div>
              <div>
                <dt className="text-xs text-[var(--zf-muted)]">Last tick</dt>
                <dd>{status.status.last_tick ? new Date(status.status.last_tick).toLocaleString() : 'never'}</dd>
              </div>
            </dl>
          ) : null}
        </CardBody>
      </Card>

      {status && running && (
        <Card>
          <CardBody className="space-y-3">
            <div className="text-sm font-medium text-[var(--zf-ink)]">Targets</div>
            <AutoscalerTargetsTable targets={status.targets} />
          </CardBody>
        </Card>
      )}

      {status && running && (
        <Card>
          <CardBody className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <div className="text-sm font-medium text-[var(--zf-ink)]">Recent decisions</div>
              <label className="text-xs text-[var(--zf-muted)] flex items-center gap-2">
                Target
                <select
                  className={INPUT_CLASS}
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  aria-label="Filter by target"
                >
                  <option value="">All</option>
                  {status.targets.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.id}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <p className="text-xs text-[var(--zf-muted)]">
              Kept in memory only (up to 100 per target, lost on restart). Repeated identical holds are folded into one row.
            </p>
            <DecisionsTable decisions={decisions?.decisions ?? []} />
          </CardBody>
        </Card>
      )}
    </div>
  )
}
