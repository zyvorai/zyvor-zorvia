// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, it, expect } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { CertificatesTable, CertManagerTable } from '../CertificatesTable'
import ExperimentalDisabledNotice from '../ExperimentalDisabledNotice'
import type { CertificateInfo, CertManagerCertificate } from '../../api/certificates'

const cert = (over: Partial<CertificateInfo> = {}): CertificateInfo => ({
  source: 'tls-secret', namespace: 'prod', name: 'web', key: 'tls.crt', role: 'leaf', index: 0,
  subject: 'CN=web.example.com', issuer: 'CN=Test CA', sans: ['DNS:web.example.com'],
  not_before: '2026-01-01T00:00:00Z', not_after: '2026-12-01T00:00:00Z', days_remaining: 100,
  expired: false, not_yet_valid: false, key_type: 'EC', key_bits: 256, curve: 'prime256v1',
  signature_algorithm: 'ecdsa-with-SHA256', self_signed: false, fingerprint_sha256: 'ab', weak: [], flags: [],
  ...over,
})

const cm = (over: Partial<CertManagerCertificate> = {}): CertManagerCertificate => ({
  namespace: 'web', name: 'site', secret_name: 'site-tls', ready: 'False', ready_reason: 'Failed',
  ready_message: 'issuer not ready', issuing: false, not_after: '2026-09-01T00:00:00Z',
  renewal_time: null, issuer: 'ClusterIssuer/le', dns_names: [], ...over,
})

describe('CertificatesTable', () => {
  it('shows subject, issuer, SANs, expiry badge, key and flags', () => {
    const html = renderToStaticMarkup(
      <CertificatesTable certs={[cert({ days_remaining: 5, flags: ['weak-key:RSA-1024', 'self-signed'] })]} />,
    )
    expect(html).toContain('prod/web (tls.crt)')
    expect(html).toContain('CN=web.example.com')
    expect(html).toContain('issued by CN=Test CA')
    expect(html).toContain('DNS:web.example.com')
    expect(html).toContain('5 days left')
    expect(html).toContain('EC 256 (prime256v1)')
    expect(html).toContain('weak-key:RSA-1024')
    expect(html).toContain('self-signed')
  })
  it('renders an empty state', () => {
    expect(renderToStaticMarkup(<CertificatesTable certs={[]} />)).toContain('No certificates match')
  })
})

describe('CertManagerTable', () => {
  const noop = () => {}
  it('shows Ready state and a renew action for admins only', () => {
    const admin = renderToStaticMarkup(<CertManagerTable certs={[cm()]} canAdmin busy={false} onRenew={noop} />)
    expect(admin).toContain('Not ready')
    expect(admin).toContain('issuer not ready')
    expect(admin).toContain('>Renew</button>')
    const viewer = renderToStaticMarkup(<CertManagerTable certs={[cm()]} canAdmin={false} busy={false} onRenew={noop} />)
    expect(viewer).not.toContain('Renew</button>')
  })
  it('disables renew while an issuance is running', () => {
    const html = renderToStaticMarkup(<CertManagerTable certs={[cm({ issuing: true })]} canAdmin busy={false} onRenew={noop} />)
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Renew/)
    expect(html).toContain('Issuing')
  })
})

describe('certificate lifecycle disabled state', () => {
  it('uses the shared experimental-disabled notice', () => {
    const html = renderToStaticMarkup(<ExperimentalDisabledNotice feature="Certificate lifecycle" message="turn it on" />)
    expect(html).toContain('Certificate lifecycle: experimental, disabled')
    expect(html).toContain('ZORVIA_EXPERIMENTAL=1')
  })
})
