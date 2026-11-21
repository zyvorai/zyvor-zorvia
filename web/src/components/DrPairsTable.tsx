// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { DrPairStatus } from '../api/drPairs'
import { StatusBadge } from './ui'
import { formatDuration, healthBadge, lagPercent } from '../utils/drPairs'

export default function DrPairsTable({
  pairs,
  canAdmin,
  onFailover,
  onDelete,
}: {
  pairs: DrPairStatus[]
  canAdmin: boolean
  onFailover: (name: string) => void
  onDelete: (name: string) => void
}) {
  if (pairs.length === 0) {
    return <p className="text-sm text-[var(--zf-muted)]">No replication pairs yet.</p>
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" aria-label="DR replication pairs">
        <thead>
          <tr className="text-left text-xs text-[var(--zf-muted)]">
            <th className="py-2 pr-3">Pair</th>
            <th className="pr-3">VM to DR VM</th>
            <th className="pr-3">Health</th>
            <th className="pr-3">RPO (lag / target)</th>
            <th className="pr-3">RTO (measured / target)</th>
            <th className="pr-3">Last backup</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {pairs.map((p) => {
            const badge = healthBadge(p.health)
            const pct = lagPercent(p)
            return (
              <tr key={p.name} className="border-t border-[var(--zf-hairline)] align-top">
                <td className="py-2 pr-3 font-medium text-[var(--zf-ink)]">{p.name}</td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  {p.namespace}/{p.vm} to {p.dr_namespace}/{p.dr_vm}
                </td>
                <td className="pr-3">
                  <StatusBadge status={badge.status} label={badge.label} title={p.last_error ?? undefined} />
                  {p.last_error && <div className="text-xs text-[var(--zf-danger)] mt-1 max-w-xs">{p.last_error}</div>}
                </td>
                <td className="pr-3">
                  <div>
                    {formatDuration(p.lag_seconds)} / {formatDuration(p.rpo_seconds)}{' '}
                    <span className={p.rpo_met ? 'text-[var(--zf-muted)]' : 'text-[var(--zf-danger)]'}>
                      {p.rpo_met ? 'met' : 'breached'}
                    </span>
                  </div>
                  <div
                    role="progressbar"
                    aria-label={`RPO lag for ${p.name}`}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={pct}
                    className="h-1.5 w-28 mt-1 rounded bg-[var(--zf-fill-tertiary)] overflow-hidden"
                  >
                    <div className={`h-full ${p.rpo_met ? 'bg-[var(--zf-cta)]' : 'bg-[var(--zf-danger)]'}`} style={{ width: `${pct}%` }} />
                  </div>
                </td>
                <td className="pr-3">
                  {formatDuration(p.rto_measured_seconds)} / {formatDuration(p.rto_target_seconds)}
                  {p.rto_met !== null && (
                    <span className={p.rto_met ? ' text-[var(--zf-muted)]' : ' text-[var(--zf-danger)]'}>
                      {p.rto_met ? ' met' : ' missed'}
                    </span>
                  )}
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  {p.last_success ? new Date(p.last_success).toLocaleString() : 'never'}
                </td>
                <td className="text-right whitespace-nowrap">
                  {canAdmin && (
                    <>
                      <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" onClick={() => onFailover(p.name)} disabled={p.phase === 'failed_over' || p.health === 'failed_over'}>
                        Failover
                      </button>
                      <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" onClick={() => onDelete(p.name)}>
                        Remove
                      </button>
                    </>
                  )}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
