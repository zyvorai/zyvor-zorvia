// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { AutoscalerStatus, DecisionRecord } from '../api/autoscaler'
import { StatusBadge } from './ui'
import { describeDecision, formatLoad, outcomeBadge } from '../utils/autoscaler'

export function AutoscalerTargetsTable({ targets }: { targets: AutoscalerStatus['targets'] }) {
  if (targets.length === 0) {
    return <p className="text-sm text-[var(--zf-muted)]">No targets configured.</p>
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" aria-label="Autoscaler targets">
        <thead>
          <tr className="text-left text-xs text-[var(--zf-muted)]">
            <th className="py-2 pr-3">Target</th>
            <th className="pr-3">Resource</th>
            <th className="pr-3">Bounds</th>
            <th className="pr-3">Thresholds (down / up)</th>
            <th className="pr-3">Streak (up / down of needed)</th>
            <th className="pr-3">Last action</th>
          </tr>
        </thead>
        <tbody>
          {targets.map((t) => (
            <tr key={t.id} className="border-t border-[var(--zf-hairline)] align-top">
              <td className="py-2 pr-3 font-medium text-[var(--zf-ink)]">{t.id}</td>
              <td className="pr-3 text-[var(--zf-muted)]">
                {t.kind === 'vm_pool' ? 'pool' : 'vCPU'} {t.namespace}/{t.name} ({t.load_metric})
              </td>
              <td className="pr-3">
                {t.min} to {t.max}, step {t.step}
              </td>
              <td className="pr-3">
                {t.scale_down_threshold}% / {t.scale_up_threshold}%
              </td>
              <td className="pr-3">
                {t.up_streak} / {t.down_streak} of {t.required_samples}
              </td>
              <td className="pr-3 text-[var(--zf-muted)]">
                {t.last_action ? new Date(t.last_action).toLocaleString() : 'never'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function DecisionsTable({ decisions }: { decisions: DecisionRecord[] }) {
  if (decisions.length === 0) {
    return <p className="text-sm text-[var(--zf-muted)]">No decisions recorded yet.</p>
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" aria-label="Autoscaler decisions">
        <thead>
          <tr className="text-left text-xs text-[var(--zf-muted)]">
            <th className="py-2 pr-3">Time</th>
            <th className="pr-3">Target</th>
            <th className="pr-3">Load</th>
            <th className="pr-3">Source</th>
            <th className="pr-3">Decision</th>
            <th className="pr-3">Outcome</th>
            <th className="pr-3">Reason</th>
          </tr>
        </thead>
        <tbody>
          {decisions.map((d) => {
            const badge = outcomeBadge(d.outcome)
            return (
              <tr key={d.seq} className="border-t border-[var(--zf-hairline)] align-top">
                <td className="py-2 pr-3 text-[var(--zf-muted)]">{new Date(d.timestamp).toLocaleString()}</td>
                <td className="pr-3 font-medium text-[var(--zf-ink)]">{d.target}</td>
                <td className="pr-3">
                  {formatLoad(d.load_percent)} {d.metric}
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">{d.source}</td>
                <td className="pr-3">{describeDecision(d)}</td>
                <td className="pr-3">
                  <StatusBadge status={badge.status} label={badge.label} />
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  {d.reason}
                  {d.count > 1 ? ` (x${d.count})` : ''}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
