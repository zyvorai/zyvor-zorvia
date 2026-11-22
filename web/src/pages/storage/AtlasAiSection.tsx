// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useState } from 'react'
import { Sparkles, Search } from 'lucide-react'
import {
  askAtlasAdvisor,
  getAtlasAnomalies,
  getAtlasIncidents,
  runAtlasWhatIf,
  AtlasAdvisorResponse,
  AtlasAnomaliesResponse,
  AtlasIncidentsResponse,
  AtlasWhatIfResponse,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'

const RISK_STYLES: Record<string, string> = {
  low: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  medium: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
  high: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
  critical: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
}

/** Read-mostly AI insights: a deterministic local advisor (never mutates
    storage, never calls an external model unless Atlas itself is configured
    with a provider -- this UI always passes mode=local), anomaly detection,
    correlated incidents, and a what-if capacity projector. */
export default function AtlasAiSection() {
  const toast = useToastContext()
  const [anomalies, setAnomalies] = useState<AtlasAnomaliesResponse | null>(null)
  const [incidents, setIncidents] = useState<AtlasIncidentsResponse | null>(null)
  const [advisor, setAdvisor] = useState<AtlasAdvisorResponse | null>(null)
  const [question, setQuestion] = useState('')
  const [whatIf, setWhatIf] = useState<AtlasWhatIfResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  useEffect(() => {
    Promise.all([getAtlasAnomalies(), getAtlasIncidents()])
      .then(([a, i]) => {
        setAnomalies(a)
        setIncidents(i)
      })
      .catch((e) => toastFailure(toast, 'Failed to load AI insights', e))
      .finally(() => setLoading(false))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const ask = (q: string) => {
    setBusy('advisor')
    askAtlasAdvisor(q)
      .then(setAdvisor)
      .catch((e) => toastFailure(toast, 'Advisor request failed', e))
      .finally(() => setBusy(null))
  }

  const runWhatIf = (addTiB: number, horizonDays: number, assumeResolved: boolean) => {
    setBusy('what-if')
    runAtlasWhatIf({
      add_capacity_bytes: addTiB * 1024 * 1024 * 1024 * 1024,
      horizon_days: horizonDays,
      assume_alerts_resolved: assumeResolved,
    })
      .then(setWhatIf)
      .catch((e) => toastFailure(toast, 'What-if projection failed', e))
      .finally(() => setBusy(null))
  }

  if (loading) return null

  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
        <Sparkles className="w-4 h-4 text-[var(--zf-muted)]" />
        AI insights
      </h2>
      <p className="text-xs text-[var(--zf-muted)]">
        Read-only: a deterministic advisor over Atlas's own inventory/metrics. Recommendations only — nothing here executes a runbook.
      </p>

      <div className="flex items-end gap-2 flex-wrap">
        <div className="flex-1 min-w-[200px]">
          <label className="block text-xs text-[var(--zf-muted)] mb-1">Ask the advisor</label>
          <input
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            placeholder="is storage healthy?"
            className="w-full px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
          />
        </div>
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => ask(question.trim())}
          className="zf-btn zf-btn-primary zf-btn-sm"
        >
          <Search className="w-4 h-4" />
          {busy === 'advisor' ? 'Asking…' : 'Ask'}
        </button>
      </div>

      {advisor && (
        <div className="rounded-md border border-[var(--zf-hairline)] p-3 space-y-2">
          <div className="flex items-center justify-between gap-2">
            <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${RISK_STYLES[advisor.risk_level] ?? RISK_STYLES.low}`}>
              {advisor.risk_level} risk ({advisor.risk_score})
            </span>
          </div>
          <p className="text-sm text-[var(--zf-ink)]">{advisor.summary}</p>
          {advisor.actions.length > 0 && (
            <ul className="text-xs text-[var(--zf-muted)] space-y-1">
              {advisor.actions.map((a) => (
                <li key={a.title}>• {a.title} — {a.rationale}</li>
              ))}
            </ul>
          )}
        </div>
      )}

      {anomalies && anomalies.anomalies.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Anomalies ({anomalies.telemetry_status})</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {anomalies.anomalies.map((a) => (
              <div key={a.id} className="py-2 text-sm">
                <div className="font-medium text-[var(--zf-ink)]">{a.label}</div>
                <div className="text-xs text-[var(--zf-muted)]">{a.explanation}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {incidents && incidents.incidents.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Correlated incidents</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {incidents.incidents.map((i) => (
              <div key={i.id} className="py-2 text-sm">
                <div className="font-medium text-[var(--zf-ink)]">{i.title}</div>
                <div className="text-xs text-[var(--zf-muted)]">{i.likely_cause}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      <WhatIfForm disabled={busy !== null} onRun={runWhatIf} />

      {whatIf && (
        <div className="rounded-md border border-[var(--zf-hairline)] p-3 text-sm space-y-1">
          <div className="text-[var(--zf-ink)]">
            Capacity: {whatIf.baseline.capacity_used_percent.toFixed(1)}% → {whatIf.projected.capacity_used_percent.toFixed(1)}% over {whatIf.horizon_days}d
          </div>
          <div className="text-xs text-[var(--zf-muted)]">Risk delta: {whatIf.risk_delta}</div>
        </div>
      )}
    </div>
  )
}

function WhatIfForm({
  disabled,
  onRun,
}: {
  disabled: boolean
  onRun: (addTiB: number, horizonDays: number, assumeResolved: boolean) => void
}) {
  const [addTiB, setAddTiB] = useState(2)
  const [horizonDays, setHorizonDays] = useState(30)
  const [assumeResolved, setAssumeResolved] = useState(false)

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Add capacity (TiB)</label>
        <input
          type="number"
          min={0}
          value={addTiB}
          onChange={(e) => setAddTiB(parseFloat(e.target.value) || 0)}
          className="w-24 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Horizon (days)</label>
        <input
          type="number"
          min={1}
          value={horizonDays}
          onChange={(e) => setHorizonDays(parseInt(e.target.value) || 30)}
          className="w-24 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <label className="flex items-center gap-1.5 text-xs text-[var(--zf-muted)] pb-1.5">
        <input type="checkbox" checked={assumeResolved} onChange={(e) => setAssumeResolved(e.target.checked)} />
        Assume alerts resolved
      </label>
      <button
        type="button"
        disabled={disabled}
        onClick={() => onRun(addTiB, horizonDays, assumeResolved)}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        Project
      </button>
    </div>
  )
}
