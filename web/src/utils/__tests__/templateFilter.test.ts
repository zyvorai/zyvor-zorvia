// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import { filterTemplates, countTemplates } from '../templateFilter'

const catalog = {
  Linux: ['ubuntu-24.04', 'debian-12', 'rhel-9'],
  Windows: ['windows-11', 'windows-server-2022'],
}

describe('filterTemplates', () => {
  it('returns everything for an empty query', () => {
    expect(countTemplates(filterTemplates(catalog, '', null))).toBe(5)
  })
  it('matches names case-insensitively and drops empty families', () => {
    expect(filterTemplates(catalog, 'UBUNTU', null)).toEqual({ Linux: ['ubuntu-24.04'] })
  })
  it('requires every word to match, including the family name', () => {
    expect(filterTemplates(catalog, 'windows server', null)).toEqual({ Windows: ['windows-server-2022'] })
    expect(filterTemplates(catalog, 'linux 12', null)).toEqual({ Linux: ['debian-12'] })
  })
  it('restricts to a family tag', () => {
    expect(Object.keys(filterTemplates(catalog, '', 'Windows'))).toEqual(['Windows'])
  })
  it('returns nothing when no template matches', () => {
    expect(filterTemplates(catalog, 'solaris', null)).toEqual({})
  })
})
