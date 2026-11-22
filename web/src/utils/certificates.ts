// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { CertificateInfo, CertManagerCertificate } from '../api/certificates'

/** Badge for a certificate: expired is a failure, under a week is a failure,
    under a month a warning. */
export function expiryBadge(c: Pick<CertificateInfo, 'expired' | 'days_remaining' | 'not_yet_valid'>): {
  status: string
  label: string
} {
  if (c.expired) return { status: 'failed', label: 'Expired' }
  if (c.not_yet_valid) return { status: 'warning', label: 'Not yet valid' }
  if (c.days_remaining < 7) return { status: 'failed', label: `${daysText(c.days_remaining)} left` }
  if (c.days_remaining < 30) return { status: 'warning', label: `${daysText(c.days_remaining)} left` }
  return { status: 'healthy', label: `${daysText(c.days_remaining)} left` }
}

export function daysText(d: number): string {
  const n = Math.abs(d)
  return `${n} day${n === 1 ? '' : 's'}`
}

/** Key summary such as "RSA 2048", "EC 256 (prime256v1)" or "Ed25519". */
export function keySummary(c: Pick<CertificateInfo, 'key_type' | 'key_bits' | 'curve'>): string {
  if (c.key_type === 'Ed25519' || c.key_type === 'Ed448') return c.key_type
  const base = `${c.key_type} ${c.key_bits}`
  return c.curve ? `${base} (${c.curve})` : base
}

export function certLocation(c: Pick<CertificateInfo, 'namespace' | 'name' | 'key'>): string {
  return c.namespace ? `${c.namespace}/${c.name} (${c.key})` : `${c.name} (${c.key})`
}

export type CertFilter = 'all' | 'expiring' | 'flagged'

/** "expiring" = expired or under 30 days; "flagged" = any weak/self-signed/expired flag. */
export function filterCertificates(list: CertificateInfo[], filter: CertFilter, search: string): CertificateInfo[] {
  const q = search.trim().toLowerCase()
  return list.filter((c) => {
    if (filter === 'expiring' && !(c.expired || c.days_remaining < 30)) return false
    if (filter === 'flagged' && c.flags.length === 0) return false
    if (!q) return true
    return [c.name, c.namespace ?? '', c.subject, c.issuer, ...c.sans].some((s) => s.toLowerCase().includes(q))
  })
}

/** The phrase the renew endpoint requires. */
export function renewPhrase(namespace: string, name: string): string {
  return `renew ${namespace}/${name}`
}

export function readyBadge(c: Pick<CertManagerCertificate, 'ready' | 'issuing'>): { status: string; label: string } {
  if (c.issuing) return { status: 'in_progress', label: 'Issuing' }
  switch (c.ready) {
    case 'True':
      return { status: 'healthy', label: 'Ready' }
    case 'False':
      return { status: 'failed', label: 'Not ready' }
    default:
      return { status: 'unknown', label: 'Unknown' }
  }
}
