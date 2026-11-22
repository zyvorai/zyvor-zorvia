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

import { getCertificateInventory, listCertManagerCertificates, renewCertManagerCertificate } from '../certificates'
import { ExperimentalDisabledError } from '../../utils/experimental'

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

describe('certificates api', () => {
  it('requests the inventory, optionally scoped to a namespace', async () => {
    fetchMock.mockResolvedValue(res(200, { certificates: [], summary: {}, skipped: [] }))
    await getCertificateInventory()
    expect(fetchMock.mock.calls[0][0]).toBe('/api/certificates/inventory')
    await getCertificateInventory('team a/b')
    expect(fetchMock.mock.calls[1][0]).toBe('/api/certificates/inventory?namespace=team%20a%2Fb')
  })
  it('posts the renew confirmation phrase', async () => {
    fetchMock.mockResolvedValue(res(202, { renewing: true, certificate: {} }))
    await renewCertManagerCertificate('web', 'site', 'renew web/site')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/certificates/cert-manager/renew')
    expect(fetchMock.mock.calls[0][1].method).toBe('POST')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ namespace: 'web', name: 'site', confirm: 'renew web/site' })
  })
  it('reports the disabled feature', async () => {
    fetchMock.mockResolvedValue(
      res(403, { success: false, error: { code: 'EXPERIMENTAL_DISABLED', message: 'cert-lifecycle is Experimental' } }),
    )
    await expect(listCertManagerCertificates()).rejects.toBeInstanceOf(ExperimentalDisabledError)
  })
})
