// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useState, useEffect, useCallback } from 'react'
import { Layers, Rocket, Loader2, Search } from 'lucide-react'
import { listTemplatesByFamily, getTemplate, deployTemplate, TemplateDetail } from '../api/templates'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState } from '../components/ui'
import { filterTemplates, countTemplates } from '../utils/templateFilter'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

export default function Templates() {
  const toast = useToastContext()
  const [families, setFamilies] = useState<Record<string, string[]>>({})
  const [selected, setSelected] = useState<string | null>(null)
  const [detail, setDetail] = useState<TemplateDetail | null>(null)
  const [loading, setLoading] = useState(true)
  const [detailLoading, setDetailLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [vmName, setVmName] = useState('')
  const [deploying, setDeploying] = useState(false)
  const [deployError, setDeployError] = useState('')
  const [query, setQuery] = useState('')
  const [familyTag, setFamilyTag] = useState<string | null>(null)

  const fetchFamilies = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setFamilies(await listTemplatesByFamily())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load templates', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchFamilies() }, [fetchFamilies])

  const handleSelect = async (name: string) => {
    setSelected(name)
    setDeployError('')
    setDetailLoading(true)
    try {
      setDetail(await getTemplate(name))
    } catch (err) {
      toastFailure(toast, 'Failed to load template details', err)
    } finally {
      setDetailLoading(false)
    }
  }

  const handleDeploy = async () => {
    if (!selected) return
    if (!vmName.trim()) { setDeployError('VM name is required'); return }
    setDeploying(true)
    setDeployError('')
    try {
      await deployTemplate(selected, vmName.trim())
      toast.success(`Deployed "${vmName}" from ${selected}`)
      setVmName('')
    } catch (err) {
      setDeployError(formatUserError(err))
      toastFailure(toast, 'Failed to deploy template', err)
    } finally {
      setDeploying(false)
    }
  }

  const allFamilyNames = Object.keys(families).sort()
  const visible = filterTemplates(families, query, familyTag)
  const familyNames = Object.keys(visible).sort()
  const matchCount = countTemplates(visible)

  return (
    <div className="space-y-6">
      <PageHeader
        title="VM Templates"
        description="Curated OS presets — real, bootable VM configs with a real container-disk image and cloud-init"
        onRefresh={fetchFamilies}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load templates" headline={loadError} hints={hintsForError(loadError)} onRetry={fetchFamilies} />
      )}

      {!loading && !loadError && allFamilyNames.length > 0 && (
        <div className="space-y-3">
          <div className="relative max-w-md">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--zf-muted)]" aria-hidden="true" />
            <input type="search" value={query} onChange={(e) => setQuery(e.target.value)}
              aria-label="Search templates" placeholder="Search templates (e.g. ubuntu, windows server)"
              className="input-field text-sm w-full pl-9" />
          </div>
          <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Filter by family">
            <button type="button" aria-pressed={familyTag === null} onClick={() => setFamilyTag(null)}
              className={`zf-chip ${familyTag === null ? 'zf-chip-active' : ''}`}>All</button>
            {allFamilyNames.map(f => (
              <button key={f} type="button" aria-pressed={familyTag === f} onClick={() => setFamilyTag(familyTag === f ? null : f)}
                className={`zf-chip ${familyTag === f ? 'zf-chip-active' : ''}`}>{f}</button>
            ))}
            <span className="text-xs text-[var(--zf-muted)] ml-auto" aria-live="polite">{matchCount} template{matchCount === 1 ? '' : 's'}</span>
          </div>
        </div>
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading templates…
        </div>
      ) : !loadError && allFamilyNames.length === 0 ? (
        <EmptyState icon={<Layers className="w-8 h-8" />} title="No templates found" description="No OS templates are registered." />
      ) : !loadError && familyNames.length === 0 ? (
        <EmptyState icon={<Search className="w-8 h-8" />} title="No matching templates" description="Try a different search or clear the family filter." />
      ) : !loadError ? (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          <div className="lg:col-span-2 space-y-4">
            {familyNames.map(family => (
              <div key={family} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
                <div className="px-4 py-2.5 border-b border-[var(--zf-hairline)] text-xs font-semibold text-[var(--zf-muted)] uppercase tracking-wider">{family}</div>
                <div className="p-3 flex flex-wrap gap-2">
                  {families[family].map(name => (
                    <button key={name} onClick={() => handleSelect(name)}
                      className={`zf-chip ${selected === name? 'zf-chip-active' : ''}`}>
                      {name}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] p-5 h-fit sticky top-4">
            {!selected ? (
              <p className="text-sm text-[var(--zf-muted)]">Select a template to see its configuration.</p>
            ) : detailLoading ? (
              <div className="flex items-center justify-center py-8 text-[var(--zf-muted)]">
                <div className="animate-spin w-5 h-5 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full" />
              </div>
            ) : detail ? (
              <div className="space-y-3">
                <h3 className="text-sm font-semibold text-[var(--zf-ink)]">{selected}</h3>
                <dl className="text-sm space-y-1.5">
                  <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">vCPUs</dt><dd className="text-[var(--zf-ink)]">{detail.cpu.cores * detail.cpu.sockets * detail.cpu.threads}</dd></div>
                  {detail.cpu.model && <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">CPU model</dt><dd className="text-[var(--zf-ink)]">{detail.cpu.model}</dd></div>}
                  {detail.machine_type && <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">Machine type</dt><dd className="text-[var(--zf-ink)]">{detail.machine_type}</dd></div>}
                  {detail.eviction_strategy && <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">Eviction</dt><dd className="text-[var(--zf-ink)]">{detail.eviction_strategy}</dd></div>}
                  <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">TPM</dt><dd className="text-[var(--zf-ink)]">{detail.enable_tpm ? 'Enabled' : 'Disabled'}</dd></div>
                  <div className="flex justify-between"><dt className="text-[var(--zf-muted)]">RNG</dt><dd className="text-[var(--zf-ink)]">{detail.enable_rng ? 'Enabled' : 'Disabled'}</dd></div>
                </dl>
                {detail.firmware && (
                  <div>
                    <div className="text-xs font-medium text-[var(--zf-muted)] mb-1 mt-3">Firmware</div>
                    <pre className="text-[10px] bg-[var(--zf-canvas)] rounded-lg p-2 overflow-x-auto">{JSON.stringify(detail.firmware, null, 2)}</pre>
                  </div>
                )}

                <div className="pt-3 border-t border-[var(--zf-hairline)] space-y-2">
                  <label className="block text-xs font-medium text-[var(--zf-muted)]">Deploy as</label>
                  <input type="text" value={vmName} onChange={(e) => setVmName(e.target.value)} placeholder="my-new-vm" className="input-field text-sm w-full" />
                  {deployError && <p className="text-xs text-[var(--zf-danger)]">{deployError}</p>}
                  <button onClick={handleDeploy} disabled={deploying} className="zf-btn zf-btn-primary zf-btn-sm w-full">
                    {deploying ? <Loader2 className="w-4 h-4 animate-spin" /> : <Rocket className="w-4 h-4" />}
                    {deploying ? 'Deploying…' : 'Deploy VM'}
                  </button>
                </div>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  )
}
