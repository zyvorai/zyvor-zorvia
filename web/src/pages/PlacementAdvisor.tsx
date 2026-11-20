// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { Scale, Check, X } from 'lucide-react'
import { getPlacementRecommendation, getRebalanceSuggestions, PlacementRecommendation, RebalanceMove } from '../api/placement'
import { listVMs, VM } from '../api/vm'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

export default function PlacementAdvisor() {
  const toast = useToastContext()
  const [vms, setVms] = useState<VM[]>([])
  const [selectedVm, setSelectedVm] = useState('')
  const [recommendation, setRecommendation] = useState<PlacementRecommendation | null>(null)
  const [moves, setMoves] = useState<RebalanceMove[]>([])
  const [loading, setLoading] = useState(true)
  const [recommending, setRecommending] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [recError, setRecError] = useState<string | null>(null)

  const fetchOverview = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [vmList, rebalance] = await Promise.all([listVMs(), getRebalanceSuggestions()])
      setVms(vmList)
      setMoves(rebalance)
      if (!selectedVm && vmList[0]) setSelectedVm(vmList[0].name)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load placement data', err)
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { fetchOverview() }, [fetchOverview])

  const handleRecommend = async () => {
    if (!selectedVm) return
    setRecommending(true)
    setRecError(null)
    try {
      setRecommendation(await getPlacementRecommendation(selectedVm))
    } catch (err) {
      setRecError(formatUserError(err))
      toastFailure(toast, 'Failed to get placement recommendation', err)
    } finally {
      setRecommending(false)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Placement Advisor"
        description="Node placement scoring and cluster-balance suggestions, computed from real node capacity and VM usage. Recommendation only — nothing here moves a VM automatically."
        onRefresh={fetchOverview}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load placement data" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchOverview} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading…
        </div>
      ) : !loadError ? (
        <>
          <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5">
            <h3 className="text-sm font-semibold text-[var(--zf-ink)] mb-3">Where should this VM run?</h3>
            <div className="flex flex-col md:flex-row gap-3 items-start md:items-end">
              <div className="flex-1 w-full">
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">VM</label>
                <select value={selectedVm} onChange={(e) => setSelectedVm(e.target.value)} className="input-field text-sm w-full">
                  {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                </select>
              </div>
              <button type="button" onClick={handleRecommend} disabled={recommending || !selectedVm} className="zf-btn zf-btn-primary zf-btn-sm">
                {recommending ? 'Scoring…' : 'Recommend'}
              </button>
            </div>

            {recError && <p className="text-sm text-[var(--zf-danger)] mt-3">{recError}</p>}

            {recommendation && (
              <div className="mt-4 space-y-2">
                <p className="text-sm text-[var(--zf-muted)]">
                  Best node: <span className="font-semibold text-[var(--zf-ink)]">{recommendation.selected_node ?? 'none eligible'}</span>
                </p>
                <DataTable
                  columns={[
                    { key: 'node', header: 'Node', render: (c) => <span className="font-medium text-[var(--zf-ink)]">{c.node}</span> },
                    {
                      key: 'eligible',
                      header: 'Eligible',
                      render: (c) => (c.eligible ? <Check className="w-4 h-4 text-[var(--zf-success)]" /> : <X className="w-4 h-4 text-[var(--zf-danger)]" />),
                    },
                    { key: 'score', header: 'Score', render: (c) => <span className="text-[var(--zf-muted)]">{c.score.toFixed(1)}</span> },
                    { key: 'reasons', header: 'Reasons', render: (c) => <span className="text-xs text-[var(--zf-muted)]">{c.reasons.join('; ')}</span> },
                  ]}
                  rows={recommendation.candidates}
                  getRowKey={(c) => c.node}
                  bordered={false}
                />
              </div>
            )}
          </div>

          <div>
            <h3 className="text-sm font-semibold text-[var(--zf-ink)] mb-3">Rebalance Suggestions</h3>
            {moves.length === 0 ? (
              <EmptyState icon={<Scale className="w-8 h-8" />} title="Cluster is balanced" description="No VM is on a node significantly more loaded than the cluster average." />
            ) : (
              <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
                <DataTable
                  columns={[
                    { key: 'vm', header: 'VM', className: 'px-5', render: (m) => <span className="font-medium text-[var(--zf-ink)]">{m.workload}</span> },
                    { key: 'from', header: 'From', render: (m) => <span className="font-mono text-xs text-[var(--zf-muted)]">{m.from_node}</span> },
                    { key: 'to', header: 'To', render: (m) => <span className="font-mono text-xs text-[var(--zf-muted)]">{m.to_node}</span> },
                    { key: 'reason', header: 'Reason', render: (m) => <span className="text-xs text-[var(--zf-muted)]">{m.reason}</span> },
                  ]}
                  rows={moves}
                  getRowKey={(m) => `${m.workload}-${m.from_node}-${m.to_node}`}
                  bordered={false}
                />
              </div>
            )}
          </div>
        </>
      ) : null}
    </div>
  )
}
