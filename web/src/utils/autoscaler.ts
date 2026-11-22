// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import type { DecisionOutcome, DecisionRecord } from '../api/autoscaler'

/** Maps a decision outcome onto a StatusBadge status key and label. */
export function outcomeBadge(o: DecisionOutcome): { status: string; label: string } {
  switch (o) {
    case 'applied':
      return { status: 'healthy', label: 'Applied' }
    case 'recommended':
      return { status: 'pending', label: 'Recommended' }
    case 'failed':
      return { status: 'failed', label: 'Failed' }
    case 'held':
      return { status: 'paused', label: 'Held' }
    case 'skipped':
      return { status: 'warning', label: 'Skipped' }
    default:
      return { status: 'unknown', label: String(o) }
  }
}

/** "scale up 2 to 3", "hold at 3", "skipped". */
export function describeDecision(d: DecisionRecord): string {
  switch (d.decision) {
    case 'scale_up':
    case 'scale_down': {
      const verb = d.decision === 'scale_up' ? 'scale up' : 'scale down'
      return d.from !== null && d.to !== null ? `${verb} ${d.from} to ${d.to}` : verb
    }
    case 'hold':
      return d.from !== null ? `hold at ${d.from}` : 'hold'
    default:
      return 'skipped'
  }
}

export function formatLoad(p: number | null | undefined): string {
  return p === null || p === undefined || !Number.isFinite(p) ? '—' : `${p.toFixed(1)}%`
}
