// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from 'vitest'
import { buildImportRequest, emptyImportDraft, preflightMatches } from './vmImportDraft'

const draft = { ...emptyImportDraft, vcenter: 'vc.example.com', secret: 'vc-creds', sources: 'web-01\ndb-01' }

describe('VMware import draft', () => {
  it('leaves VMs stopped and preserves default networking unless explicitly mapped', () => {
    const req = buildImportRequest(draft)
    expect(req.vms).toHaveLength(2)
    expect(req.vms.every(vm => vm.start === false && vm.networks === undefined)).toBe(true)
  })
  it('maps source network labels to a local attachment for every VM', () => {
    const req = buildImportRequest({ ...draft, attachment: 'prod-vlan', sourceNetwork: 'Production', start: true, requireAgent: true })
    expect(req.vms[0].networks).toEqual([{ name: 'default' }, { name: 'prod', attachment: 'prod-vlan', source_network: 'Production' }])
    expect(req.vms[1].require_guest_agent).toBe(true)
    expect(buildImportRequest({ ...draft, attachment: 'isolated', includePodNetwork: false }).vms[0].networks).toHaveLength(1)
  })
  it('invalidates readiness when any request setting changes', () => {
    const req = buildImportRequest(draft)
    const checked = JSON.stringify(req)
    expect(preflightMatches(req, checked)).toBe(true)
    expect(preflightMatches(buildImportRequest({ ...draft, start: true }), checked)).toBe(false)
    expect(preflightMatches(buildImportRequest({ ...draft, attachment: 'new-vlan' }), checked)).toBe(false)
    expect(preflightMatches(req, null)).toBe(false)
  })
  it('rejects ambiguous waves and unsatisfied boot policies', () => {
    expect(() => buildImportRequest({ ...draft, sources: 'web-01\nweb-01' })).toThrow('more than once')
    expect(() => buildImportRequest({ ...draft, sources: '' })).toThrow('1 and 50')
    expect(() => buildImportRequest({ ...draft, requireAgent: true })).toThrow('requires starting')
    expect(() => buildImportRequest({ ...draft, bootTimeout: 29 })).toThrow('30 and 3600')
  })
})
