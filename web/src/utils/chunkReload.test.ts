// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { isChunkLoadError, reloadOnceForChunkError } from './chunkReload'

describe('isChunkLoadError', () => {
  it('matches the Vite dynamic-import failure message', () => {
    expect(isChunkLoadError(new Error('Failed to fetch dynamically imported module: /assets/VMList-abc.js'))).toBe(
      true,
    )
  })

  it('matches the Firefox-style module script failure message', () => {
    expect(isChunkLoadError(new Error('error loading dynamically imported module: https://x/y.js'))).toBe(true)
  })

  it('does not match unrelated errors', () => {
    expect(isChunkLoadError(new Error('vm start failed'))).toBe(false)
  })

  it('handles non-Error values', () => {
    expect(isChunkLoadError('failed to fetch dynamically imported module')).toBe(true)
    expect(isChunkLoadError(undefined)).toBe(false)
  })
})

describe('reloadOnceForChunkError', () => {
  const storage = new Map<string, string>()
  const sessionStorageMock = {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
    removeItem: vi.fn((key: string) => storage.delete(key)),
  }
  const reload = vi.fn()

  beforeEach(() => {
    storage.clear()
    reload.mockClear()
    Object.defineProperty(globalThis, 'window', {
      value: { ...globalThis.window, sessionStorage: sessionStorageMock, location: { reload } },
      writable: true,
    })
  })

  it('reloads and returns true on first call', () => {
    expect(reloadOnceForChunkError()).toBe(true)
    expect(reload).toHaveBeenCalledTimes(1)
  })

  it('does not reload again in the same session', () => {
    reloadOnceForChunkError()
    expect(reloadOnceForChunkError()).toBe(false)
    expect(reload).toHaveBeenCalledTimes(1)
  })
})
