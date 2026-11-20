// Copyright 2026 Zyvor AI Labs · SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0
import type { FleetCluster, FleetVm } from '../api/fleet'
export const FLEET_STALE_MS = 60_000
export function fleetIsStale(observedAt: string, now = Date.now()): boolean {
  const time = Date.parse(observedAt)
  return !Number.isFinite(time) || now - time > FLEET_STALE_MS || time > now + FLEET_STALE_MS
}
export function inventoryCount(value: number | null): string {
  return value === null ? 'Unknown' : value.toLocaleString()
}
export function fleetRows(clusters: FleetCluster[], query: string, selectedCluster: string): (FleetVm & { cluster: string })[] {
  const term = query.trim().toLocaleLowerCase()
  return clusters.filter(c => selectedCluster === '' || c.name === selectedCluster)
    .flatMap(c => (c.vms ?? []).map(vm => ({ ...vm, cluster: c.name })))
    .filter(vm => [vm.cluster, vm.namespace, vm.name, vm.status].some(v => v.toLocaleLowerCase().includes(term)))
}
