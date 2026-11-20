// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { Shield, CheckCircle, AlertTriangle, XCircle, RefreshCw } from 'lucide-react'
import { getComplianceSummary, rescan, ComplianceSummary } from '../api/compliance'
import { useToastContext } from '../contexts/ToastContext'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function scoreColor(score: number): string {
  if (score >= 90) return 'text-[var(--zf-success)]'
  if (score >= 70) return 'text-[var(--zf-warning)]'
  return 'text-[var(--zf-danger)]'
}

function scoreBorder(score: number): string {
  if (score >= 90) return 'border-[var(--zf-success)]'
  if (score >= 70) return 'border-[var(--zf-warning)]'
  return 'border-[var(--zf-danger)]'
}

function statusIcon(status: string) {
  switch (status) {
    case 'pass': return <CheckCircle className="w-5 h-5 text-[var(--zf-success)]" />
    case 'warning': return <AlertTriangle className="w-5 h-5 text-[var(--zf-warning)]" />
    default: return <XCircle className="w-5 h-5 text-[var(--zf-danger)]" />
  }
}

function statusBadge(status: string): string {
  switch (status) {
    case 'pass': return 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25'
    case 'warning': return 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25'
    default: return 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'
  }
}

export default function ComplianceDashboard() {
  const toast = useToastContext()
  const [data, setData] = useState<ComplianceSummary | null>(null)
  const [loading, setLoading] = useState(true)
  const [scanning, setScanning] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [selectedCategory, setSelectedCategory] = useState('all')

  const fetchCompliance = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setData(await getComplianceSummary())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load compliance data', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchCompliance() }, [fetchCompliance])

  const handleRescan = async () => {
    setScanning(true)
    try {
      setData(await rescan())
      toast.success('Compliance scan complete')
    } catch (err) {
      toastFailure(toast, 'Rescan failed', err)
    } finally {
      setScanning(false)
    }
  }

  const filteredChecks = data?.checks.filter(c => selectedCategory === 'all' || c.category === selectedCategory) ?? []

  return (
    <div className="space-y-6">
      <PageHeader
        title="Compliance"
        description="Live PCI-DSS / HIPAA / SOC 2 checks against real VM specs — recomputed on every load"
        onRefresh={fetchCompliance}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={handleRescan} disabled={scanning} className="zf-btn zf-btn-primary zf-btn-sm">
            <RefreshCw className={`w-4 h-4 ${scanning ? 'animate-spin' : ''}`} /> {scanning ? 'Scanning…' : 'Rescan'}
          </button>
        }
      />

      {loadError && (
        <ErrorBanner title="Could not load compliance data" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchCompliance} />
      )}

      {loading && !data && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Running compliance checks…
        </div>
      ) : !loadError && data ? (
        <>
          <div className="grid grid-cols-1 lg:grid-cols-4 gap-3">
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-6 flex flex-col items-center justify-center">
              <div className={`w-24 h-24 rounded-full flex items-center justify-center bg-[var(--zf-surface)] border-4 ${scoreBorder(data.score)}`}>
                <span className={`text-3xl font-bold ${scoreColor(data.score)}`}>{data.score}</span>
              </div>
              <span className="text-xs text-[var(--zf-muted)] mt-2">Compliance Score</span>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-success)]">{data.passed}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Passed</div>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-warning)]">{data.warnings}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Warnings</div>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-danger)]">{data.failed}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Failed</div>
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            <button onClick={() => setSelectedCategory('all')} className={`zf-chip ${selectedCategory === 'all'? 'zf-chip-active' : ''}`}>All ({data.checks.length})</button>
            {data.categories.map(cat => (
              <button key={cat} onClick={() => setSelectedCategory(cat)} className={`zf-chip ${selectedCategory === cat? 'zf-chip-active' : ''}`}>{cat}</button>
            ))}
          </div>

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            {filteredChecks.length === 0 ? (
              <div className="p-10 text-center text-[var(--zf-muted)] text-sm">
                <Shield className="w-10 h-10 mx-auto mb-3 opacity-50" />
                No checks in this category
              </div>
            ) : (
              <div className="divide-y divide-[var(--zf-hairline)]/30">
                {filteredChecks.map(check => (
                  <div key={check.id} className="px-5 py-3 flex items-start gap-3">
                    {statusIcon(check.status)}
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium text-[var(--zf-ink)]">{check.name}</div>
                      <div className="text-xs text-[var(--zf-muted)] mt-0.5">{check.description}</div>
                      {check.remediation && <div className="text-xs text-[var(--zf-muted)] mt-1 font-mono">{check.remediation}</div>}
                    </div>
                    <span className={`px-2 py-0.5 rounded-full text-xs font-medium border shrink-0 ${statusBadge(check.status)}`}>{check.category}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      ) : null}
    </div>
  )
}
