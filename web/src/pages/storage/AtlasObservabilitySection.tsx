// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useState } from 'react'
import { Activity, Download } from 'lucide-react'
import {
  getAtlasMetricsSummary,
  getAtlasMetricsForecast,
  getAtlasChargeback,
  getAtlasPolicyDrift,
  listAtlasAudit,
  exportAtlasAuditCsv,
  AtlasChargebackTenant,
  AtlasAuditEntry,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { formatBytes } from '../../utils/format'

/** Read-only observability views scoped to the Atlas area of the Storage
    page -- deliberately not merged into Zorvia's own audit trail or cost/
    FinOps surfaces, since those are Zorvia's own equivalents and unifying
    them is a separate product decision, not something to fold in here. */
export default function AtlasObservabilitySection() {
  const toast = useToastContext()
  const [summary, setSummary] = useState<Record<string, unknown> | null>(null)
  const [forecast, setForecast] = useState<{ days_to_full: number | null; growth_bytes_per_day: number } | null>(null)
  const [chargeback, setChargeback] = useState<{ usd_per_gib_month: number; tenants: AtlasChargebackTenant[] } | null>(null)
  const [drift, setDrift] = useState<{ count: number; drift: Record<string, unknown>[] } | null>(null)
  const [audit, setAudit] = useState<AtlasAuditEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [exporting, setExporting] = useState(false)

  useEffect(() => {
    Promise.all([getAtlasMetricsSummary(), getAtlasMetricsForecast(), getAtlasChargeback(), getAtlasPolicyDrift(), listAtlasAudit(20)])
      .then(([s, f, c, d, a]) => {
        setSummary(s)
        setForecast(f)
        setChargeback(c)
        setDrift(d)
        setAudit(a)
      })
      .catch((e) => toastFailure(toast, 'Failed to load observability data', e))
      .finally(() => setLoading(false))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const downloadCsv = () => {
    setExporting(true)
    exportAtlasAuditCsv()
      .then((csv) => {
        const blob = new Blob([csv], { type: 'text/csv' })
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = 'atlas-audit.csv'
        a.click()
        URL.revokeObjectURL(url)
      })
      .catch((e) => toastFailure(toast, 'Failed to export audit CSV', e))
      .finally(() => setExporting(false))
  }

  if (loading) return null

  const usedBytes = typeof summary?.used_capacity_bytes === 'number' ? summary.used_capacity_bytes : null
  const rawBytes = typeof summary?.raw_capacity_bytes === 'number' ? summary.raw_capacity_bytes : null

  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
        <Activity className="w-4 h-4 text-[var(--zf-muted)]" />
        Observability
      </h2>

      {(usedBytes != null || rawBytes != null) && (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
          {usedBytes != null && <StatTile label="Used" value={formatBytes(usedBytes)} />}
          {rawBytes != null && <StatTile label="Raw capacity" value={formatBytes(rawBytes)} />}
          {forecast && (
            <StatTile label="Days to full" value={forecast.days_to_full != null ? Math.round(forecast.days_to_full).toString() : '—'} />
          )}
        </div>
      )}

      {chargeback && chargeback.tenants.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">
            Chargeback {chargeback.usd_per_gib_month > 0 ? `($${chargeback.usd_per_gib_month}/GiB-month)` : '(usage only)'}
          </h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {chargeback.tenants.map((t) => (
              <div key={t.tenant_id} className="flex items-center justify-between py-2 text-sm">
                <span className="text-[var(--zf-ink)]">{t.tenant_id}</span>
                <span className="text-xs text-[var(--zf-muted)]">
                  {t.used_gib.toFixed(1)} GiB · {t.volume_count} vol
                  {t.estimated_usd_month > 0 ? ` · $${t.estimated_usd_month.toFixed(2)}/mo` : ''}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {drift && drift.count > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-warning)] mb-2">Policy drift ({drift.count})</h3>
          <div className="text-xs text-[var(--zf-muted)] space-y-1">
            {drift.drift.map((d, i) => (
              <div key={i}>• {JSON.stringify(d)}</div>
            ))}
          </div>
        </div>
      )}

      {audit.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-xs font-medium text-[var(--zf-muted)]">Recent audit entries</h3>
            <button type="button" disabled={exporting} onClick={downloadCsv} className="zf-btn zf-btn-ghost zf-btn-sm">
              <Download className="w-3.5 h-3.5" />
              {exporting ? 'Exporting…' : 'Export CSV'}
            </button>
          </div>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {audit.map((entry) => (
              <div key={entry.id} className="py-1.5 text-xs text-[var(--zf-muted)]">
                {entry.created_at} · {entry.actor_id} · {entry.action} · {entry.resource_type}/{entry.resource_id} · {entry.status}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function StatTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-md border border-[var(--zf-hairline)] px-3 py-2">
      <div className="text-lg font-semibold text-[var(--zf-ink)]">{value}</div>
      <div className="text-xs text-[var(--zf-muted)]">{label}</div>
    </div>
  )
}
