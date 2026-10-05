// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { bootstrapKrytonGolden, createKrytonMachine, getKrytonGoldenPassport, listKrytonMachines, startKrytonGolden } from '../kryton'

const storage = new Map<string, string>()
const localStorageMock = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
}
Object.defineProperty(globalThis, 'localStorage', { value: localStorageMock, writable: true, configurable: true })

const fetchMock = vi.fn()
globalThis.fetch = fetchMock

function okJson(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

beforeEach(() => {
  storage.clear()
  fetchMock.mockReset()
  storage.set('zorvia_token', 'zorvia-jwt')
})

describe('Kryton API adapter', () => {
  it('scopes machine listing through the Zorvia adapter', async () => {
    fetchMock.mockResolvedValue(okJson({ items: [], nextCursor: 'next' }))

    const result = await listKrytonMachines('finance')

    expect(result.nextCursor).toBe('next')
    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/kryton/machines?project=finance')
    expect((init.headers as Headers).get('Authorization')).toBe('Bearer zorvia-jwt')
  })

  it('sends the Kryton machine contract without any upstream token', async () => {
    fetchMock.mockResolvedValue(okJson({
      id: 'vm-uuid', project: 'finance', provider: 'kubevirt', state: 'provisioning',
      spec: { name: 'win11-01', image: 'windows-11-enterprise', compute: { cpu: 4, memoryMiB: 8192 }, disk: { sizeGiB: 80 } },
      providerRef: { provider: 'kubevirt', name: 'win11-01' }, createdAt: '2026-09-09T00:00:00Z', updatedAt: '2026-09-09T00:00:00Z',
    }, 201))

    await createKrytonMachine({
      project: 'finance', name: 'win11-01', image: 'windows-11-enterprise',
      compute: { cpu: 4, memoryMiB: 8192 }, disk: { sizeGiB: 80 }, ttlMinutes: 60,
    })

    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/kryton/machines')
    expect(init.method).toBe('POST')
    expect(JSON.parse(init.body as string)).toMatchObject({
      project: 'finance', compute: { memoryMiB: 8192 }, disk: { sizeGiB: 80 }, ttlMinutes: 60,
    })
    expect((init.headers as Headers).get('Authorization')).toBe('Bearer zorvia-jwt')
    expect(init.body).not.toContain('KRYTON_TOKEN')
  })

  it('starts a Windows golden build in the requested project', async () => {
    fetchMock.mockResolvedValue(okJson({ id: 'gb-1', imageId: 'windows-11-pro', state: 'building' }, 202))

    const build = await startKrytonGolden({ imageId: 'windows-11-pro', auto: true }, 'finance')

    expect(build.state).toBe('building')
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/kryton/golden?project=finance')
    expect(init.method).toBe('POST')
    expect(JSON.parse(init.body as string)).toEqual({ imageId: 'windows-11-pro', auto: true })
  })

  it('encodes build ids for bootstrap and passport', async () => {
    fetchMock.mockResolvedValue(okJson({ id: 'gb 1', imageId: 'windows-11-pro', state: 'ready' }, 202))
    await bootstrapKrytonGolden('gb 1', 'finance')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/kryton/golden/gb%201/bootstrap?project=finance')
    expect(fetchMock.mock.calls[0][1].method).toBe('POST')

    fetchMock.mockResolvedValue(okJson({ score: 90 }))
    await getKrytonGoldenPassport('gb 1')
    expect(fetchMock.mock.calls[1][0]).toBe('/api/v1/kryton/golden/gb%201/passport')
  })
})
