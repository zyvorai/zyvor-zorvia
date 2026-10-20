// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { hotremoveDisk } from '../hotplug'
import { deleteUser } from '../users'
import { getOperation } from '../operations'

const storage = new Map<string, string>()
Object.defineProperty(globalThis, 'localStorage', {
  value: {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => storage.set(k, v),
    removeItem: (k: string) => storage.delete(k),
  },
  writable: true,
  configurable: true,
})

const fetchMock = vi.fn()
globalThis.fetch = fetchMock

beforeEach(() => {
  fetchMock.mockReset()
  fetchMock.mockResolvedValue(
    new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } }),
  )
})

describe('API path segments are URL-encoded', () => {
  it('encodes deviceId and vmName in hot-remove', async () => {
    await hotremoveDisk('vm1', '../../users/x?y')
    expect(fetchMock.mock.calls[0][0]).toContain('/vms/vm1/hotplug/disk/..%2F..%2Fusers%2Fx%3Fy')
  })

  it('encodes user ids', async () => {
    await deleteUser('a/b')
    expect(fetchMock.mock.calls[0][0]).toContain('/users/a%2Fb')
  })

  it('encodes operation ids', async () => {
    await getOperation('a/../b')
    expect(fetchMock.mock.calls[0][0]).toContain('/operations/a%2F..%2Fb')
  })
})
