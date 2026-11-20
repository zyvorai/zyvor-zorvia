// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { GitCompare, Check, X } from 'lucide-react'
import { listVMs, getVM, VM } from '../api/vm'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { useToastContext } from '../contexts/ToastContext'

interface CompareRow {
  label: string
  source: string
  target: string
  match: boolean
}

function fmtMem(mib?: number): string {
  if (!mib) return '0 MB'
  if (mib >= 1024) return `${(mib / 1024).toFixed(1)} GB`
  return `${mib} MB`
}

function buildRows(source: VM, target: VM): CompareRow[] {
  const row = (label: string, a: string, b: string): CompareRow => ({ label, source: a, target: b, match: a === b })
  const tagsOf = (vm: VM) => (vm.tags ?? []).slice().sort().join(', ') || '(none)'
  return [
    row('State', source.state, target.state),
    row('vCPUs', String(source.cpus ?? 0), String(target.cpus ?? 0)),
    row('Memory', fmtMem(source.memory), fmtMem(target.memory)),
    row('Image', source.image || '(none)', target.image || '(none)'),
    row('IP Address', source.ip || '(none)', target.ip || '(none)'),
    row('Tags', tagsOf(source), tagsOf(target)),
    row('Network tap', String(!!source.network_tap), String(!!target.network_tap)),
    row('Static IP', String(!!source.network_static_ip), String(!!target.network_static_ip)),
  ]
}

export default function VMCompare() {
  const toast = useToastContext()
  const [vms, setVMs] = useState<VM[]>([])
  const [sourceName, setSourceName] = useState('')
  const [targetName, setTargetName] = useState('')
  const [rows, setRows] = useState<CompareRow[] | null>(null)
  const [loadingVMs, setLoadingVMs] = useState(true)
  const [comparing, setComparing] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [compareError, setCompareError] = useState<string | null>(null)

  const loadVMs = useCallback(async () => {
    setLoadingVMs(true)
    setLoadError(null)
    try {
      const list = await listVMs()
      setVMs(list)
      if (!sourceName && list[0]) setSourceName(list[0].name)
      if (!targetName && list[1]) setTargetName(list[1].name)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load VMs', err)
    } finally {
      setLoadingVMs(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast])

  useEffect(() => { loadVMs() }, [loadVMs])

  const handleCompare = async () => {
    if (!sourceName || !targetName) return
    setComparing(true)
    setCompareError(null)
    setRows(null)
    try {
      const [source, target] = await Promise.all([getVM(sourceName), getVM(targetName)])
      setRows(buildRows(source, target))
    } catch (err) {
      setCompareError(formatUserError(err))
      toastFailure(toast, 'Failed to compare VMs', err)
    } finally {
      setComparing(false)
    }
  }

  const diffCount = rows?.filter(r => !r.match).length ?? 0

  return (
    <div className="space-y-6">
      <PageHeader
        title="Compare VMs"
        description="Diff configuration and state between two virtual machines"
        onRefresh={loadVMs}
        refreshing={loadingVMs}
      />

      {loadError && (
        <ErrorBanner title="Could not load VMs" headline={loadError} onRetry={loadVMs} />
      )}

      {!loadError && vms.length === 0 && !loadingVMs ? (
        <EmptyState icon={<GitCompare className="w-8 h-8" />} title="No VMs to compare" description="Create at least two VMs to use comparison." />
      ) : (
        <>
          <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5">
            <div className="grid grid-cols-1 md:grid-cols-[1fr_auto_1fr_auto] gap-4 items-end">
              <div>
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Source VM</label>
                <select value={sourceName} onChange={(e) => setSourceName(e.target.value)} className="input-field text-sm w-full">
                  {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                </select>
              </div>
              <GitCompare className="hidden md:block w-5 h-5 text-[var(--zf-muted)] mb-2.5" />
              <div>
                <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Target VM</label>
                <select value={targetName} onChange={(e) => setTargetName(e.target.value)} className="input-field text-sm w-full">
                  {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                </select>
              </div>
              <button
                type="button"
                onClick={handleCompare}
                disabled={comparing || !sourceName || !targetName || sourceName === targetName}
                className="zf-btn zf-btn-primary zf-btn-sm"
              >
                {comparing ? 'Comparing…' : 'Compare'}
              </button>
            </div>
            {sourceName && targetName && sourceName === targetName && (
              <p className="text-xs text-[var(--zf-warning)] mt-2">Pick two different VMs to compare.</p>
            )}
          </div>

          {compareError && (
            <ErrorBanner title="Could not compare VMs" headline={compareError} onRetry={handleCompare} />
          )}

          {rows && (
            <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
              <div className="px-5 py-3 border-b border-[var(--zf-hairline)] text-xs text-[var(--zf-muted)]">
                {diffCount === 0 ? 'All fields match' : `${diffCount} field${diffCount !== 1 ? 's' : ''} differ`}
              </div>
              <DataTable
                columns={[
                  { key: 'field', header: 'Field', className: 'px-5', render: (r) => <span className="font-medium text-[var(--zf-ink)]">{r.label}</span> },
                  { key: 'source', header: sourceName, render: (r) => <span className="text-[var(--zf-muted)] font-mono text-xs">{r.source}</span> },
                  { key: 'target', header: targetName, render: (r) => <span className="text-[var(--zf-muted)] font-mono text-xs">{r.target}</span> },
                  {
                    key: 'match',
                    header: 'Match',
                    className: 'text-center',
                    render: (r) => (r.match ? <Check className="w-4 h-4 text-[var(--zf-success)] inline" /> : <X className="w-4 h-4 text-[var(--zf-warning)] inline" />),
                  },
                ]}
                rows={rows}
                getRowKey={(r) => r.label}
                bordered={false}
                rowClassName={(r) => (!r.match ? 'bg-[var(--zf-warning)]/10' : '')}
              />
            </div>
          )}
        </>
      )}
    </div>
  )
}
