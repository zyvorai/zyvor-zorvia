// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { apiDelete, apiGet, apiPost } from './client'

export interface KrytonStatus {
  enabled: boolean
  connected: boolean
  project?: string | null
  error?: string
}

export interface KrytonList<T> {
  items: T[]
  nextCursor?: string
}

export interface KrytonCompute { cpu: number; memoryMiB: number }
export interface KrytonDisk { sizeGiB: number; storageClass?: string; volumeMode?: string }
export interface KrytonNetwork { networkId?: string }

export interface KrytonMachine {
  id: string
  project: string
  provider: string
  state: string
  spec: {
    name: string
    image: string
    compute: KrytonCompute
    disk: KrytonDisk
    network?: KrytonNetwork
    ttlMinutes?: number
  }
  providerRef: { provider: string; namespace?: string; name: string }
  ipAddresses?: string[]
  rdpHost?: string
  rdpPort?: number
  rdpUsername?: string
  progressPercent?: number
  message?: string
  createdAt: string
  updatedAt: string
  expiresAt?: string
}

export interface KrytonImage {
  id: string
  name: string
  version: string
  family: string
  description: string
  minCpu: number
  minMemoryMiB: number
  defaultDiskGiB: number
  availability?: string
  storageSource?: string
  ready: boolean
  certified?: boolean
  validationScore?: number
  /** `linux` for cloud-image templates; empty for Windows images. */
  os?: string
}

export interface KrytonCapabilities {
  provider: string
  snapshots: boolean
  networks: boolean
  ttl: boolean
  liveMigration: boolean
  console: boolean
  goldenImages: boolean
}

/** A Windows golden-image build on Kryton: dockur install, Sysprep, qcow2
    capture, then an optional CDI bootstrap into a DataSource. */
export interface KrytonGoldenBuild {
  id: string
  version?: string
  imageId: string
  state: string
  phase?: string
  progressPercent?: number
  message?: string
  consoleUrl?: string
  outputPath?: string
  sha256?: string
  startedAt?: string | null
  updatedAt?: string | null
  error?: string
  bootstrapState?: string
  bootstrapMessage?: string
  dataSource?: string
  certified?: boolean
  validationScore?: number
}

export interface StartKrytonGolden {
  imageId: string
  version?: string
  auto: boolean
}

export interface KrytonSummary {
  project: string
  provider: string
  machines: number
  running: number
  stopped: number
  attention: number
  cpu: number
  memoryMiB: number
}

export interface CreateKrytonMachine {
  project: string
  name: string
  image: string
  compute: KrytonCompute
  disk: KrytonDisk
  network?: KrytonNetwork
  ttlMinutes?: number
}

function query(project?: string): string {
  return project ? `?project=${encodeURIComponent(project)}` : ''
}

export const getKrytonStatus = () => apiGet<KrytonStatus>('/api/v1/kryton/status')
export const getKrytonImages = () => apiGet<KrytonList<KrytonImage>>('/api/v1/kryton/images')
export const getKrytonSummary = (project?: string) => apiGet<KrytonSummary>(`/api/v1/kryton/summary${query(project)}`)
export const listKrytonMachines = (project?: string) => apiGet<KrytonList<KrytonMachine>>(`/api/v1/kryton/machines${query(project)}`)
export const createKrytonMachine = (body: CreateKrytonMachine) => apiPost<KrytonMachine>('/api/v1/kryton/machines', body)
export const startKrytonMachine = (id: string, project?: string) => apiPost<KrytonMachine>(`/api/v1/kryton/machines/${encodeURIComponent(id)}/start${query(project)}`)
export const stopKrytonMachine = (id: string, project?: string) => apiPost<KrytonMachine>(`/api/v1/kryton/machines/${encodeURIComponent(id)}/stop${query(project)}`)
export const snapshotKrytonMachine = (id: string, project?: string, name?: string) => apiPost(`/api/v1/kryton/machines/${encodeURIComponent(id)}/snapshot${query(project)}`, name ? { name } : {})
export const deleteKrytonMachine = (id: string, project?: string) => apiDelete(`/api/v1/kryton/machines/${encodeURIComponent(id)}${query(project)}`)
export const getKrytonCapabilities = () => apiGet<KrytonCapabilities>('/api/v1/kryton/capabilities')
export const listKrytonGolden = () => apiGet<KrytonList<KrytonGoldenBuild>>('/api/v1/kryton/golden')
export const getKrytonGolden = (id: string) => apiGet<KrytonGoldenBuild>(`/api/v1/kryton/golden/${encodeURIComponent(id)}`)
export const startKrytonGolden = (body: StartKrytonGolden, project?: string) => apiPost<KrytonGoldenBuild>(`/api/v1/kryton/golden${query(project)}`, body)
export const bootstrapKrytonGolden = (id: string, project?: string) => apiPost<KrytonGoldenBuild>(`/api/v1/kryton/golden/${encodeURIComponent(id)}/bootstrap${query(project)}`)
export const getKrytonGoldenPassport = (id: string) => apiGet<Record<string, unknown>>(`/api/v1/kryton/golden/${encodeURIComponent(id)}/passport`)
