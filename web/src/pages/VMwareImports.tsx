// Copyright 2026 Zyvor AI Labs · SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0
import { useCallback, useEffect, useState } from 'react'
import { createImportWave, listImports, preflightImport, type ImportSummary, type PreflightResult } from '../api/vmImport'
import { cancelOperation, isTerminal } from '../api/operations'
import { buildImportRequest, emptyImportDraft, preflightMatches, type ImportDraft } from '../utils/vmImportDraft'
import { formatUserError } from '../utils/apiError'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import { PageHeader } from '../components/ui'

export default function VMwareImports() {
  const [draft, setDraft] = useState<ImportDraft>(emptyImportDraft)
  const [checked, setChecked] = useState<string | null>(null)
  const [preflight, setPreflight] = useState<PreflightResult | null>(null)
  const [imports, setImports] = useState<ImportSummary[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [wave, setWave] = useState<string | null>(null)
  const { confirmState, confirm, cancel } = useConfirm()

  const refresh = useCallback(async () => {
    try { setImports(await listImports()); setListError(null) }
    catch (e) { setListError(formatUserError(e)) }
  }, [])
  useEffect(() => { void refresh(); const timer = setInterval(() => void refresh(), 5000); return () => clearInterval(timer) }, [refresh])

  const update = <K extends keyof ImportDraft>(key: K, value: ImportDraft[K]) => {
    setDraft(d => ({ ...d, [key]: value }))
    setChecked(null); setPreflight(null); setError(null)
  }
  const check = async () => {
    setBusy(true); setError(null); setChecked(null)
    try {
      const req = buildImportRequest(draft)
      const result = await preflightImport(req)
      setPreflight(result)
      if (result.ready) setChecked(JSON.stringify(req))
    } catch (e) { setError(formatUserError(e)) }
    finally { setBusy(false) }
  }
  const queue = async () => {
    setBusy(true); setError(null)
    try {
      const req = buildImportRequest(draft)
      if (!preflightMatches(req, checked)) throw new Error('Check readiness again before importing.')
      if (!await confirm('Import VMware VMs', draft.start
        ? 'Imported VMs will start after target networking is configured. Confirm the source VMs are stopped or isolated before cutover.'
        : 'Import these VMs and leave them stopped for inspection?', { confirmLabel: 'Import VMs' })) return
      const result = await createImportWave(req)
      setWave(result.wave_id); setChecked(null); setPreflight(null)
      await refresh()
    } catch (e) { setError(formatUserError(e)) }
    finally { setBusy(false) }
  }
  const cancelImport = async (op: ImportSummary) => {
    if (!await confirm('Cancel import', 'Stop this import Job? Any imported VM and disks remain for inspection. An already-started VM stays running.', { variant: 'danger', confirmLabel: 'Cancel import' })) return
    try { await cancelOperation(op.operation_id); await refresh() }
    catch (e) { setError(formatUserError(e)) }
  }
  const text = (key: 'vcenter' | 'datacenter' | 'secret' | 'namespace' | 'storageClass' | 'scratchSize' | 'attachment' | 'sourceNetwork', label: string, placeholder = '') => (
    <label className="flex flex-col gap-1 text-sm">{label}<input className="zf-input" value={draft[key]} placeholder={placeholder} onChange={e => update(key, e.target.value)} /></label>
  )

  return <div className="space-y-6">
    <PageHeader title="VMware Imports" description="Check readiness, map target networks, and import VM waves from vCenter." onRefresh={() => void refresh()} />
    <p className="text-sm text-[var(--zf-muted)]">Experimental. Requires a configured h2kvm image and existing credentials Secret. Validate with a test VM before production migration.</p>
    {error && <p role="alert" className="text-[var(--zf-danger)]">{error}</p>}
    {wave && <p role="status">Queued wave <span className="font-mono">{wave}</span>.</p>}
    <fieldset disabled={busy} className="space-y-4 rounded-lg border border-[var(--zf-hairline)] p-5">
      <legend className="px-2 font-semibold">Source and destination</legend>
      <div className="grid gap-4 md:grid-cols-2">
        {text('vcenter', 'vCenter host', 'vc.example.com')}{text('datacenter', 'Datacenter (optional)')}
        {text('secret', 'Credentials Secret', 'vcenter-creds')}{text('namespace', 'Target namespace')}
        {text('storageClass', 'Storage class (optional)')}{text('scratchSize', 'Scratch disk size')}
      </div>
      <label className="flex flex-col gap-1 text-sm">VM names, one per line<textarea className="zf-input min-h-28" value={draft.sources} onChange={e => update('sources', e.target.value)} placeholder={'web-01\ndb-01'} /></label>
      <div className="grid gap-4 md:grid-cols-2">
        {text('sourceNetwork', 'Source port group (record only)')}{text('attachment', 'Target network attachment (optional)', 'prod-vlan')}
      </div>
      <p className="text-sm text-[var(--zf-muted)]">The target attachment must exist in the target namespace. This mapping applies to all VMs in the wave. Leave it empty to keep the importer’s networking.</p>
      <label className="flex gap-2 text-sm"><input type="checkbox" checked={draft.includePodNetwork} disabled={!draft.attachment.trim()} onChange={e => update('includePodNetwork', e.target.checked)} />Keep a pod-network NIC alongside the mapped network</label>
      <label className="flex gap-2 text-sm"><input type="checkbox" checked={draft.start} onChange={e => { update('start', e.target.checked); if (!e.target.checked) update('requireAgent', false) }} />Start imported VMs after network configuration</label>
      <label className="flex gap-2 text-sm"><input type="checkbox" checked={draft.requireAgent} disabled={!draft.start} onChange={e => update('requireAgent', e.target.checked)} />Require a connected guest agent to pass boot verification</label>
      <label className="flex flex-col gap-1 text-sm">Boot verification timeout (seconds)<input className="zf-input" type="number" min={30} max={3600} value={draft.bootTimeout} onChange={e => update('bootTimeout', Number(e.target.value))} /></label>
      <div className="flex gap-3"><button type="button" className="zf-btn" onClick={() => void check()}>Check readiness</button><button type="button" className="zf-btn zf-btn-primary" disabled={!checked} onClick={() => void queue()}>Import VMs</button></div>
    </fieldset>
    {preflight && <section aria-label="Readiness results" className="space-y-2">
      <p className="font-semibold">{preflight.ready ? 'Ready to queue this wave' : 'Resolve blockers before importing'}</p>
      {preflight.blockers.length > 0 && <ul className="list-disc pl-5 text-[var(--zf-danger)]">{preflight.blockers.map(b => <li key={b}>{b}</li>)}</ul>}
      {preflight.warnings.length > 0 && <ul className="list-disc pl-5 text-[var(--zf-muted)]">{preflight.warnings.map(w => <li key={w}>{w}</li>)}</ul>}
    </section>}
    <section className="space-y-3"><h2 className="font-semibold">Import operations</h2>
      {listError && <p role="alert" className="text-[var(--zf-danger)]">Could not refresh imports: {listError}</p>}
      <div className="overflow-x-auto"><table className="w-full text-left text-sm"><thead><tr><th>Source VM</th><th>Target</th><th>State</th><th>Progress</th><th>Actions</th></tr></thead><tbody>
        {imports.map(op => <tr key={op.operation_id} className="border-t border-[var(--zf-hairline)]"><td className="py-3">{op.source_vm}</td><td>{op.namespace}/{op.target_vm_name}</td><td>{op.state}{op.error && <p className="text-[var(--zf-danger)]">{op.error}</p>}</td><td>{op.progress}% · {op.phase}</td><td>{!isTerminal(op.state) && <button className="zf-btn zf-btn-sm" onClick={() => void cancelImport(op)}>Cancel</button>}</td></tr>)}
      </tbody></table></div>
      {imports.length === 0 && !listError && <p className="text-[var(--zf-muted)]">No import operations yet.</p>}
    </section>
    {confirmState && <ConfirmDialog title={confirmState.title} message={confirmState.message} confirmLabel={confirmState.confirmLabel} variant={confirmState.variant} onConfirm={confirmState.onConfirm} onCancel={cancel} />}
  </div>
}
