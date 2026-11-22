// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiDelete, apiGet, apiPost, getToken } from './client'

export interface PodContainer {
  name: string
  image: string
  init: boolean
  ready: boolean
  restarts: number
  state: string
}

export interface PodSummary {
  name: string
  namespace: string
  phase: string
  status: string
  ready: string
  restarts: number
  created: string | null
  age_seconds: number | null
  node: string | null
  pod_ip: string | null
  owner_kind: string | null
  containers: PodContainer[]
}

export async function listPods(namespace?: string): Promise<PodSummary[]> {
  const qs = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  const res = await apiGet<{ pods: PodSummary[] }>(`/api/v1/pods${qs}`)
  return res.pods ?? []
}

export async function listNamespaces(): Promise<string[]> {
  const res = await apiGet<{ namespaces: string[] }>('/api/v1/namespaces')
  return res.namespaces ?? []
}

/** Which pod RBAC rules the Zorvia service account actually has. */
export interface PodCapabilities {
  logs: boolean
  exec: boolean
  delete: boolean
}

export const ALL_POD_CAPABILITIES: PodCapabilities = { logs: true, exec: true, delete: true }

export function podCapabilities(): Promise<PodCapabilities> {
  return apiGet<PodCapabilities>('/api/v1/pods/capabilities')
}

export interface PodEvent {
  type: string
  reason: string
  message: string
  count: number
  source: string
  first_seen: string | null
  last_seen: string | null
}

const podPath = (namespace: string, pod: string) =>
  `/api/v1/pods/${encodeURIComponent(namespace)}/${encodeURIComponent(pod)}`

export function deletePod(namespace: string, pod: string, opts: { force?: boolean } = {}): Promise<void> {
  return apiDelete(`${podPath(namespace, pod)}${opts.force ? '?grace=0' : ''}`)
}

export function restartPod(
  namespace: string,
  pod: string,
): Promise<{ restarted: boolean; owner_kind: string; owner: string }> {
  return apiPost(`${podPath(namespace, pod)}/restart`)
}

export async function podEvents(namespace: string, pod: string): Promise<PodEvent[]> {
  const res = await apiGet<{ events: PodEvent[] }>(`${podPath(namespace, pod)}/events`)
  return res.events ?? []
}

export async function podYaml(namespace: string, pod: string): Promise<string> {
  const res = await apiGet<{ yaml: string }>(`${podPath(namespace, pod)}/yaml`)
  return res.yaml ?? ''
}

/** SPA route for a full-window log view (opened in a new tab). */
export function podLogsPagePath(namespace: string, pod: string, container?: string): string {
  const base = `/app/pods/${encodeURIComponent(namespace)}/${encodeURIComponent(pod)}/logs`
  return container ? `${base}?container=${encodeURIComponent(container)}` : base
}

/** Absolute ws(s):// URL for a `/ws/...` path, with the session token attached. */
export function wsUrl(path: string, params: Record<string, string | number | boolean | undefined> = {}): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const qs = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== '') qs.set(k, String(v))
  }
  const token = getToken()
  if (token) qs.set('token', token)
  return `${proto}//${window.location.host}${path}?${qs.toString()}`
}

export function podLogsUrl(
  namespace: string,
  pod: string,
  opts: { container?: string; tail?: number; timestamps?: boolean; previous?: boolean },
): string {
  return wsUrl(`/ws/pods/${encodeURIComponent(namespace)}/${encodeURIComponent(pod)}/logs`, {
    container: opts.container,
    tail: opts.tail,
    timestamps: opts.timestamps ? 1 : 0,
    previous: opts.previous ? 1 : 0,
  })
}

export function podExecUrl(namespace: string, pod: string, opts: { container?: string; shell?: string }): string {
  return wsUrl(`/ws/pods/${encodeURIComponent(namespace)}/${encodeURIComponent(pod)}/exec`, {
    container: opts.container,
    shell: opts.shell,
  })
}
