// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { ReactNode, type ComponentType } from 'react'
import { RefreshCw } from 'lucide-react'

interface PageHeaderProps {
  title: string
  description?: string
  eyebrow?: string
  actions?: ReactNode
  onRefresh?: () => void
  refreshing?: boolean
  primaryAction?: ReactNode
  icon?: ComponentType<{ className?: string }>
  iconColor?: 'blue' | 'green' | 'purple' | 'orange' | 'red' | 'cyan'
}

export function PageHeader({
  title,
  description,
  eyebrow,
  actions,
  onRefresh,
  refreshing,
  primaryAction,
}: PageHeaderProps) {
  return (
    <div className="page-hero flex flex-wrap items-end justify-between gap-4 mb-8 animate-fade-in">
      <div className="min-w-0">
        {eyebrow ? <p className="apple-eyebrow">{eyebrow}</p> : null}
        <h1 className="page-hero-title truncate">{title}</h1>
        {description && <p className="page-hero-lede">{description}</p>}
      </div>
      <div className="flex items-center gap-2 shrink-0 pb-1">
        {onRefresh && (
          <button
            type="button"
            onClick={onRefresh}
            disabled={refreshing}
            className="zf-btn zf-btn-ghost zf-btn-sm"
            title="Refresh"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
            Refresh
          </button>
        )}
        {primaryAction}
        {actions}
      </div>
    </div>
  )
}
