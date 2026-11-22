// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, it, expect, vi, beforeEach } from 'vitest'

const storage = new Map<string, string>()
Object.defineProperty(globalThis, 'localStorage', {
  value: {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => storage.set(k, v),
    removeItem: (k: string) => storage.delete(k),
  },
  writable: true,
})
const fetchMock = vi.fn()
globalThis.fetch = fetchMock

import { getAutoscalerDecisions, getAutoscalerStatus } from '../autoscaler'
import { ExperimentalDisabledError } from '../../utils/experimental'
import { describeDecision, formatLoad, outcomeBadge } from '../../utils/autoscaler'
import type { DecisionRecord } from '../autoscaler'

function res(status: number, body: unknown): Response {
  const text = JSON.stringify(body)
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: '',
    headers: new Headers({ 'content-type': 'application/json' }),
    text: () => Promise.resolve(text),
  } as unknown as Response
}

beforeEach(() => {
  fetchMock.mockReset()
  storage.clear()
})

describe('autoscaler api', () => {
  it('unwraps status', async () => {
    fetchMock.mockResolvedValue(res(200, { success: true, data: { enabled: true, targets: [] } }))
    const s = await getAutoscalerStatus()
    expect(s.enabled).toBe(true)
    expect(String(fetchMock.mock.calls[0][0])).toContain('/api/v1/autoscaler/status')
  })
  it('builds the decisions query', async () => {
    fetchMock.mockResolvedValue(res(200, { success: true, data: { decisions: [] } }))
    await getAutoscalerDecisions({ limit: 20, target: 'web' })
    expect(String(fetchMock.mock.calls[0][0])).toContain('/api/v1/autoscaler/decisions?limit=20&target=web')
    await getAutoscalerDecisions()
    expect(String(fetchMock.mock.calls[1][0]).endsWith('/api/v1/autoscaler/decisions')).toBe(true)
  })
  it('maps a 403 EXPERIMENTAL_DISABLED to ExperimentalDisabledError', async () => {
    fetchMock.mockResolvedValue(
      res(403, { success: false, error: { code: 'EXPERIMENTAL_DISABLED', message: 'turn it on' } }),
    )
    await expect(getAutoscalerStatus()).rejects.toBeInstanceOf(ExperimentalDisabledError)
  })
})

describe('autoscaler utils', () => {
  const base: DecisionRecord = {
    seq: 1, timestamp: '2026-01-01T00:00:00Z', target: 'web', metric: 'cpu', load_percent: 91.25,
    source: 'metrics_api', decision: 'scale_up', outcome: 'recommended', from: 2, to: 3, reason: 'r', count: 1,
  }
  it('describes decisions', () => {
    expect(describeDecision(base)).toBe('scale up 2 to 3')
    expect(describeDecision({ ...base, decision: 'scale_down', from: 3, to: 2 })).toBe('scale down 3 to 2')
    expect(describeDecision({ ...base, decision: 'hold', to: null })).toBe('hold at 2')
    expect(describeDecision({ ...base, decision: 'skipped', from: null, to: null })).toBe('skipped')
  })
  it('formats load and badges', () => {
    expect(formatLoad(91.25)).toBe('91.3%')
    expect(formatLoad(null)).toBe('—')
    expect(outcomeBadge('failed').status).toBe('failed')
    expect(outcomeBadge('applied').label).toBe('Applied')
  })
})
