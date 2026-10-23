// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

export interface OnboardingStep {
  id: string
  title: string
  description: string
  to: string
  cta: string
}

export const ONBOARDING_STEPS: OnboardingStep[] = [
  { id: 'vm', title: 'Create your first VM', description: 'Linux with cloud-init or Windows with VNC/RDP.', to: '/app/create', cta: 'Create VM' },
  { id: 'template', title: 'Browse templates', description: 'Search the catalog and filter by OS family.', to: '/app/templates', cta: 'Open templates' },
  { id: 'snapshot', title: 'Take a snapshot', description: 'Capture a point-in-time copy of a VM.', to: '/app/snapshots', cta: 'Open snapshots' },
  { id: 'backup', title: 'Schedule backups', description: 'Add a recurring backup policy.', to: '/app/backup-scheduler', cta: 'Open scheduler' },
  { id: 'alert', title: 'Add an alert rule', description: 'Get told when something needs attention.', to: '/app/alerts', cta: 'Open alerts' },
]

export const ONBOARDING_DONE_KEY = 'zorvia-onboarding-done'
export const ONBOARDING_DISMISSED_KEY = 'zorvia-onboarding-dismissed'

export interface OnboardingProgress {
  done: number
  total: number
  pct: number
  complete: boolean
}

/** Steps counted as done: the user's own check-offs, plus `vm` once a VM exists. */
export function onboardingProgress(
  steps: OnboardingStep[],
  checked: ReadonlySet<string>,
  hasVMs: boolean,
): OnboardingProgress {
  const done = steps.filter((s) => checked.has(s.id) || (s.id === 'vm' && hasVMs)).length
  const total = steps.length
  return { done, total, pct: total === 0 ? 100 : Math.round((100 * done) / total), complete: done === total }
}

export function readChecked(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(ONBOARDING_DONE_KEY) ?? '[]')
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

export function writeChecked(ids: ReadonlySet<string>): void {
  try {
    localStorage.setItem(ONBOARDING_DONE_KEY, JSON.stringify([...ids]))
  } catch {
    // storage unavailable; progress resets next visit
  }
}

export function readDismissed(): boolean {
  try {
    return localStorage.getItem(ONBOARDING_DISMISSED_KEY) === '1'
  } catch {
    return false
  }
}

export function writeDismissed(): void {
  try {
    localStorage.setItem(ONBOARDING_DISMISSED_KEY, '1')
  } catch {
    // ignore
  }
}
