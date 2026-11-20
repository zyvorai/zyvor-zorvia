// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

interface StatusBadgeProps {
  status: string
  variant?: 'dot' | 'pill'
  title?: string
  /** Text to show instead of `status` (which still picks the color). */
  label?: string
}

/** Netra: color is deviation — nominal (running/healthy) stays graphite. */
const NOMINAL = 'text-[var(--zf-ink)] bg-[var(--zf-fill-tertiary)] border-[var(--zf-hairline)]'
const DANGER = 'zf-tint-danger'
const WARNING = 'zf-tint-warning'
const SUCCESS = 'zf-tint-success'

const statusStyles: Record<string, string> = {
  running: NOMINAL,
  active: NOMINAL,
  enabled: NOMINAL,
  healthy: NOMINAL,
  completed: SUCCESS,
  success: SUCCESS,
  stopped: DANGER,
  failed: DANGER,
  error: DANGER,
  disabled: DANGER,
  paused: WARNING,
  warning: WARNING,
  pending: WARNING,
  in_progress: WARNING,
  unknown: NOMINAL,
}

const dotColors: Record<string, string> = {
  running: 'bg-[var(--zf-ink)]',
  active: 'bg-[var(--zf-ink)]',
  enabled: 'bg-[var(--zf-ink)]',
  healthy: 'bg-[var(--zf-ink)]',
  completed: 'bg-[var(--zf-success)]',
  success: 'bg-[var(--zf-success)]',
  stopped: 'bg-[var(--zf-danger)]',
  failed: 'bg-[var(--zf-danger)]',
  error: 'bg-[var(--zf-danger)]',
  disabled: 'bg-[var(--zf-danger)]',
  paused: 'bg-[var(--zf-warning)]',
  warning: 'bg-[var(--zf-warning)]',
  pending: 'bg-[var(--zf-warning)]',
  in_progress: 'bg-[var(--zf-warning)]',
  unknown: 'bg-[var(--zf-muted)]',
}

const isDeviated = (s: string) =>
  ['stopped', 'failed', 'error', 'disabled', 'paused', 'warning', 'pending', 'in_progress'].includes(
    s.toLowerCase(),
  )

export function StatusBadge({ status, variant = 'pill', title, label }: StatusBadgeProps) {
  const key = status.toLowerCase()
  const style = statusStyles[key] || statusStyles.unknown
  const pulse = isDeviated(key) && ['pending', 'in_progress'].includes(key)

  if (variant === 'dot') {
    const dot = dotColors[key] || dotColors.unknown
    return (
      <span className="flex items-center gap-2" title={title}>
        <span className="relative flex h-2 w-2">
          {pulse && <span className={`absolute inset-0 rounded-full ${dot} opacity-40 animate-ping`} />}
          <span className={`relative w-2 h-2 rounded-full ${dot}`} />
        </span>
        <span className="capitalize text-[var(--zf-ink)]">{label ?? status}</span>
      </span>
    )
  }

  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium capitalize border ${style}`}
    >
      <span className="relative flex h-1.5 w-1.5">
        {pulse && (
          <span
            className={`absolute inset-0 rounded-full ${dotColors[key] || dotColors.unknown} opacity-40 animate-ping`}
          />
        )}
        <span className={`relative w-1.5 h-1.5 rounded-full ${dotColors[key] || dotColors.unknown}`} />
      </span>
      {label ?? status}
    </span>
  )
}
