// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet, apiPost, apiDelete } from './client'
import { API_BASE_URL } from './config'

const BASE = `${API_BASE_URL}/storage/rook`

export type CephHealthState = 'NotProvisioned' | 'Provisioning' | 'Healthy' | 'Warning' | 'Error' | 'Unknown'

export interface CephHealthSummary {
  state: CephHealthState
  phase?: string
  ceph_health?: string
  message?: string
}

export interface BootstrapReport {
  applied: string[]
  skipped: string[]
  failed: [string, string][]
}

export async function bootstrapRook(namespace?: string, version?: string): Promise<BootstrapReport> {
  return apiPost<BootstrapReport>(`${BASE}/bootstrap`, { namespace, version })
}

export async function getClusterStatus(namespace?: string): Promise<CephHealthSummary> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  return apiGet<CephHealthSummary>(`${BASE}/cluster${qs}`)
}

export interface CreateClusterRequest {
  namespace: string
  mon_count?: number
  data_dir_host_path?: string
  use_all_nodes?: boolean
  use_all_devices?: boolean
  device_filter?: string
}

export async function createCluster(req: CreateClusterRequest): Promise<unknown> {
  return apiPost(`${BASE}/cluster`, req)
}

export async function deleteCluster(namespace?: string): Promise<void> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  await apiDelete(`${BASE}/cluster${qs}`)
}

export interface RookPool {
  metadata: { name: string; namespace: string }
  spec: {
    failureDomain?: string
    replicated?: { size: number }
    erasureCoded?: { dataChunks: number; codingChunks: number }
    deviceClass?: string
  }
  status?: { phase?: string }
}

export async function listPools(namespace?: string): Promise<RookPool[]> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  return apiGet<RookPool[]>(`${BASE}/pools${qs}`)
}

export interface CreatePoolRequest {
  name: string
  namespace: string
  failure_domain?: string
  replicated_size?: number
  erasure_coded?: [number, number]
  device_class?: string
}

export async function createPool(req: CreatePoolRequest): Promise<unknown> {
  return apiPost(`${BASE}/pools`, req)
}

export async function deletePool(name: string, namespace?: string): Promise<void> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  await apiDelete(`${BASE}/pools/${encodeURIComponent(name)}${qs}`)
}

export interface RookFilesystem {
  metadata: { name: string; namespace: string }
  spec: unknown
  status?: { phase?: string }
}

export async function listFilesystems(namespace?: string): Promise<RookFilesystem[]> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  return apiGet<RookFilesystem[]>(`${BASE}/filesystems${qs}`)
}

export interface CreateFilesystemRequest {
  name: string
  namespace: string
  failure_domain?: string
  metadata_pool_replicated_size?: number
  data_pool_replicated_size?: number
  active_mds_count?: number
}

export async function createFilesystem(req: CreateFilesystemRequest): Promise<unknown> {
  return apiPost(`${BASE}/filesystems`, req)
}

export async function deleteFilesystem(name: string, namespace?: string): Promise<void> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  await apiDelete(`${BASE}/filesystems/${encodeURIComponent(name)}${qs}`)
}

export interface RookObjectStore {
  metadata: { name: string; namespace: string }
  spec: unknown
  status?: { phase?: string }
}

export async function listObjectStores(namespace?: string): Promise<RookObjectStore[]> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  return apiGet<RookObjectStore[]>(`${BASE}/objectstores${qs}`)
}

export interface CreateObjectStoreRequest {
  name: string
  namespace: string
  failure_domain?: string
  metadata_pool_replicated_size?: number
  data_pool_replicated_size?: number
  gateway_port?: number
  gateway_instances?: number
}

export async function createObjectStore(req: CreateObjectStoreRequest): Promise<unknown> {
  return apiPost(`${BASE}/objectstores`, req)
}

export async function deleteObjectStore(name: string, namespace?: string): Promise<void> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  await apiDelete(`${BASE}/objectstores/${encodeURIComponent(name)}${qs}`)
}

export type CreateStorageClassRequest =
  | { type: 'rbd'; name: string; namespace: string; pool: string; reclaim_policy?: string }
  | { type: 'cephfs'; name: string; namespace: string; filesystem: string; reclaim_policy?: string }

export async function createStorageClass(req: CreateStorageClassRequest): Promise<unknown> {
  return apiPost(`${BASE}/storage-classes`, req)
}

export async function createVolumeSnapshotClass(name: string, namespace: string): Promise<unknown> {
  return apiPost(`${BASE}/volume-snapshot-classes`, { name, namespace })
}
