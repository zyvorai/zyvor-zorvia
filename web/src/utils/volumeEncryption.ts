// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import type { EncryptRequest, RotateRequest } from '../api/volumeEncryption'

export interface EncryptForm {
  namespace: string
  vm: string
  volume: string
  size: string
  storageClass: string
  acknowledge: boolean
}

const LABEL = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/
const SIZE = /^[1-9]\d*(Ki|Mi|Gi|Ti|K|M|G|T)?$/

function nameProblem(what: string, v: string): string | null {
  if (!v) return `${what} is required.`
  if (v.length > 63 || !LABEL.test(v)) return `${what} must be a DNS label (lowercase letters, digits, "-").`
  return null
}

export function rotateFormProblem(f: Pick<EncryptForm, 'namespace' | 'vm' | 'volume'>): string | null {
  return (
    nameProblem('Namespace', f.namespace) ?? nameProblem('VM', f.vm) ?? nameProblem('Volume', f.volume)
  )
}

export function encryptFormProblem(f: EncryptForm): string | null {
  const base = rotateFormProblem(f)
  if (base) return base
  if (!SIZE.test(f.size)) return 'Size must look like 20Gi (a whole number plus Ki, Mi, Gi or Ti).'
  return nameProblem('StorageClass', f.storageClass)
}

export function toEncryptRequest(f: EncryptForm): EncryptRequest {
  return {
    namespace: f.namespace,
    vm: f.vm,
    volume: f.volume,
    size: f.size,
    storage_class: f.storageClass,
    mode: 'storage_class',
    acknowledge_unverified_encryption: f.acknowledge,
  }
}

export function toRotateRequest(f: Pick<EncryptForm, 'namespace' | 'vm' | 'volume'>): RotateRequest {
  return { namespace: f.namespace, vm: f.vm, volume: f.volume }
}

/** The DataVolume the server creates is named <vm>-<volume>. */
export function dataVolumeName(vm: string, volume: string): string {
  return `${vm}-${volume}`
}

export function rotateSummary(r: { version: number; rewrapped: boolean; provider: string }): string {
  return r.rewrapped
    ? `Data key re-wrapped under the current ${r.provider} key (now version ${r.version}). No data was re-encrypted.`
    : `Already wrapped under the current key (version ${r.version}); nothing changed.`
}
