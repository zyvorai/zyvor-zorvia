// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import { unseenCount, latestId } from '../whatsNew'

const entries = [
  { id: 3, date: 'd', title: 'c', body: '' },
  { id: 2, date: 'd', title: 'b', body: '' },
  { id: 1, date: 'd', title: 'a', body: '' },
]

describe('whatsNew', () => {
  it('counts every entry as unseen on first visit', () => {
    expect(unseenCount(entries, 0)).toBe(3)
  })
  it('counts only newer entries', () => {
    expect(unseenCount(entries, 2)).toBe(1)
  })
  it('is zero once the latest id is seen', () => {
    expect(unseenCount(entries, latestId(entries))).toBe(0)
  })
  it('latestId of an empty list is 0', () => {
    expect(latestId([])).toBe(0)
  })
})
