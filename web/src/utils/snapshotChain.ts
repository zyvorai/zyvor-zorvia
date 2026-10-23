// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import type { VMSnapshot } from '../api/snapshots'

export interface SnapshotNode {
  snapshot: VMSnapshot
  depth: number
  children: SnapshotNode[]
}

/**
 * Arrange snapshots into a parent → child forest (oldest first). Snapshots whose
 * parent is missing from the list (or that point at themselves) become roots, so
 * partial lists never drop entries or loop.
 */
export function buildSnapshotChain(snapshots: VMSnapshot[]): SnapshotNode[] {
  const byTime = [...snapshots].sort(
    (a, b) => new Date(a.created).getTime() - new Date(b.created).getTime(),
  )
  const ids = new Set(byTime.map((s) => s.id))
  const childrenOf = new Map<string, VMSnapshot[]>()
  const roots: VMSnapshot[] = []
  for (const snap of byTime) {
    if (snap.parent_id && snap.parent_id !== snap.id && ids.has(snap.parent_id)) {
      const list = childrenOf.get(snap.parent_id) ?? []
      list.push(snap)
      childrenOf.set(snap.parent_id, list)
    } else {
      roots.push(snap)
    }
  }
  const seen = new Set<string>()
  const build = (snap: VMSnapshot, depth: number): SnapshotNode => {
    seen.add(snap.id)
    const kids = (childrenOf.get(snap.id) ?? []).filter((c) => !seen.has(c.id))
    return { snapshot: snap, depth, children: kids.map((c) => build(c, depth + 1)) }
  }
  const forest = roots.map((r) => build(r, 0))
  // Cycles (a → b → a) leave members unreachable from any root; surface them as roots.
  for (const snap of byTime) {
    if (!seen.has(snap.id)) forest.push(build(snap, 0))
  }
  return forest
}
