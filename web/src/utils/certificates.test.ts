// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, it, expect } from 'vitest'
import type { CertificateInfo } from '../api/certificates'
import { certLocation, daysText, expiryBadge, filterCertificates, keySummary, readyBadge, renewPhrase } from './certificates'

const cert = (over: Partial<CertificateInfo> = {}): CertificateInfo => ({
  source: 'tls-secret', namespace: 'prod', name: 'web', key: 'tls.crt', role: 'leaf', index: 0,
  subject: 'CN=web.example.com', issuer: 'CN=Test CA', sans: ['DNS:web.example.com'],
  not_before: '2026-01-01T00:00:00Z', not_after: '2026-12-01T00:00:00Z', days_remaining: 100,
  expired: false, not_yet_valid: false, key_type: 'EC', key_bits: 256, curve: 'prime256v1',
  signature_algorithm: 'ecdsa-with-SHA256', self_signed: false, fingerprint_sha256: 'ab', weak: [], flags: [],
  ...over,
})

describe('expiryBadge', () => {
  it('maps days remaining to a badge', () => {
    expect(expiryBadge(cert({ days_remaining: 100 }))).toEqual({ status: 'healthy', label: '100 days left' })
    expect(expiryBadge(cert({ days_remaining: 29 })).status).toBe('warning')
    expect(expiryBadge(cert({ days_remaining: 6 })).status).toBe('failed')
    expect(expiryBadge(cert({ days_remaining: 1 })).label).toBe('1 day left')
    expect(expiryBadge(cert({ expired: true, days_remaining: -3 }))).toEqual({ status: 'failed', label: 'Expired' })
    expect(expiryBadge(cert({ not_yet_valid: true })).label).toBe('Not yet valid')
  })
  it('pluralises', () => {
    expect(daysText(0)).toBe('0 days')
    expect(daysText(-2)).toBe('2 days')
  })
})

describe('summaries', () => {
  it('describes keys and locations', () => {
    expect(keySummary(cert())).toBe('EC 256 (prime256v1)')
    expect(keySummary(cert({ key_type: 'RSA', key_bits: 2048, curve: null }))).toBe('RSA 2048')
    expect(keySummary(cert({ key_type: 'Ed25519', key_bits: 253, curve: null }))).toBe('Ed25519')
    expect(certLocation(cert())).toBe('prod/web (tls.crt)')
    expect(certLocation(cert({ namespace: null, name: 'serving-cert', key: 'ZORVIA_TLS_CERT' }))).toBe('serving-cert (ZORVIA_TLS_CERT)')
  })
  it('builds the exact renew phrase', () => {
    expect(renewPhrase('web', 'site')).toBe('renew web/site')
  })
  it('maps cert-manager state', () => {
    expect(readyBadge({ ready: 'True', issuing: false }).label).toBe('Ready')
    expect(readyBadge({ ready: 'False', issuing: false }).status).toBe('failed')
    expect(readyBadge({ ready: 'Absent', issuing: false }).label).toBe('Unknown')
    expect(readyBadge({ ready: 'True', issuing: true }).label).toBe('Issuing')
  })
})

describe('filterCertificates', () => {
  const list = [
    cert({ name: 'a' }),
    cert({ name: 'b', days_remaining: 10 }),
    cert({ name: 'c', expired: true, days_remaining: -1, flags: ['expired'] }),
    cert({ name: 'd', flags: ['self-signed'], sans: ['DNS:shop.test'] }),
  ]
  it('filters by expiry and flags', () => {
    expect(filterCertificates(list, 'all', '').length).toBe(4)
    expect(filterCertificates(list, 'expiring', '').map((c) => c.name)).toEqual(['b', 'c'])
    expect(filterCertificates(list, 'flagged', '').map((c) => c.name)).toEqual(['c', 'd'])
  })
  it('searches name, subject and SANs case-insensitively', () => {
    expect(filterCertificates(list, 'all', 'SHOP').map((c) => c.name)).toEqual(['d'])
    expect(filterCertificates(list, 'all', 'web.example').length).toBe(4)
    expect(filterCertificates(list, 'all', 'nomatch')).toEqual([])
  })
})
