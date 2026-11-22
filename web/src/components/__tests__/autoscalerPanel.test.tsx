// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, it, expect } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import ExperimentalDisabledNotice from '../ExperimentalDisabledNotice'
import { AutoscalerTargetsTable, DecisionsTable } from '../AutoscalerPanel'
import type { AutoscalerTarget, DecisionRecord } from '../../api/autoscaler'

const target: AutoscalerTarget = {
  id: 'web', kind: 'vm_pool', namespace: 'prod', name: 'web-pool', load_metric: 'cpu', min: 2, max: 10, step: 1,
  scale_up_threshold: 80, scale_down_threshold: 30, required_samples: 3, cooldown_secs: 300,
  up_streak: 2, down_streak: 0, last_action: null,
}

const rec = (over: Partial<DecisionRecord>): DecisionRecord => ({
  seq: 1, timestamp: '2026-01-01T00:00:00Z', target: 'web', metric: 'cpu', load_percent: 92, source: 'metrics_api',
  decision: 'scale_up', outcome: 'recommended', from: 3, to: 4, reason: 'recommendation only: dry-run', count: 1,
  ...over,
})

describe('AutoscalerTargetsTable', () => {
  it('shows policy and streak state', () => {
    const html = renderToStaticMarkup(<AutoscalerTargetsTable targets={[target]} />)
    expect(html).toContain('web-pool')
    expect(html).toContain('2 to 10, step 1')
    expect(html).toContain('30% / 80%')
    expect(html).toContain('2 / 0 of 3')
    expect(html).toContain('never')
  })
  it('renders an empty state', () => {
    expect(renderToStaticMarkup(<AutoscalerTargetsTable targets={[]} />)).toContain('No targets configured')
  })
})

describe('DecisionsTable', () => {
  it('shows load, source, decision, outcome and reason', () => {
    const html = renderToStaticMarkup(<DecisionsTable decisions={[rec({})]} />)
    expect(html).toContain('92.0% cpu')
    expect(html).toContain('metrics_api')
    expect(html).toContain('scale up 3 to 4')
    expect(html).toContain('Recommended')
    expect(html).toContain('recommendation only: dry-run')
  })
  it('shows folded repeat counts and skipped rows without load', () => {
    const html = renderToStaticMarkup(
      <DecisionsTable
        decisions={[rec({ seq: 3, decision: 'hold', outcome: 'held', reason: 'cooldown', count: 12 }),
          rec({ seq: 2, decision: 'skipped', outcome: 'skipped', load_percent: null, from: null, to: null, reason: 'load unavailable' })]}
      />,
    )
    expect(html).toContain('cooldown (x12)')
    expect(html).toContain('hold at 3')
    expect(html).toContain('Skipped')
    expect(html).toContain('—')
  })
  it('renders an empty state', () => {
    expect(renderToStaticMarkup(<DecisionsTable decisions={[]} />)).toContain('No decisions recorded yet')
  })
})

describe('disabled state', () => {
  it('says experimental, disabled and names the loop flags', () => {
    const html = renderToStaticMarkup(
      <ExperimentalDisabledNotice feature="Autoscaler loop" hint="and ZORVIA_AUTOSCALER_LOOP=1 with ZORVIA_AUTOSCALER_TARGETS" />,
    )
    expect(html).toContain('Autoscaler loop: experimental, disabled')
    expect(html).toContain('ZORVIA_AUTOSCALER_LOOP=1')
  })
})
