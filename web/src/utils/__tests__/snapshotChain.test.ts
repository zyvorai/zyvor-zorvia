// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import type { VMSnapshot } from '../../api/snapshots'
import { buildSnapshotChain } from '../snapshotChain'

const snap = (id: string, parent: string | null, created: string): VMSnapshot => ({
  id,
  vm_name: 'vm',
  name: id,
  description: null,
  snapshot_type: 'Disk',
  parent_id: parent,
  size_bytes: 0,
  created,
})

describe('buildSnapshotChain', () => {
  it('nests children under their parent, oldest first', () => {
    const forest = buildSnapshotChain([
      snap('c', 'b', '2026-01-03T00:00:00Z'),
      snap('a', null, '2026-01-01T00:00:00Z'),
      snap('b', 'a', '2026-01-02T00:00:00Z'),
    ])
    expect(forest).toHaveLength(1)
    expect(forest[0].snapshot.id).toBe('a')
    expect(forest[0].children[0].snapshot.id).toBe('b')
    expect(forest[0].children[0].children[0].snapshot.id).toBe('c')
    expect(forest[0].children[0].children[0].depth).toBe(2)
  })

  it('treats snapshots with a missing parent as roots', () => {
    const forest = buildSnapshotChain([snap('x', 'gone', '2026-01-01T00:00:00Z')])
    expect(forest.map((n) => n.snapshot.id)).toEqual(['x'])
  })

  it('branches when two snapshots share a parent', () => {
    const forest = buildSnapshotChain([
      snap('a', null, '2026-01-01T00:00:00Z'),
      snap('b', 'a', '2026-01-02T00:00:00Z'),
      snap('c', 'a', '2026-01-03T00:00:00Z'),
    ])
    expect(forest[0].children.map((n) => n.snapshot.id)).toEqual(['b', 'c'])
  })

  it('never drops or loops on cycles and self-parents', () => {
    const forest = buildSnapshotChain([
      snap('a', 'b', '2026-01-01T00:00:00Z'),
      snap('b', 'a', '2026-01-02T00:00:00Z'),
      snap('s', 's', '2026-01-03T00:00:00Z'),
    ])
    const ids: string[] = []
    const walk = (n: (typeof forest)[number]) => {
      ids.push(n.snapshot.id)
      n.children.forEach(walk)
    }
    forest.forEach(walk)
    expect(ids.sort()).toEqual(['a', 'b', 's'])
  })
})
