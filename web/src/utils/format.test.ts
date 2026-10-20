// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from 'vitest'
import { formatBytes } from './format'

describe('formatBytes', () => {
  it('formats normal values', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1536)).toBe('1.5 KB')
  })

  it('never returns undefined for fractional or negative input', () => {
    expect(formatBytes(0.5)).toBe('0.5 B')
    expect(formatBytes(-10)).toBe('0 B')
    expect(formatBytes(NaN)).toBe('0 B')
  })
})
