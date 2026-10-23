// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import { ONBOARDING_STEPS, onboardingProgress, readChecked, readDismissed } from '../onboarding'

describe('onboardingProgress', () => {
  it('starts at zero with no VMs', () => {
    const p = onboardingProgress(ONBOARDING_STEPS, new Set(), false)
    expect(p).toEqual({ done: 0, total: ONBOARDING_STEPS.length, pct: 0, complete: false })
  })
  it('counts the vm step once a VM exists', () => {
    expect(onboardingProgress(ONBOARDING_STEPS, new Set(), true).done).toBe(1)
  })
  it('does not double count a checked vm step', () => {
    expect(onboardingProgress(ONBOARDING_STEPS, new Set(['vm']), true).done).toBe(1)
  })
  it('completes when every step is checked', () => {
    const all = new Set(ONBOARDING_STEPS.map((s) => s.id))
    const p = onboardingProgress(ONBOARDING_STEPS, all, false)
    expect(p.complete).toBe(true)
    expect(p.pct).toBe(100)
  })
  it('treats an empty step list as complete', () => {
    expect(onboardingProgress([], new Set(), false).complete).toBe(true)
  })
  it('has unique step ids and app routes', () => {
    const ids = ONBOARDING_STEPS.map((s) => s.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(ONBOARDING_STEPS.every((s) => s.to.startsWith('/app/'))).toBe(true)
  })
  it('falls back safely when storage is unavailable', () => {
    expect(readChecked().size).toBe(0)
    expect(readDismissed()).toBe(false)
  })
})
