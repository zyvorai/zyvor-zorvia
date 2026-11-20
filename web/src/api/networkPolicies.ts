// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost, apiDelete } from './client'

export interface PolicyRule {
  cidr?: string | null
  protocol?: string | null
  port?: string | null
}

/** Mirrors the backend's real k8s NetworkPolicy projection
 * (src/api/http_server/web/network_policy_handlers.rs) -- `vm_name` is set
 * only when the policy's podSelector matches KubeVirt's `kubevirt.io/domain`
 * label for a single VM; null means it targets every pod in the namespace. */
export interface NetworkPolicy {
  name: string
  vm_name: string | null
  ingress: PolicyRule[]
  egress: PolicyRule[]
  created: string | null
}

export interface CreateNetworkPolicyRequest {
  name: string
  vm_name?: string
  ingress?: { cidr?: string; protocol?: string; port?: number }[]
  egress?: { cidr?: string; protocol?: string; port?: number }[]
}

const API_BASE = '/api'

export async function listNetworkPolicies(): Promise<NetworkPolicy[]> {
  return apiGet<NetworkPolicy[]>(`${API_BASE}/network-policies`)
}

export async function createNetworkPolicy(req: CreateNetworkPolicyRequest): Promise<NetworkPolicy> {
  return apiPost<NetworkPolicy>(`${API_BASE}/network-policies`, req)
}

export async function deleteNetworkPolicy(name: string): Promise<void> {
  return apiDelete(`${API_BASE}/network-policies/${encodeURIComponent(name)}`)
}
