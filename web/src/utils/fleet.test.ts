// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, expect, it } from 'vitest'
import { fleetIsStale, fleetRows, inventoryCount } from './fleet'
import type { FleetCluster } from '../api/fleet'
const cluster = (name: string, vms: FleetCluster['vms']): FleetCluster => ({
  name, vms, namespaces: ['tenant'], environment: '', region: '', observed_at: '', health: 'unknown',
  node_count: null, ready_nodes: null, vm_count: vms?.length ?? null, ready_vms: null, issues: [],
})
describe('fleet inventory', () => {
  it('distinguishes unknown counts from empty inventory', () => {
    expect(inventoryCount(null)).toBe('Unknown')
    expect(inventoryCount(0)).toBe('0')
  })
  it('marks old, invalid and implausibly future timestamps stale', () => {
    const now = Date.parse('2026-10-01T00:00:00Z')
    expect(fleetIsStale(new Date(now - 10_000).toISOString(), now)).toBe(false)
    expect(fleetIsStale(new Date(now - 61_000).toISOString(), now)).toBe(true)
    expect(fleetIsStale(new Date(now + 61_000).toISOString(), now)).toBe(true)
    expect(fleetIsStale('invalid', now)).toBe(true)
  })
  it('keeps remote identity and filters cluster, namespace, VM and status', () => {
    const rows = [cluster('east', [{ namespace: 'tenant', name: 'db', status: 'Running', ready: true }]), cluster('west', [{ namespace: 'tenant', name: 'db', status: 'Stopped', ready: false }]), cluster('unknown', null)]
    expect(fleetRows(rows, '', '')).toHaveLength(2)
    expect(fleetRows(rows, ' EAST ', '')[0].cluster).toBe('east')
    expect(fleetRows(rows, 'tenant', '')).toHaveLength(2)
    expect(fleetRows(rows, 'db', 'west')[0].status).toBe('Stopped')
    expect(fleetRows(rows, 'running', 'west')).toHaveLength(0)
    expect(fleetRows(rows, '', 'unknown')).toHaveLength(0)
  })
})
