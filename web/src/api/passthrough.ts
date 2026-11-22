// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiDelete, apiGet, apiPost } from './client'

export interface PassthroughDevice {
  name: string
  device_name: string
  kind?: 'gpu' | 'host-device'
}

export interface DeviceNode {
  node: string
  /** The node runs the static CPU manager policy (needed for dedicated CPUs). */
  cpu_manager?: boolean
  /** Free hugepages by page size ("2Mi", "1Gi"). */
  hugepages?: Record<string, number>
  /** Passthrough candidates: extended resources the device plugin advertises, with free counts. */
  devices: Record<string, number>
  /** KubeVirt's own pseudo-devices (kvm, tun, ...). */
  builtin: Record<string, number>
}

export interface PermittedDevice {
  resource_name: string
  kind: 'pci' | 'mediated' | 'usb'
  selector: string
  external: boolean
}

export interface SriovPool {
  namespace: string
  network: string
  resource_name: string
  /** Free virtual functions across all nodes. */
  free: number
  nodes: number
}

export interface DeviceInventory {
  nodes: DeviceNode[]
  sriov_pools?: SriovPool[]
  /** Likely kind (`gpu` / `host-device`) per advertised resource. */
  kind_hints?: Record<string, 'gpu' | 'host-device'>
  /** Null when the KubeVirt CR could not be read. */
  kubevirt: { feature_gates: string[]; permitted: PermittedDevice[] } | null
  multus_installed: boolean
}

export interface DeviceIssue {
  severity: 'error' | 'warning'
  subject: string
  message: string
}

export interface DevicePreflight {
  ok: boolean
  issues: DeviceIssue[]
}

/** Cluster-admin only. */
export async function getDeviceInventory(): Promise<DeviceInventory> {
  return apiGet<DeviceInventory>('/api/v1/devices')
}

/** Would these devices / SR-IOV networks start? Read-only; VM creation runs the same check. */
export interface PlacementOptions {
  dedicated_cpus?: boolean
  numa_passthrough?: boolean
  /** "2Mi" or "1Gi" */
  hugepages?: string
}

export async function preflightDevices(
  devices: PassthroughDevice[],
  sriovNetworks: string[] = [],
  namespace?: string,
  placement: PlacementOptions = {},
): Promise<DevicePreflight> {
  return apiPost<DevicePreflight>('/api/v1/devices/preflight', {
    devices,
    sriov_networks: sriovNetworks,
    ...(namespace ? { namespace } : {}),
    ...placement,
  })
}

/** A device to add to the KubeVirt CR's permittedHostDevices (cluster admin). */
export type PermitDevice =
  | { kind: 'pci'; resource_name: string; pci_vendor_selector: string; external_resource_provider?: boolean }
  | { kind: 'mediated'; resource_name: string; mdev_name_selector: string; external_resource_provider?: boolean }
  | { kind: 'usb'; resource_name: string; vendor: string; product: string; external_resource_provider?: boolean }

export interface PermitResult {
  resource_name: string
  /** false when exactly this entry was already permitted. */
  changed: boolean
}

export async function permitDevice(device: PermitDevice): Promise<PermitResult> {
  return apiPost<PermitResult>('/api/v1/devices/permitted', device)
}

export async function unpermitDevice(resourceName: string): Promise<void> {
  return apiDelete(`/api/v1/devices/permitted?resource_name=${encodeURIComponent(resourceName)}`)
}

/** Resource names worth suggesting: advertised on a node and with units free. */
export function suggestedResources(inv: DeviceInventory): string[] {
  const names = new Set<string>()
  for (const n of inv.nodes) for (const [k, v] of Object.entries(n.devices)) if (v > 0) names.add(k)
  return [...names].sort()
}
