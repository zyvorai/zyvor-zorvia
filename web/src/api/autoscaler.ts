// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { featureJson } from './experimental'

/** Autoscaler loop status and recent decisions (Experimental, read-only;
    ZORVIA_EXPERIMENTAL=1, cluster.admin). See docs/AUTOSCALER_LOOP.md.
    Responses are wrapped as `{ success, data }`. */
const BASE = '/api/v1/autoscaler'

export type DecisionKind = 'scale_up' | 'scale_down' | 'hold' | 'skipped'
export type DecisionOutcome = 'applied' | 'recommended' | 'failed' | 'held' | 'skipped'

export interface AutoscalerTarget {
  id: string
  kind: 'vm_pool' | 'vm_cpu'
  namespace: string
  name: string
  load_metric: 'cpu' | 'memory'
  min: number
  max: number
  step: number
  scale_up_threshold: number
  scale_down_threshold: number
  required_samples: number
  cooldown_secs: number
  up_streak: number
  down_streak: number
  last_action: string | null
}

export interface AutoscalerStatus {
  experimental: boolean
  /** False when the loop is not running in the API process. */
  enabled: boolean
  disabled_reason: string | null
  status: {
    running: boolean
    mode: 'apply' | 'recommend_only' | null
    interval_secs: number | null
    leader: boolean | null
    last_tick: string | null
  }
  targets: AutoscalerTarget[]
}

export interface DecisionRecord {
  seq: number
  timestamp: string
  target: string
  metric: 'cpu' | 'memory'
  load_percent: number | null
  source: string
  decision: DecisionKind
  outcome: DecisionOutcome
  from: number | null
  to: number | null
  reason: string
  /** Identical consecutive hold/skip ticks folded into this record. */
  count: number
}

export interface AutoscalerDecisions {
  experimental: boolean
  enabled: boolean
  disabled_reason: string | null
  decisions: DecisionRecord[]
}

interface Envelope<T> {
  success: boolean
  data: T
}

export async function getAutoscalerStatus(): Promise<AutoscalerStatus> {
  return (await featureJson<Envelope<AutoscalerStatus>>(`${BASE}/status`)).data
}

export async function getAutoscalerDecisions(opts: { limit?: number; target?: string } = {}): Promise<AutoscalerDecisions> {
  const q = new URLSearchParams()
  if (opts.limit !== undefined) q.set('limit', String(opts.limit))
  if (opts.target) q.set('target', opts.target)
  const qs = q.toString()
  return (await featureJson<Envelope<AutoscalerDecisions>>(`${BASE}/decisions${qs ? `?${qs}` : ''}`)).data
}
