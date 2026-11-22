// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet } from './client'
import { API_BASE_URL } from './config'

export interface GuestHook {
  name: string
  success: boolean
  message?: string
}

export interface GuestQuiesce {
  agent: string
  hooks_ok: boolean
  hooks: GuestHook[]
  quiesce_supported: boolean
  /** What a snapshot taken now can honestly be called. */
  consistency: 'application' | 'filesystem' | 'crash'
}

export interface GuestAgentInfo {
  vm_name: string
  agent: string
  version: { version: string; protocol: string }
  health: Record<string, unknown> | null
  snapshot_readiness: GuestQuiesce | null
}

export type GuestInventoryKind = 'packages' | 'users' | 'certificates' | 'containers' | 'security'

export const GUEST_INVENTORY_KINDS: { id: GuestInventoryKind; label: string }[] = [
  { id: 'packages', label: 'Packages' },
  { id: 'users', label: 'Users' },
  { id: 'certificates', label: 'Certificates' },
  { id: 'containers', label: 'Containers' },
  { id: 'security', label: 'Security posture' },
]

export interface GuestInventory {
  vm_name: string
  kind: GuestInventoryKind
  inventory: Record<string, unknown>
}

/** Needs the Zyvor guest agent (`guest_agent: "zyvor"`); 409 GUEST_AGENT_NOT_CONNECTED otherwise. */
export function getGuestAgent(vmName: string): Promise<GuestAgentInfo> {
  return apiGet<GuestAgentInfo>(`${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/guest/agent`)
}

export function getGuestInventory(vmName: string, kind: GuestInventoryKind): Promise<GuestInventory> {
  return apiGet<GuestInventory>(
    `${API_BASE_URL}/vms/${encodeURIComponent(vmName)}/guest/inventory/${kind}`,
  )
}
