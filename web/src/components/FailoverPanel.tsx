// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { FailoverPlan } from '../api/drPairs'
import { formatDuration, canConfirmFailover } from '../utils/drPairs'
import { INPUT_CLASS } from './ExperimentalDisabledNotice'

/** Plan view plus the two failover actions. The dry run is always available;
    the real failover needs an allowed plan and the exact confirm phrase. */
export default function FailoverPanel({
  plan,
  typed,
  busy,
  onTyped,
  onDryRun,
  onConfirm,
  onClose,
}: {
  plan: FailoverPlan
  typed: string
  busy: boolean
  onTyped: (v: string) => void
  onDryRun: () => void
  onConfirm: () => void
  onClose: () => void
}) {
  const ok = canConfirmFailover(plan, typed)
  return (
    <div className="space-y-3" data-testid="failover-panel">
      <div className="text-sm font-medium text-[var(--zf-ink)]">
        Failover plan: {plan.source} to {plan.dr_target}
      </div>
      <dl className="grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
        <div><dt className="text-[var(--zf-muted)]">Source state</dt><dd>{plan.source_state}</dd></div>
        <div><dt className="text-[var(--zf-muted)]">Backup age</dt><dd>{formatDuration(plan.backup_age_seconds)}</dd></div>
        <div><dt className="text-[var(--zf-muted)]">Potential data loss</dt><dd>{formatDuration(plan.potential_data_loss_seconds)}</dd></div>
        <div><dt className="text-[var(--zf-muted)]">RTO target</dt><dd>{formatDuration(plan.rto_target_seconds)}</dd></div>
      </dl>
      {plan.blockers.length > 0 && (
        <ul className="text-sm text-[var(--zf-danger)] list-disc pl-5" aria-label="Blockers">
          {plan.blockers.map((b) => <li key={b}>{b}</li>)}
        </ul>
      )}
      {plan.warnings.length > 0 && (
        <ul className="text-sm text-[var(--zf-warning)] list-disc pl-5" aria-label="Warnings">
          {plan.warnings.map((w) => <li key={w}>{w}</li>)}
        </ul>
      )}
      <ol className="text-sm text-[var(--zf-muted)] list-decimal pl-5">
        {plan.steps.map((s) => <li key={s}>{s}</li>)}
      </ol>
      <p className="text-xs text-[var(--zf-muted)]">
        This restores the last verified backup as {plan.dr_target}. Nothing repoints traffic or DNS. A dry run changes nothing.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" onClick={onDryRun} disabled={busy}>
          Dry run
        </button>
        <input
          aria-label="Confirmation phrase"
          placeholder={`Type: ${plan.confirm_phrase}`}
          value={typed}
          onChange={(e) => onTyped(e.target.value)}
          disabled={busy || !plan.allowed}
          className={`${INPUT_CLASS} w-64`}
        />
        <button type="button" className="zf-btn zf-btn-danger zf-btn-sm" onClick={onConfirm} disabled={busy || !ok}>
          Fail over now
        </button>
        <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  )
}
