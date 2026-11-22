// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet, apiPost, apiDelete } from './client'

/** Mirrors the backend's QuotaSummary (src/api/http_server/web/quota_handlers.rs),
 * itself a projection of the real Kubernetes ResourceQuota object -- `hard`
 * and `used` are whatever resource keys the quota actually sets
 * (e.g. "requests.cpu", "requests.memory", "pods"), not a fixed shape. */
export interface ResourceQuota {
  name: string
  namespace: string
  hard: Record<string, string>
  used: Record<string, string>
  created?: string | null
}

const API_BASE = '/api/v1'

export async function listQuotas(namespace?: string): Promise<ResourceQuota[]> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  return apiGet<ResourceQuota[]>(`${API_BASE}/quotas${qs}`)
}

export async function createQuota(
  name: string,
  hard: Record<string, string>,
  namespace?: string,
): Promise<ResourceQuota> {
  return apiPost<ResourceQuota>(`${API_BASE}/quotas`, { name, namespace, hard })
}

export async function deleteQuota(namespace: string, name: string): Promise<void> {
  return apiDelete(`${API_BASE}/quotas/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`)
}
