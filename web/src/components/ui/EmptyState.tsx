// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { ReactNode } from 'react'

interface EmptyStateProps {
  icon?: ReactNode
  title: string
  description?: string
  action?: ReactNode
}

/** Netra-style list empty: title + one sentence + next action. */
export function EmptyState({ icon, title, description, action }: EmptyStateProps) {
  return (
    <div className="list-empty">
      {icon ? <div className="text-[var(--zf-muted)] mb-2 opacity-80">{icon}</div> : null}
      <h3>{title}</h3>
      {description ? <p>{description}</p> : null}
      {action ? <div className="list-empty-action">{action}</div> : null}
    </div>
  )
}
