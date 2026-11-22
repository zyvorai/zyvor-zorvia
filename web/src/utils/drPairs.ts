// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import type { DrHealth, DrPairStatus } from '../api/drPairs'

export const RPO_MIN_SECONDS = 60
export const RPO_MAX_SECONDS = 604800

/** 3725 -> "1h 2m"; 59 -> "59s"; 0 -> "0s". */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) return '—'
  const s = Math.floor(seconds)
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`
  if (m > 0) return `${m}m`
  return `${s}s`
}

/** Maps the API health string onto a StatusBadge status key. */
export function healthBadge(health: DrHealth): { status: string; label: string } {
  switch (health) {
    case 'healthy':
      return { status: 'healthy', label: 'Healthy' }
    case 'rpo_breached':
      return { status: 'failed', label: 'RPO breached' }
    case 'no_backup_yet':
      return { status: 'pending', label: 'No backup yet' }
    case 'failing':
      return { status: 'warning', label: 'Backups failing' }
    case 'paused':
      return { status: 'paused', label: 'Paused' }
    case 'failed_over':
      return { status: 'warning', label: 'Failed over' }
    default:
      return { status: 'unknown', label: String(health) }
  }
}

/** Lag as a share of the RPO, capped at 100 for a progress bar. */
export function lagPercent(p: Pick<DrPairStatus, 'lag_seconds' | 'rpo_seconds'>): number {
  if (!p.rpo_seconds) return 0
  return Math.min(100, Math.round((p.lag_seconds / p.rpo_seconds) * 100))
}

export interface PairForm {
  name: string
  vm: string
  namespace: string
  drNamespace: string
  rpoSeconds: string
  rtoSeconds: string
}

const LABEL = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/

export function pairFormProblem(f: PairForm): string | null {
  for (const [what, v, optional] of [
    ['Pair name', f.name, false],
    ['VM', f.vm, false],
    ['Namespace', f.namespace, true],
    ['DR namespace', f.drNamespace, true],
  ] as const) {
    if (!v) {
      if (optional) continue
      return `${what} is required.`
    }
    if (v.length > 63 || !LABEL.test(v)) return `${what} must be a DNS label (lowercase letters, digits, "-").`
  }
  const rpo = Number(f.rpoSeconds)
  if (!Number.isInteger(rpo) || rpo < RPO_MIN_SECONDS || rpo > RPO_MAX_SECONDS) {
    return `RPO must be a whole number of seconds from ${RPO_MIN_SECONDS} to ${RPO_MAX_SECONDS}.`
  }
  if (f.rtoSeconds) {
    const rto = Number(f.rtoSeconds)
    if (!Number.isInteger(rto) || rto <= 0) return 'RTO must be a positive whole number of seconds.'
  }
  return null
}

/** The phrase a real failover must echo back; the server sends it in the plan. */
export function failoverPhraseFor(pair: string): string {
  return `failover ${pair}`
}

/** A confirmed failover is only offered when the plan allows it, there are no
    blockers, and the typed phrase matches exactly. */
export function canConfirmFailover(
  plan: { allowed: boolean; blockers: string[]; confirm_phrase: string } | null,
  typed: string,
): boolean {
  return !!plan && plan.allowed && plan.blockers.length === 0 && typed === plan.confirm_phrase
}
