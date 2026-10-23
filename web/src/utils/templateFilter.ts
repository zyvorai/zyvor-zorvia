// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

export type Families = Record<string, string[]>

/** Filters the template catalog by a free-text query (matched against the
 * template name and its family, every word must match) and an optional
 * family tag. Families left with no templates are dropped. */
export function filterTemplates(families: Families, query: string, family: string | null): Families {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const out: Families = {}
  for (const [fam, names] of Object.entries(families)) {
    if (family && fam !== family) continue
    const hits = names.filter(name => {
      const hay = `${name} ${fam}`.toLowerCase()
      return words.every(w => hay.includes(w))
    })
    if (hits.length) out[fam] = hits
  }
  return out
}

export function countTemplates(families: Families): number {
  return Object.values(families).reduce((n, names) => n + names.length, 0)
}
