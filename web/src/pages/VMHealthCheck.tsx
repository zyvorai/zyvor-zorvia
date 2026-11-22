// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useEffect, useCallback } from 'react'
import { listVMs, getGuestInsight, VM, GuestInsightReport } from '../api/vm'
import ErrorBanner from '../components/ErrorBanner'
import PageLoadBanner from '../components/PageLoadBanner'
import { PageHeader } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { hintsForError } from '../utils/daemonHints'
import { usePageLoader } from '../hooks/usePageLoader'

type CheckStatus = 'pass' | 'warning' | 'fail'
interface HealthCheck { name: string; status: CheckStatus; message: string }

function StatusIcon({ status }: { status: CheckStatus }) {
  if (status === 'pass') return <span className="flex items-center justify-center w-6 h-6 rounded-full bg-[var(--zf-success)]/10 text-[var(--zf-success)] text-xs">&#10003;</span>
  if (status === 'warning') return <span className="flex items-center justify-center w-6 h-6 rounded-full bg-[var(--zf-warning)]/10 text-[var(--zf-warning)] text-xs">&#9888;</span>
  return <span className="flex items-center justify-center w-6 h-6 rounded-full bg-[var(--zf-danger)]/10 text-[var(--zf-danger)] text-xs">&#10007;</span>
}

/** Derives operator-facing health checks from the real guest-insight report
 * (KubeVirt VMI status + QEMU Guest Agent, no fake data). */
function buildChecks(report: GuestInsightReport): HealthCheck[] {
  const checks: HealthCheck[] = []

  checks.push({
    name: 'VM Phase',
    status: report.phase === 'Running' ? 'pass' : report.phase === 'NotRunning' ? 'fail' : 'warning',
    message: `VM is ${report.phase}`,
  })

  const agentMessage: Record<GuestInsightReport['agent_state'], string> = {
    'connected': 'QEMU Guest Agent is connected and reporting status',
    'not-detected': 'Guest Agent not detected -- install qemu-guest-agent in the guest for full insight',
    'not-running': 'VM is not running, so the Guest Agent cannot be checked',
    'unknown': 'Guest Agent status could not be determined',
  }
  checks.push({
    name: 'Guest Agent',
    status: report.agent_state === 'connected' ? 'pass' : report.agent_state === 'not-running' ? 'fail' : 'warning',
    message: agentMessage[report.agent_state],
  })

  const primaryIface = report.interfaces.find((i) => i.primary_ip) || report.interfaces[0]
  checks.push({
    name: 'Network',
    status: primaryIface?.primary_ip ? 'pass' : 'warning',
    message: primaryIface?.primary_ip
      ? `Primary IP ${primaryIface.primary_ip}${primaryIface.name ? ` on ${primaryIface.name}` : ''}`
      : 'No IP address reported yet',
  })

  checks.push({
    name: 'Readiness Score',
    status: report.readiness_score >= 80 ? 'pass' : report.readiness_score >= 40 ? 'warning' : 'fail',
    message: `${report.readiness_score}/100`,
  })

  for (const rec of report.recommendations) {
    checks.push({ name: 'Recommendation', status: 'warning', message: rec })
  }

  return checks
}

export default function VMHealthCheck() {
  const [vms, setVMs] = useState<VM[]>([])
  const [selectedVM, setSelectedVM] = useState('')
  const [report, setReport] = useState<GuestInsightReport | null>(null)
  const [checks, setChecks] = useState<HealthCheck[]>([])
  const [checking, setChecking] = useState(false)
  const [checkError, setCheckError] = useState<string | null>(null)
  const { loading: loadingVMs, loadError, run } = usePageLoader('Failed to load VMs')

  const fetchVMs = useCallback(() => {
    return run(async () => {
      setVMs(await listVMs())
    })
  }, [run])

  useEffect(() => { void fetchVMs() }, [fetchVMs])

  const runHealthCheck = async () => {
    if (!selectedVM) return
    setChecking(true)
    setCheckError(null)
    setReport(null)
    try {
      const insight = await getGuestInsight(selectedVM)
      setReport(insight)
      setChecks(buildChecks(insight))
    } catch (err) {
      setCheckError(formatUserError(err))
    } finally {
      setChecking(false)
    }
  }

  if (loadingVMs && vms.length === 0 && !loadError) {
    return (
      <div className="space-y-6">
        <PageHeader title="VM Health Check" description="Guest Agent and readiness checks derived from real KubeVirt VMI status" />
        <div className="bg-[var(--zf-canvas)] rounded-xl p-10 border border-[var(--zf-hairline)] flex flex-col items-center justify-center text-[var(--zf-muted)] gap-3">
          <div className="w-6 h-6 border-2 border-[var(--zf-hairline)] border-t-[var(--zf-link)] rounded-full animate-spin" />
          <span className="text-sm">Loading VMs…</span>
        </div>
      </div>
    )
  }

  const overallHealthy = report ? !checks.some((c) => c.status === 'fail') && report.readiness_score >= 80 : false

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="VM Health Check"
        description="Guest Agent and readiness checks derived from real KubeVirt VMI status"
        onRefresh={() => void fetchVMs()}
        refreshing={loadingVMs}
      />
      <PageLoadBanner title="Could not load VMs" headline={loadError} onRetry={() => void fetchVMs()} />

      {!loadError && (
        <>
          <div className="bg-[var(--zf-canvas)] rounded-xl p-4 border border-[var(--zf-hairline)]">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4 items-end">
              <div className="md:col-span-2">
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Select VM</label>
                <select
                  value={selectedVM}
                  onChange={(e) => setSelectedVM(e.target.value)}
                  className="w-full bg-[var(--zf-canvas)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] focus:outline-none focus:ring-1 focus:ring-[var(--zf-ink)]"
                >
                  <option value="">Select a VM…</option>
                  {vms.map((vm) => (
                    <option key={vm.name} value={vm.name}>
                      {vm.name} {vm.state ? `(${vm.state})` : ''}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <button
                  onClick={runHealthCheck}
                  disabled={!selectedVM || checking}
                  className="w-full zf-btn zf-btn-primary"
                >
                  {checking ? 'Checking…' : 'Run Health Check'}
                </button>
              </div>
            </div>
          </div>

          {checkError && (
            <ErrorBanner
              title="Health check failed"
              headline={checkError}
              hints={hintsForError(checkError, 'vm')}
              onRetry={runHealthCheck}
            />
          )}

          {checking && (
            <div className="bg-[var(--zf-canvas)] rounded-xl p-10 border border-[var(--zf-hairline)] flex flex-col items-center justify-center text-[var(--zf-muted)] gap-3">
              <div className="w-6 h-6 border-2 border-[var(--zf-hairline)] border-t-[var(--zf-link)] rounded-full animate-spin" />
              <span className="text-sm">Running health checks on {selectedVM}…</span>
            </div>
          )}

          {report && !checking && (
            <div className="flex flex-col gap-4">
              <div
                className={`rounded-xl p-4 border flex items-center gap-3 ${overallHealthy ? 'bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25' : 'bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'}`}
              >
                <div
                  className={`w-8 h-8 rounded-full flex items-center justify-center ${overallHealthy ? 'bg-[var(--zf-success)]/20 text-[var(--zf-success)]' : 'bg-[var(--zf-danger)]/20 text-[var(--zf-danger)]'}`}
                >
                  {overallHealthy ? '✓' : '⚠'}
                </div>
                <div>
                  <div className="text-sm font-semibold text-[var(--zf-ink)]">
                    {overallHealthy ? 'Healthy' : 'Issues Found'}
                  </div>
                  <div className="text-xs text-[var(--zf-muted)]">
                    {checks.filter((c) => c.status === 'pass').length} of {checks.length} checks passed for {report.vm}
                    {report.os_name ? ` · ${report.os_name}` : ''}
                  </div>
                </div>
              </div>

              <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] divide-y divide-[var(--zf-hairline)]">
                {checks.map((check, idx) => (
                  <div key={idx} className="flex items-start gap-3 p-4">
                    <StatusIcon status={check.status} />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium text-[var(--zf-ink)]">{check.name}</div>
                      <div className="text-xs text-[var(--zf-muted)] mt-0.5">{check.message}</div>
                    </div>
                    <span
                      className={`px-2 py-0.5 rounded-full text-xs font-medium border ${check.status === 'pass' ? 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25' : check.status === 'warning' ? 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25' : 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'}`}
                    >
                      {check.status}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {!report && !checkError && !checking && (
            <div className="bg-[var(--zf-canvas)] rounded-xl p-10 border border-[var(--zf-hairline)] flex flex-col items-center justify-center text-[var(--zf-muted)] gap-2">
              <span className="text-sm">Select a VM and run a health check</span>
            </div>
          )}
        </>
      )}
    </div>
  )
}
