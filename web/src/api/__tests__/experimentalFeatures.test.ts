// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

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

import { featureFetch } from '../experimental'
import { ExperimentalDisabledError } from '../../utils/experimental'
import { failoverDrPair, listDrPairs } from '../drPairs'
import { planEncryption } from '../volumeEncryption'
import { probeImageUploadDisabled, validateImage } from '../imageUpload'

function res(status: number, body: unknown): Response {
  const text = typeof body === 'string' ? body : JSON.stringify(body)
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: '',
    headers: new Headers({ 'content-type': 'application/json' }),
    text: () => Promise.resolve(text),
  } as unknown as Response
}

const disabled = (code: string) => res(403, { success: false, error: { code, message: 'turn it on' } })

beforeEach(() => {
  fetchMock.mockReset()
  storage.clear()
})

describe('featureFetch', () => {
  it('throws ExperimentalDisabledError on a 403 EXPERIMENTAL_DISABLED', async () => {
    fetchMock.mockResolvedValue(disabled('EXPERIMENTAL_DISABLED'))
    await expect(featureFetch('/api/x')).rejects.toBeInstanceOf(ExperimentalDisabledError)
  })
  it('throws a plain error for other failures', async () => {
    fetchMock.mockResolvedValue(res(409, { error: { code: 'PREFLIGHT_FAILED', message: 'blocked' } }))
    const err = await featureFetch('/api/x').catch((e) => e)
    expect(err).toBeInstanceOf(Error)
    expect(err).not.toBeInstanceOf(ExperimentalDisabledError)
    expect(err.message).toContain('blocked')
  })
})

describe('DR pairs api', () => {
  it('unwraps the data envelope', async () => {
    fetchMock.mockResolvedValue(res(200, { success: true, data: [{ name: 'p1' }] }))
    expect(await listDrPairs()).toEqual([{ name: 'p1' }])
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/enterprise/dr/pairs')
  })
  it('reports the feature_disabled answer as experimental-disabled', async () => {
    fetchMock.mockResolvedValue(disabled('feature_disabled'))
    await expect(listDrPairs()).rejects.toBeInstanceOf(ExperimentalDisabledError)
  })
  it('dry-run sends no confirm; a real run sends the phrase', async () => {
    fetchMock.mockResolvedValue(res(200, { success: true, data: { dry_run: true, plan: {} } }))
    await failoverDrPair('web pair', { dryRun: true })
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/enterprise/dr/pairs/web%20pair/failover')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ dry_run: true })
    await failoverDrPair('web', { dryRun: false, confirm: 'failover web' })
    expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({ dry_run: false, confirm: 'failover web' })
  })
})

describe('volume encryption api', () => {
  it('posts the plan request as JSON', async () => {
    fetchMock.mockResolvedValue(res(200, { ready: true, blockers: [], warnings: [], steps: [] }))
    const req = {
      namespace: 'vms', vm: 'db', volume: 'data', size: '20Gi', storage_class: 'enc',
      mode: 'storage_class' as const, acknowledge_unverified_encryption: false,
    }
    expect((await planEncryption(req)).ready).toBe(true)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/volume-encryption/plan')
    expect(fetchMock.mock.calls[0][1].method).toBe('POST')
  })
})

describe('image upload api', () => {
  it('probe returns the server message when the feature is disabled and null otherwise', async () => {
    fetchMock.mockResolvedValueOnce(disabled('EXPERIMENTAL_DISABLED'))
    expect(await probeImageUploadDisabled()).toBe('turn it on')
    fetchMock.mockResolvedValueOnce(res(200, { valid: false, error: 'empty' }))
    expect(await probeImageUploadDisabled()).toBeNull()
    fetchMock.mockRejectedValueOnce(new Error('offline'))
    expect(await probeImageUploadDisabled()).toBeNull()
  })
  it('validate sends only the head of the file', async () => {
    fetchMock.mockResolvedValue(res(200, { valid: true, format: 'qcow2' }))
    const slice = vi.fn(() => 'HEAD')
    const out = await validateImage({ slice } as unknown as Blob, 'qcow2')
    expect(out.valid).toBe(true)
    expect(slice).toHaveBeenCalledWith(0, 33 * 1024)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/images/validate?format=qcow2')
    expect(fetchMock.mock.calls[0][1].body).toBe('HEAD')
  })
})
