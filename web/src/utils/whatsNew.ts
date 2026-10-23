// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import type { WhatsNewEntry } from '../config/whatsNew'

export const WHATS_NEW_KEY = 'zorvia-whats-new-seen'

export function unseenCount(entries: WhatsNewEntry[], lastSeen: number): number {
  return entries.filter((e) => e.id > lastSeen).length
}

export function latestId(entries: WhatsNewEntry[]): number {
  return entries.reduce((m, e) => Math.max(m, e.id), 0)
}

export function readLastSeen(): number {
  try {
    const n = Number(localStorage.getItem(WHATS_NEW_KEY))
    return Number.isFinite(n) ? n : 0
  } catch {
    return 0
  }
}

export function writeLastSeen(id: number): void {
  try {
    localStorage.setItem(WHATS_NEW_KEY, String(id))
  } catch {
    // storage unavailable; the dot simply reappears next visit
  }
}
