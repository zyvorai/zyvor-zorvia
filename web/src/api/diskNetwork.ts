// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost } from './client'
import { API_BASE_URL } from './config'

export interface VmInterface {
  name: string
  network: string | null
  model: string | null
  network_type: 'pod' | 'multus'
  mac_address: string | null
  ip_address: string | null
  ip_addresses: string[]
  interface_name: string | null
}

export async function listInterfaces(vmName: string): Promise<VmInterface[]> {
  return apiGet<VmInterface[]>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/interfaces`)
}

export interface VmDisk {
  name: string
  boot_order: number | null
  device_type: 'disk' | 'cdrom' | 'lun'
  bus: string | null
  source: 'pvc' | 'dataVolume' | 'containerDisk' | 'blank' | 'cloudInit' | 'unknown'
  size: string | null
  resizable: boolean
}

export async function listDisks(vmName: string): Promise<VmDisk[]> {
  return apiGet<VmDisk[]>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/disks`)
}

export interface ResizeDiskResponse {
  disk: string
  size?: string
}

export async function resizeDisk(vmName: string, diskName: string, size: string): Promise<ResizeDiskResponse> {
  return apiPost<ResizeDiskResponse>(
    `${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/disks/${encodeURIComponent(diskName)}/resize`,
    { size },
  )
}
