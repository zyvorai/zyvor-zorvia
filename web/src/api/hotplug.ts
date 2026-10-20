// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { API_BASE_URL } from './config'
import { apiPost, apiFetch } from './client'

export interface HotplugCpuRequest {
  count: number
}

export interface HotplugMemoryRequest {
  size_mb: number
}

export interface HotplugDiskRequest {
  path: string
  bus?: string
}

export interface HotplugNicRequest {
  bridge: string
  model?: string
}

// KubeVirt live-update reconciliation is asynchronous, so the backend
// polls briefly after accepting a CPU/memory hotplug patch and reports
// what actually happened -- accepting the patch is not the same as the
// guest running with it live. Callers must branch on these instead of
// assuming success just because the request didn't throw.
export interface HotplugApplyResult {
  hotplug_converged?: boolean
  restart_required?: boolean
}

export async function hotplugCpu(vmName: string, req: HotplugCpuRequest): Promise<HotplugApplyResult> {
  return apiPost<HotplugApplyResult>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/cpu`, req)
}

export async function hotplugMemory(vmName: string, req: HotplugMemoryRequest): Promise<HotplugApplyResult> {
  return apiPost<HotplugApplyResult>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/memory`, req)
}

export async function hotplugDisk(vmName: string, req: HotplugDiskRequest): Promise<unknown> {
  return apiPost<unknown>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/disk`, req)
}

export async function hotremoveDisk(vmName: string, deviceId: string): Promise<void> {
  const res = await apiFetch(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/disk/${encodeURIComponent(deviceId)}`, {
    method: 'DELETE',
  })
  if (!res.ok) throw new Error('Failed to hot-remove disk')
}

export async function hotplugNic(vmName: string, req: HotplugNicRequest): Promise<unknown> {
  return apiPost<unknown>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/nic`, req)
}

export async function hotremoveNic(vmName: string, deviceId: string): Promise<void> {
  const res = await apiFetch(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/hotplug/nic/${encodeURIComponent(deviceId)}`, {
    method: 'DELETE',
  })
  if (!res.ok) throw new Error('Failed to hot-remove NIC')
}
