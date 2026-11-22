// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { CertificateInfo, CertManagerCertificate } from '../api/certificates'
import { StatusBadge } from './ui'
import { certLocation, expiryBadge, keySummary, readyBadge } from '../utils/certificates'

export function CertificatesTable({ certs }: { certs: CertificateInfo[] }) {
  if (certs.length === 0) {
    return <p className="text-sm text-[var(--zf-muted)]">No certificates match.</p>
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" aria-label="Certificates">
        <thead>
          <tr className="text-left text-xs text-[var(--zf-muted)]">
            <th className="py-2 pr-3">Location</th>
            <th className="pr-3">Subject / issuer</th>
            <th className="pr-3">SANs</th>
            <th className="pr-3">Expires</th>
            <th className="pr-3">Key</th>
            <th className="pr-3">Flags</th>
          </tr>
        </thead>
        <tbody>
          {certs.map((c) => {
            const badge = expiryBadge(c)
            return (
              <tr key={`${c.source}/${c.namespace}/${c.name}/${c.key}/${c.index}`} className="border-t border-[var(--zf-hairline)] align-top">
                <td className="py-2 pr-3 font-medium text-[var(--zf-ink)]">
                  {certLocation(c)}
                  <div className="text-xs text-[var(--zf-muted)]">{c.source}, {c.role}</div>
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  <div>{c.subject || '—'}</div>
                  <div className="text-xs">issued by {c.issuer || '—'}</div>
                </td>
                <td className="pr-3 text-xs text-[var(--zf-muted)] max-w-xs break-words">
                  {c.sans.length > 0 ? c.sans.join(', ') : '—'}
                </td>
                <td className="pr-3">
                  <StatusBadge status={badge.status} label={badge.label} />
                  <div className="text-xs text-[var(--zf-muted)] mt-1">{new Date(c.not_after).toLocaleDateString()}</div>
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  {keySummary(c)}
                  <div className="text-xs">{c.signature_algorithm}</div>
                </td>
                <td className="pr-3 text-xs">
                  {c.flags.length === 0 ? (
                    <span className="text-[var(--zf-muted)]">none</span>
                  ) : (
                    c.flags.map((f) => (
                      <div key={f} className={f.startsWith('weak') || f === 'expired' ? 'text-[var(--zf-danger)]' : 'text-[var(--zf-warning)]'}>
                        {f}
                      </div>
                    ))
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

export function CertManagerTable({
  certs,
  canAdmin,
  busy,
  onRenew,
}: {
  certs: CertManagerCertificate[]
  canAdmin: boolean
  busy: boolean
  onRenew: (c: CertManagerCertificate) => void
}) {
  if (certs.length === 0) {
    return <p className="text-sm text-[var(--zf-muted)]">No cert-manager Certificates found.</p>
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" aria-label="cert-manager Certificates">
        <thead>
          <tr className="text-left text-xs text-[var(--zf-muted)]">
            <th className="py-2 pr-3">Certificate</th>
            <th className="pr-3">Ready</th>
            <th className="pr-3">Issuer</th>
            <th className="pr-3">Expires</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {certs.map((c) => {
            const badge = readyBadge(c)
            return (
              <tr key={`${c.namespace}/${c.name}`} className="border-t border-[var(--zf-hairline)] align-top">
                <td className="py-2 pr-3 font-medium text-[var(--zf-ink)]">
                  {c.namespace}/{c.name}
                  {c.secret_name && <div className="text-xs text-[var(--zf-muted)]">Secret {c.secret_name}</div>}
                </td>
                <td className="pr-3">
                  <StatusBadge status={badge.status} label={badge.label} title={c.ready_message ?? undefined} />
                  {c.ready !== 'True' && c.ready_message && (
                    <div className="text-xs text-[var(--zf-muted)] mt-1 max-w-xs">{c.ready_message}</div>
                  )}
                </td>
                <td className="pr-3 text-[var(--zf-muted)]">{c.issuer ?? '—'}</td>
                <td className="pr-3 text-[var(--zf-muted)]">
                  {c.not_after ? new Date(c.not_after).toLocaleDateString() : '—'}
                  {c.renewal_time && <div className="text-xs">renews {new Date(c.renewal_time).toLocaleDateString()}</div>}
                </td>
                <td className="text-right whitespace-nowrap">
                  {canAdmin && (
                    <button
                      type="button"
                      className="zf-btn zf-btn-ghost zf-btn-sm"
                      disabled={busy || c.issuing}
                      title={c.issuing ? 'An issuance is already in progress' : undefined}
                      onClick={() => onRenew(c)}
                    >
                      Renew
                    </button>
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
