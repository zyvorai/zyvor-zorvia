// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { FlaskConical } from 'lucide-react'

export const INPUT_CLASS =
  'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50'

/** Shown in place of a feature's controls when the API answers 403
    EXPERIMENTAL_DISABLED / feature_disabled. */
export default function ExperimentalDisabledNotice({
  feature,
  message,
  hint,
}: {
  feature: string
  message?: string
  hint?: string
}) {
  return (
    <div
      role="status"
      data-testid="experimental-disabled"
      className="rounded-xl border border-[var(--zf-hairline)] bg-[var(--zf-surface)] p-5 flex gap-3"
    >
      <FlaskConical className="w-5 h-5 shrink-0 text-[var(--zf-warning)]" aria-hidden="true" />
      <div className="space-y-1 text-sm">
        <div className="font-medium text-[var(--zf-ink)]">{feature}: experimental, disabled</div>
        <p className="text-[var(--zf-muted)]">
          This feature is Experimental and is switched off on this server. An administrator can enable it with{' '}
          <code>ZORVIA_EXPERIMENTAL=1</code>
          {hint ? ` ${hint}` : ''} and a restart of the API.
        </p>
        {message && <p className="text-xs text-[var(--zf-muted)]">Server said: {message}</p>}
      </div>
    </div>
  )
}

export function ExperimentalBadge() {
  return (
    <span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded border border-[var(--zf-hairline)] text-[var(--zf-muted)]">
      Experimental
    </span>
  )
}
