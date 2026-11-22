// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { MouseEvent, ReactNode, useMemo, useState } from 'react'
import { ChevronUp, ChevronDown, ChevronsUpDown } from 'lucide-react'
import { SkeletonTable } from '../Skeleton'

export interface DataTableColumn<T> {
  /** Unique key for the column; also used as the React key for cells. */
  key: string
  header: ReactNode
  render: (row: T) => ReactNode
  /** Applied to both the <th> and every <td> in this column (e.g. widths). */
  className?: string
  /** Enables the click-to-sort header affordance for this column. */
  sortable?: boolean
  /** Value to compare when sorting -- required if `sortable` is true. */
  sortValue?: (row: T) => string | number
}

interface DataTableProps<T> {
  columns: DataTableColumn<T>[]
  rows: T[]
  getRowKey: (row: T) => string
  onRowClick?: (row: T, e: MouseEvent<HTMLTableRowElement>) => void
  onRowDoubleClick?: (row: T, e: MouseEvent<HTMLTableRowElement>) => void
  rowClassName?: (row: T) => string
  /** Renders a leading checkbox column when provided. */
  selectedKeys?: Set<string>
  onToggleRow?: (key: string) => void
  onToggleAll?: () => void
  loading?: boolean
  skeletonRows?: number
  /** Shown instead of the table when `rows` is empty and not loading. */
  emptyState?: ReactNode
  /**
   * Pins the header below the console topbar while the table scrolls past
   * it. Default false: the standard rounded-corner wrapper clips with
   * `overflow: hidden`, which becomes the sticky containing block and makes
   * the header cover the first row instead of scrolling normally. Only
   * enable this for a table inside its own genuinely scrollable container
   * (e.g. a fixed-height `overflow-y: auto` box), not the default wrapper.
   */
  stickyHeader?: boolean
  /** Renders the default bg/border/rounded-corner wrapper. Set false when embedding inside a caller-owned card (e.g. one with its own header). Default true. */
  bordered?: boolean
  className?: string
}

type SortDirection = 'asc' | 'desc'

export function DataTable<T>({
  columns,
  rows,
  getRowKey,
  onRowClick,
  onRowDoubleClick,
  rowClassName,
  selectedKeys,
  onToggleRow,
  onToggleAll,
  loading = false,
  skeletonRows = 5,
  emptyState,
  stickyHeader = false,
  bordered = true,
  className = '',
}: DataTableProps<T>) {
  const [sort, setSort] = useState<{ key: string; direction: SortDirection } | null>(null)
  const selectable = selectedKeys !== undefined && onToggleRow !== undefined

  const sortedRows = useMemo(() => {
    if (!sort) return rows
    const column = columns.find((c) => c.key === sort.key)
    if (!column?.sortValue) return rows
    const { sortValue } = column
    const sign = sort.direction === 'asc' ? 1 : -1
    return [...rows].sort((a, b) => {
      const av = sortValue(a)
      const bv = sortValue(b)
      if (av < bv) return -1 * sign
      if (av > bv) return 1 * sign
      return 0
    })
  }, [rows, sort, columns])

  const toggleSort = (key: string) => {
    setSort((prev) => {
      if (!prev || prev.key !== key) return { key, direction: 'asc' }
      if (prev.direction === 'asc') return { key, direction: 'desc' }
      return null
    })
  }

  if (loading) {
    return <SkeletonTable rows={skeletonRows} cols={columns.length} />
  }

  if (rows.length === 0 && emptyState) {
    return (
      <div className={bordered ? `table-wrap ${className}` : className}>
        {emptyState}
      </div>
    )
  }

  const allSelected = selectable && rows.length > 0 && rows.every((r) => selectedKeys!.has(getRowKey(r)))

  return (
    <div className={bordered ? `table-wrap ${className}` : className}>
      <table>
        <thead>
          <tr className={stickyHeader ? 'zf-table-header-row' : ''}>
            {selectable && (
              <th className="w-10">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={() => onToggleAll?.()}
                  aria-label="Select all rows"
                />
              </th>
            )}
            {columns.map((col) => (
              <th key={col.key} className={col.className ?? ''}>
                {col.sortable ? (
                  <button
                    type="button"
                    className="zf-table-sort-btn"
                    onClick={() => toggleSort(col.key)}
                  >
                    {col.header}
                    {sort?.key === col.key ? (
                      sort.direction === 'asc' ? (
                        <ChevronUp className="w-3 h-3" />
                      ) : (
                        <ChevronDown className="w-3 h-3" />
                      )
                    ) : (
                      <ChevronsUpDown className="w-3 h-3 opacity-40" />
                    )}
                  </button>
                ) : (
                  col.header
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sortedRows.map((row) => {
            const key = getRowKey(row)
            const selected = selectable && selectedKeys!.has(key)
            return (
              <tr
                key={key}
                onClick={(e) => onRowClick?.(row, e)}
                onDoubleClick={(e) => onRowDoubleClick?.(row, e)}
                title={onRowDoubleClick ? 'Double-click to open details' : undefined}
                className={`zf-table-row group ${selected ? 'selected' : ''} ${onRowClick || onRowDoubleClick ? 'cursor-pointer' : ''} ${rowClassName?.(row) ?? ''}`}
              >
                {selectable && (
                  <td className="w-10" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={selected}
                      onChange={() => onToggleRow!(key)}
                    />
                  </td>
                )}
                {columns.map((col) => (
                  <td key={col.key} className={col.className ?? ''}>
                    {col.render(row)}
                  </td>
                ))}
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
