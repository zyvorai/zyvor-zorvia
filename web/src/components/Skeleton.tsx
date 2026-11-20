// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

interface SkeletonProps {
  className?: string
  style?: React.CSSProperties
}

/** Shared shimmer block -- the `.skeleton` CSS class (main.css) is the one
 * skeleton treatment used both here and by PageSkeleton's route-level
 * fallback, so a loading state looks the same everywhere. */
export function Skeleton({ className = '', style }: SkeletonProps) {
  return <div className={`skeleton ${className}`} style={style} />
}

export function SkeletonText({ className = '' }: SkeletonProps) {
  return <Skeleton className={`h-4 ${className}`} />
}

export function SkeletonCard() {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
      <div className="p-5 space-y-3">
        <div className="flex items-start justify-between">
          <div className="space-y-2 flex-1">
            <Skeleton className="h-5 w-36" />
            <Skeleton className="h-3 w-24" />
          </div>
          <Skeleton className="h-5 w-16 rounded-full" />
        </div>
        <div className="flex gap-4">
          <Skeleton className="h-4 w-20" />
          <Skeleton className="h-4 w-20" />
        </div>
      </div>
      <div className="px-5 py-3 border-t border-[var(--zf-hairline)] flex gap-2">
        <Skeleton className="h-8 w-16 rounded-md" />
        <Skeleton className="h-8 w-16 rounded-md" />
      </div>
    </div>
  )
}

export function SkeletonTable({ rows = 5, cols = 4 }: { rows?: number; cols?: number }) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
      <div className="grid gap-4 p-4 border-b border-[var(--zf-hairline)]" style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}>
        {Array.from({ length: cols }).map((_, i) => (
          <Skeleton key={i} className="h-3" />
        ))}
      </div>
      {Array.from({ length: rows }).map((_, row) => (
        <div
          key={row}
          className="grid gap-4 p-4 border-b border-[var(--zf-hairline)]/50 last:border-b-0"
          style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}
        >
          {Array.from({ length: cols }).map((_, col) => (
            <Skeleton key={col} className="h-4" />
          ))}
        </div>
      ))}
    </div>
  )
}

export function SkeletonChart() {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-xl p-5 border border-[var(--zf-hairline)]">
      <div className="flex items-center justify-between mb-4">
        <Skeleton className="h-4 w-28" />
        <Skeleton className="h-5 w-12" />
      </div>
      <div className="flex items-end gap-1.5 h-[180px]">
        {Array.from({ length: 20 }).map((_, i) => (
          <Skeleton
            key={i}
            className="flex-1 rounded-sm"
            style={{ height: `${15 + Math.random() * 75}%` } as React.CSSProperties}
          />
        ))}
      </div>
    </div>
  )
}

export function SkeletonDashboard() {
  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <Skeleton className="h-7 w-32" />
        <Skeleton className="h-4 w-56" />
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="bg-[var(--zf-canvas)] rounded-xl p-5 border border-[var(--zf-hairline)]">
            <Skeleton className="h-8 w-8 rounded-lg mb-3" />
            <Skeleton className="h-7 w-16 mb-1" />
            <Skeleton className="h-3 w-20" />
          </div>
        ))}
      </div>
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <SkeletonChart />
        <SkeletonChart />
      </div>
    </div>
  )
}
