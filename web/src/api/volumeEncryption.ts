// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { featureJson } from './experimental'

/** Volume envelope encryption (Experimental, ZORVIA_EXPERIMENTAL=1,
    cluster.admin). See docs/VOLUME_ENCRYPTION.md. Responses never carry key
    material. */
const BASE = '/api/volume-encryption'

export interface VolumeEncryptionStatus {
  feature: string
  maturity: string
  provider: string | null
  ready: boolean
}

export interface EncryptRequest {
  namespace: string
  vm: string
  volume: string
  size: string
  storage_class: string
  mode: 'storage_class'
  acknowledge_unverified_encryption: boolean
}

export interface EncryptPlan {
  ready: boolean
  blockers: string[]
  warnings: string[]
  steps: string[]
}

export interface EncryptApplied {
  data_volume: string
  key_secret: string
  warnings: string[]
}

export interface RotateRequest {
  namespace: string
  vm: string
  volume: string
}

export interface RotateResult {
  key_secret: string
  version: number
  rewrapped: boolean
  provider: string
}

export const getVolumeEncryptionStatus = () => featureJson<VolumeEncryptionStatus>(`${BASE}/status`)
export const planEncryption = (r: EncryptRequest) => featureJson<EncryptPlan>(`${BASE}/plan`, 'POST', r)
export const applyEncryption = (r: EncryptRequest) => featureJson<EncryptApplied>(`${BASE}/apply`, 'POST', r)
export const rotateKey = (r: RotateRequest) => featureJson<RotateResult>(`${BASE}/rotate`, 'POST', r)
