// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { getSecuritySummary, SecuritySummary } from '../api/security'
import { useToastContext } from '../contexts/ToastContext'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function severityColor(severity: string): string {
  const s = (severity || '').toLowerCase()
  if (s === 'critical') return 'bg-[var(--zf-danger)]/10 text-[var(--zf-danger)] border border-[var(--zf-danger)]/25'
  if (s === 'warning') return 'bg-[var(--zf-warning)]/10 text-[var(--zf-warning)] border border-[var(--zf-warning)]/25'
  return 'bg-[var(--zf-link)]/10 text-[var(--zf-link)] border border-[var(--zf-link)]/25'
}

function riskColor(score: number): string {
  if (score >= 80) return 'text-[var(--zf-success)]'
  if (score >= 50) return 'text-[var(--zf-warning)]'
  return 'text-[var(--zf-danger)]'
}

function riskBorder(score: number): string {
  if (score >= 80) return 'border-[var(--zf-success)]'
  if (score >= 50) return 'border-[var(--zf-warning)]'
  return 'border-[var(--zf-danger)]'
}

export default function SecurityDashboard() {
  const toast = useToastContext()
  const [data, setData] = useState<SecuritySummary | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const fetchData = useCallback(async () => {
    setLoading(true)
    try {
      const result = await getSecuritySummary()
      setData(result)
      setLoadError(null)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load security data', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => {
    fetchData()
    const interval = setInterval(fetchData, 15000)
    return () => clearInterval(interval)
  }, [fetchData])

  const alerts = data?.alerts ?? []
  const failedLogins = data?.failed_logins ?? []
  const listeningPorts = data?.listening_ports ?? []
  const riskScore = data?.risk_score ?? 100

  return (
    <div className="space-y-6">
      <PageHeader
        title="Security"
        description="Assembled from the real audit trail and real exposed VM services. Risk score is a documented heuristic, not a certified metric."
        onRefresh={fetchData}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load security data" headline={loadError} hints={hintsForError(loadError, 'auth')} onRetry={fetchData} />
      )}

      {loading && !data && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading…
        </div>
      ) : !loadError ? (
        <>
          <div className="grid grid-cols-1 lg:grid-cols-4 gap-3">
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-6 flex flex-col items-center justify-center">
              <div className={`w-24 h-24 rounded-full flex items-center justify-center bg-[var(--zf-surface)] border-4 ${riskBorder(riskScore)}`}>
                <span className={`text-3xl font-bold ${riskColor(riskScore)}`}>{riskScore}</span>
              </div>
              <span className="text-xs text-[var(--zf-muted)] mt-2">Risk Score (heuristic)</span>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-danger)]">{alerts.filter(a => a.severity === 'critical').length}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Critical Alerts</div>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-warning)]">{alerts.filter(a => a.severity === 'warning').length}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Warnings</div>
            </div>
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] px-4 py-3 flex flex-col justify-center">
              <div className="text-2xl font-bold text-[var(--zf-ink)]">{failedLogins.length}</div>
              <div className="text-xs text-[var(--zf-muted)] mt-1">Failed Logins</div>
            </div>
          </div>

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <div className="px-5 py-4 border-b border-[var(--zf-hairline)] flex items-center justify-between">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Recent Failures (from the audit trail)</h3>
              <span className="text-xs font-medium text-[var(--zf-muted)] bg-[var(--zf-canvas)] px-2.5 py-1 rounded-full">{alerts.length}</span>
            </div>
            {alerts.length === 0 ? (
              <div className="p-8 text-center text-sm text-[var(--zf-muted)]">No recent failures recorded</div>
            ) : (
              <div className="divide-y divide-[var(--zf-hairline)]/30">
                {alerts.map((alert, i) => (
                  <div key={i} className="px-5 py-3 flex items-start gap-3">
                    <span className={`text-[10px] font-medium px-2 py-0.5 rounded-full shrink-0 mt-0.5 ${severityColor(alert.severity)}`}>{alert.severity}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-[var(--zf-ink)]">{alert.message}</div>
                      <div className="text-[10px] text-[var(--zf-muted)] mt-0.5">{alert.source} · {new Date(alert.timestamp).toLocaleString()}</div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {failedLogins.length > 0 && (
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
              <div className="px-5 py-4 border-b border-[var(--zf-hairline)]"><h3 className="text-sm font-semibold text-[var(--zf-ink)]">Failed Logins</h3></div>
              <DataTable
                columns={[
                  { key: 'time', header: 'Time', className: 'px-5', render: (login) => <span className="text-xs text-[var(--zf-muted)]">{new Date(login.timestamp).toLocaleString()}</span> },
                  { key: 'user', header: 'User', render: (login) => <span className="text-xs text-[var(--zf-ink)]">{login.user}</span> },
                ]}
                rows={failedLogins}
                getRowKey={(login) => `${login.timestamp}-${login.user}`}
                bordered={false}
              />
            </div>
          )}

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <div className="px-5 py-4 border-b border-[var(--zf-hairline)] flex items-center justify-between">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Exposed Services</h3>
              <span className="text-xs font-medium text-[var(--zf-muted)] bg-[var(--zf-canvas)] px-2.5 py-1 rounded-full">{listeningPorts.length}</span>
            </div>
            {listeningPorts.length === 0 ? (
              <div className="p-8 text-center text-sm text-[var(--zf-muted)]">No VM has an exposed port right now</div>
            ) : (
              <DataTable
                columns={[
                  { key: 'port', header: 'Port', className: 'px-5', render: (port) => <span className="text-xs text-[var(--zf-ink)] font-mono">{port.port ?? '—'}</span> },
                  { key: 'protocol', header: 'Protocol', render: (port) => <span className="text-xs text-[var(--zf-ink)] uppercase">{port.protocol}</span> },
                  { key: 'vm', header: 'VM', render: (port) => <span className="text-xs text-[var(--zf-muted)]">{port.vm_name || '—'}</span> },
                ]}
                rows={listeningPorts}
                getRowKey={(port) => `${port.port}-${port.protocol}-${port.vm_name}`}
                bordered={false}
              />
            )}
          </div>
        </>
      ) : null}
    </div>
  )
}
