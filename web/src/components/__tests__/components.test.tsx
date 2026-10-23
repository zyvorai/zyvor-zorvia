// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import OnboardingChecklist from '../OnboardingChecklist'
import WhatsNew from '../WhatsNew'
import Changelog from '../../pages/Changelog'
import { ONBOARDING_STEPS } from '../../utils/onboarding'
import { WHATS_NEW } from '../../config/whatsNew'

const render = (node: React.ReactNode) => renderToStaticMarkup(<MemoryRouter>{node}</MemoryRouter>)

describe('OnboardingChecklist', () => {
  it('lists every step as a labelled checkbox with a link', () => {
    const html = render(<OnboardingChecklist hasVMs={false} />)
    expect(html).toContain('Set up Zorvia')
    expect(html).toContain(`0 of ${ONBOARDING_STEPS.length} done`)
    for (const step of ONBOARDING_STEPS) {
      expect(html).toContain(step.title)
      expect(html).toContain(`href="${step.to}"`)
    }
    expect(html.match(/role="checkbox"/g)).toHaveLength(ONBOARDING_STEPS.length)
  })
  it('exposes progress to assistive tech', () => {
    const html = render(<OnboardingChecklist hasVMs={true} />)
    expect(html).toContain('role="progressbar"')
    expect(html).toContain(`aria-valuenow="${Math.round(100 / ONBOARDING_STEPS.length)}"`)
  })
  it('marks the vm step done and locks it once a VM exists', () => {
    const html = render(<OnboardingChecklist hasVMs={true} />)
    expect(html).toContain('1 of')
    expect(html).toContain('aria-checked="true"')
    expect(html).toContain('disabled')
  })
  it('can be dismissed', () => {
    expect(render(<OnboardingChecklist hasVMs={false} />)).toContain('Dismiss setup checklist')
  })
})

describe('WhatsNew', () => {
  it('shows an unread dot and count when nothing has been seen', () => {
    const html = render(<WhatsNew />)
    expect(html).toContain(`What&#x27;s new, ${WHATS_NEW.length} unread`)
  })
  it('keeps the dialog closed until opened', () => {
    expect(render(<WhatsNew />)).not.toContain('role="dialog"')
  })
})

describe('Changelog page', () => {
  it('renders every entry, newest first', () => {
    const html = render(<Changelog />)
    let last = -1
    for (const entry of WHATS_NEW) {
      const at = html.indexOf(entry.title.replace(/'/g, '&#x27;'))
      expect(at).toBeGreaterThan(last)
      last = at
    }
  })
})
