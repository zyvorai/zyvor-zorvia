// Copyright 2026 Zyvor AI Labs · SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0
import type { ImportRequest } from '../api/vmImport'

export interface ImportDraft {
  vcenter: string
  datacenter: string
  secret: string
  namespace: string
  sources: string
  storageClass: string
  scratchSize: string
  attachment: string
  sourceNetwork: string
  includePodNetwork: boolean
  start: boolean
  requireAgent: boolean
  bootTimeout: number
}

export const emptyImportDraft: ImportDraft = {
  vcenter: '', datacenter: '', secret: '', namespace: 'vms', sources: '',
  storageClass: '', scratchSize: '200Gi', attachment: '', sourceNetwork: '',
  includePodNetwork: true, start: false, requireAgent: false, bootTimeout: 600,
}

export function buildImportRequest(d: ImportDraft): ImportRequest {
  const sources = d.sources.split('\n').map(s => s.trim()).filter(Boolean)
  if (sources.length === 0 || sources.length > 50) throw new Error('Enter between 1 and 50 VM names, one per line.')
  if (new Set(sources).size !== sources.length) throw new Error('A source VM appears more than once.')
  if (!d.vcenter.trim() || !d.secret.trim() || !d.namespace.trim()) throw new Error('Enter the vCenter host, credentials Secret, and target namespace.')
  if (d.requireAgent && !d.start) throw new Error('Guest-agent verification requires starting the imported VM.')
  if (!Number.isInteger(d.bootTimeout) || d.bootTimeout < 30 || d.bootTimeout > 3600) throw new Error('Boot timeout must be between 30 and 3600 seconds.')
  const attachment = d.attachment.trim()
  return {
    source: { vcenter: d.vcenter.trim(), secret_name: d.secret.trim(), ...(d.datacenter.trim() ? { datacenter: d.datacenter.trim() } : {}) },
    namespace: d.namespace.trim(),
    scratch_size: d.scratchSize.trim(),
    vms: sources.map(source_vm => ({
      source_vm,
      ...(d.storageClass.trim() ? { storage_class: d.storageClass.trim() } : {}),
      start: d.start,
      require_guest_agent: d.requireAgent,
      boot_timeout_secs: d.bootTimeout,
      ...(attachment ? { networks: [
        ...(d.includePodNetwork ? [{ name: 'default' }] : []),
        { name: 'prod', attachment, ...(d.sourceNetwork.trim() ? { source_network: d.sourceNetwork.trim() } : {}) },
      ] } : {}),
    })),
  }
}

/** A successful check is reusable only for the exact request that was checked. */
export function preflightMatches(request: ImportRequest, checked: string | null): boolean {
  return checked !== null && JSON.stringify(request) === checked
}
