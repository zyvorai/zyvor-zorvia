// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { featureJson } from './experimental'

/** Certificate lifecycle (Experimental, ZORVIA_EXPERIMENTAL=1, cluster.admin).
    See docs/CERTIFICATE_LIFECYCLE.md. Responses carry certificate metadata only,
    never PEM text or keys. */
const BASE = '/api/certificates'

export interface CertificateInfo {
  source: 'tls-secret' | 'serving-cert' | 'ca-file'
  namespace: string | null
  name: string
  key: string
  role: 'leaf' | 'chain' | 'ca'
  index: number
  subject: string
  issuer: string
  sans: string[]
  not_before: string
  not_after: string
  days_remaining: number
  expired: boolean
  not_yet_valid: boolean
  key_type: string
  key_bits: number
  curve: string | null
  signature_algorithm: string
  self_signed: boolean
  fingerprint_sha256: string
  weak: string[]
  flags: string[]
}

export interface CertificateInventory {
  generated_at: string
  certificates: CertificateInfo[]
  summary: { total: number; expired: number; expiring_30d: number; self_signed: number; weak: number }
  skipped: string[]
}

export interface CertificateStatus {
  feature: string
  maturity: string
  alert_thresholds_days: number[]
  check_interval_secs: number
  scheduler_leader: boolean
  last_check: string | null
  last_certificates: number
  last_alerts: number
  last_error: string | null
  watched_files: { name: string; env: string }[]
}

export interface CertificateAlert {
  id: string
  message: string
  notification_type: 'Success' | 'Info' | 'Warning' | 'Error'
  created_at: string
}

export interface CertManagerCertificate {
  namespace: string
  name: string
  secret_name: string | null
  ready: 'True' | 'False' | 'Unknown' | 'Absent' | string
  ready_reason: string | null
  ready_message: string | null
  issuing: boolean
  not_after: string | null
  renewal_time: string | null
  issuer: string | null
  dns_names: string[]
}

export interface CertManagerList {
  installed: boolean
  certificates: CertManagerCertificate[]
}

export const getCertificateStatus = () => featureJson<CertificateStatus>(`${BASE}/status`)
export const getCertificateInventory = (namespace?: string) =>
  featureJson<CertificateInventory>(
    namespace ? `${BASE}/inventory?namespace=${encodeURIComponent(namespace)}` : `${BASE}/inventory`,
  )
export const getCertificateAlerts = () => featureJson<CertificateAlert[]>(`${BASE}/alerts`)
export const listCertManagerCertificates = () => featureJson<CertManagerList>(`${BASE}/cert-manager`)
export const renewCertManagerCertificate = (namespace: string, name: string, confirm: string) =>
  featureJson<{ renewing: boolean; certificate: CertManagerCertificate }>(`${BASE}/cert-manager/renew`, 'POST', {
    namespace,
    name,
    confirm,
  })
