// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

export interface WhatsNewEntry {
  /** Monotonic id; a higher id is newer. */
  id: number
  date: string
  title: string
  body: string
  to?: string
}

/** Newest first. Add an entry here when a user-visible feature ships. */
export const WHATS_NEW: WhatsNewEntry[] = [
  {
    id: 3,
    date: '2026-10-23',
    title: 'Zorvia for macOS',
    body: 'A signed, notarized desktop app with the server bundled. Download it from the releases page.',
  },
  {
    id: 2,
    date: '2026-10-22',
    title: 'Template search and filters',
    body: 'Search the template catalog and filter by OS family, with a live match count.',
    to: '/app/templates',
  },
  {
    id: 1,
    date: '2026-10-17',
    title: 'Leaving Nutanix, documented',
    body: 'Export, convert and land a Nutanix VM on KubeVirt, with an honest status of what works.',
  },
]
