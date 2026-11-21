// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, it, expect } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import ExperimentalDisabledNotice from '../ExperimentalDisabledNotice'
import DrPairsTable from '../DrPairsTable'
import FailoverPanel from '../FailoverPanel'
import type { DrPairStatus, FailoverPlan } from '../../api/drPairs'

const pair: DrPairStatus = {
  name: 'web', vm: 'web-1', namespace: 'prod', dr_vm: 'web-1-dr', dr_namespace: 'zorvia-dr',
  phase: 'active', enabled: true, health: 'rpo_breached', rpo_seconds: 3600, lag_seconds: 7200,
  rpo_met: false, last_success: null, last_attempt: null, next_due: null, in_flight_op: null,
  last_error: 'upload failed', consecutive_failures: 2, rto_target_seconds: 1800,
  rto_measured_seconds: 600, rto_met: true,
}

const plan: FailoverPlan = {
  pair: 'web', source: 'prod/web-1', source_state: 'Stopped', dr_target: 'zorvia-dr/web-1-dr',
  backup_op: 'op-1', backup_age_seconds: 120, potential_data_loss_seconds: 120, rpo_met: true,
  rto_target_seconds: 1800, steps: ['restore backup op-1'], warnings: ['backup is stale'],
  blockers: [], allowed: true, confirm_phrase: 'failover web',
}

describe('ExperimentalDisabledNotice', () => {
  it('states clearly that the feature is experimental and disabled', () => {
    const html = renderToStaticMarkup(<ExperimentalDisabledNotice feature="Volume encryption" message="set it" />)
    expect(html).toContain('Volume encryption: experimental, disabled')
    expect(html).toContain('ZORVIA_EXPERIMENTAL=1')
    expect(html).toContain('Server said: set it')
    expect(html).toContain('role="status"')
  })
  it('mentions the extra flag when one is needed', () => {
    const html = renderToStaticMarkup(<ExperimentalDisabledNotice feature="DR" hint="and ZORVIA_FEATURE_DR_REPLICATION=1" />)
    expect(html).toContain('ZORVIA_FEATURE_DR_REPLICATION=1')
  })
})

describe('DrPairsTable', () => {
  const noop = () => {}
  it('shows RPO/RTO status and the breach', () => {
    const html = renderToStaticMarkup(<DrPairsTable pairs={[pair]} canAdmin onFailover={noop} onDelete={noop} />)
    expect(html).toContain('RPO breached')
    expect(html).toContain('2h / 1h')
    expect(html).toContain('breached')
    expect(html).toContain('10m / 30m')
    expect(html).toContain('upload failed')
    expect(html).toContain('aria-valuenow="100"')
  })
  it('hides failover and remove actions for non-admins', () => {
    const html = renderToStaticMarkup(<DrPairsTable pairs={[pair]} canAdmin={false} onFailover={noop} onDelete={noop} />)
    expect(html).not.toContain('Failover</button>')
    expect(html).not.toContain('Remove')
  })
  it('renders an empty state', () => {
    expect(renderToStaticMarkup(<DrPairsTable pairs={[]} canAdmin onFailover={noop} onDelete={noop} />)).toContain('No replication pairs')
  })
})

describe('FailoverPanel', () => {
  const props = { busy: false, onTyped: () => {}, onDryRun: () => {}, onConfirm: () => {}, onClose: () => {} }
  it('disables the real failover until the exact phrase is typed', () => {
    const before = renderToStaticMarkup(<FailoverPanel plan={plan} typed="" {...props} />)
    expect(before).toMatch(/<button[^>]*disabled=""[^>]*>Fail over now/)
    expect(before).toContain('Dry run')
    expect(before).toContain('backup is stale')
    const after = renderToStaticMarkup(<FailoverPanel plan={plan} typed="failover web" {...props} />)
    expect(after).not.toMatch(/<button[^>]*disabled=""[^>]*>Fail over now/)
  })
  it('keeps the real failover disabled when the plan has blockers', () => {
    const blocked = { ...plan, allowed: false, blockers: ['source VM is running'] }
    const html = renderToStaticMarkup(<FailoverPanel plan={blocked} typed="failover web" {...props} />)
    expect(html).toContain('source VM is running')
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Fail over now/)
  })
})
