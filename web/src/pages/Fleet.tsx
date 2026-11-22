// Copyright 2026 Zyvor AI Labs · SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0
import { useCallback, useEffect, useRef, useState } from 'react'
import { getFleet, type FleetSnapshot } from '../api/fleet'
import { fleetIsStale, fleetRows, inventoryCount } from '../utils/fleet'
import { formatUserError } from '../utils/apiError'
import { PageHeader } from '../components/ui'

export default function Fleet() {
  const [snapshot, setSnapshot] = useState<FleetSnapshot | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [now, setNow] = useState(Date.now())
  const [query, setQuery] = useState('')
  const [cluster, setCluster] = useState('')
  const [page, setPage] = useState(0)
  const inFlight = useRef(false)
  const mounted = useRef(false)
  const refresh = useCallback(async () => {
    if (inFlight.current) return
    inFlight.current = true
    setLoading(true)
    try {
      const data = await getFleet()
      if (mounted.current) { setSnapshot(data); setError(null) }
    } catch (e) { if (mounted.current) setError(formatUserError(e)) }
    finally { inFlight.current = false; if (mounted.current) setLoading(false) }
  }, [])
  useEffect(() => {
    mounted.current = true
    void refresh()
    const poll = setInterval(() => { if (document.visibilityState === 'visible') void refresh() }, 30_000)
    const clock = setInterval(() => setNow(Date.now()), 5000)
    return () => { mounted.current = false; clearInterval(poll); clearInterval(clock) }
  }, [refresh])
  const stale = snapshot !== null && fleetIsStale(snapshot.observed_at, now)
  const rows = fleetRows(snapshot?.clusters ?? [], query, cluster)
  const currentPage = Math.min(page, Math.max(0, Math.ceil(rows.length / 50) - 1))
  return <div className="space-y-6">
    <PageHeader title="Fleet" description="Read-only VM inventory and node readiness across enrolled clusters." onRefresh={() => void refresh()} refreshing={loading} />
    <p className="text-sm text-[var(--zf-muted)]">Experimental. Clusters and namespace scopes are enrolled by your administrator. Refreshes every 30 seconds.</p>
    {error && <p role="alert" className="text-[var(--zf-danger)]">{error}{snapshot && ' Displaying the last successful response; it may be outdated.'}</p>}
    {stale && <p role="alert" className="text-[var(--zf-danger)]">Inventory is stale. Treat these counts and readiness results as historical.</p>}
    {!snapshot && !error && <p role="status">Loading fleet inventory…</p>}
    {snapshot && <>
      <p className="text-sm text-[var(--zf-muted)]">Observed {new Date(snapshot.observed_at).toLocaleString()}. {snapshot.totals.complete_clusters}/{snapshot.totals.configured_clusters} clusters fully observed. {snapshot.totals.partial ? 'Partial totals: ' : 'Totals: '}{snapshot.totals.observed_vms.toLocaleString()} observed VMs, {snapshot.totals.observed_nodes.toLocaleString()} observed nodes.</p>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {snapshot.clusters.map(c => <section key={c.name} className="space-y-3 rounded-xl border border-[var(--zf-hairline)] bg-[var(--zf-surface)] p-5">
          <div className="flex justify-between gap-2"><h2 className="font-semibold">{c.name}</h2><span className={c.health === 'healthy' && !stale && !error ? 'text-[var(--zf-success)]' : 'text-[var(--zf-muted)]'}>{stale || error ? 'Historical' : c.health}</span></div>
          <p className="text-sm text-[var(--zf-muted)]">{[c.environment, c.region].filter(Boolean).join(' · ') || 'No environment or region label'}</p>
          <p className="text-sm break-words">Namespaces: {c.namespaces.join(', ')}</p>
          <dl className="grid grid-cols-2 gap-2 text-sm"><dt>VMs</dt><dd>{inventoryCount(c.vm_count)}</dd><dt>Ready VMs</dt><dd>{inventoryCount(c.ready_vms)}</dd><dt>Nodes</dt><dd>{inventoryCount(c.node_count)}</dd><dt>Ready nodes</dt><dd>{inventoryCount(c.ready_nodes)}</dd></dl>
          {c.issues.map(issue => <p key={issue} className="text-sm text-[var(--zf-danger)]">{issue}</p>)}
          <p className="text-xs text-[var(--zf-muted)]">Checked {new Date(c.observed_at).toLocaleString()}</p>
        </section>)}
      </div>
      <p className="text-sm text-[var(--zf-muted)]">Healthy means all observed nodes are Ready and both inventory queries succeeded. It does not verify guest applications. Stopped VMs need not be Ready.</p>
      <div className="flex flex-wrap gap-3">
        <label className="flex flex-col gap-1 text-sm">Search inventory<input className="zf-input" value={query} onChange={e => { setQuery(e.target.value); setPage(0) }} placeholder="VM, namespace, cluster, status" /></label>
        <label className="flex flex-col gap-1 text-sm">Cluster<select className="zf-input" value={cluster} onChange={e => { setCluster(e.target.value); setPage(0) }}><option value="">All enrolled clusters</option>{snapshot.clusters.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}</select></label>
      </div>
      <div className="overflow-x-auto rounded-xl border border-[var(--zf-hairline)]">
        <table className="w-full text-left text-sm"><caption className="p-3 text-left">Observed VMs ({rows.length.toLocaleString()} matching){stale || error ? ' — historical inventory' : ''}</caption><thead><tr>{['Cluster', 'Namespace', 'VM', 'Status', 'Ready'].map(h => <th key={h} scope="col" className="p-3">{h}</th>)}</tr></thead><tbody>
          {rows.slice(currentPage * 50, currentPage * 50 + 50).map(vm => <tr key={JSON.stringify([vm.cluster, vm.namespace, vm.name])} className="border-t border-[var(--zf-hairline)]"><td className="p-3">{vm.cluster}</td><td className="p-3">{vm.namespace}</td><td className="p-3 font-mono">{vm.name}</td><td className="p-3">{vm.status}</td><td className="p-3">{vm.ready === null ? 'Unknown' : vm.ready ? 'Yes' : 'No'}</td></tr>)}
          {rows.length === 0 && <tr><td colSpan={5} className="p-5">No observed VMs match. Check cluster issues before assuming an empty fleet.</td></tr>}
        </tbody></table>
      </div>
      {rows.length > 50 && <div className="flex items-center gap-3"><button className="zf-btn zf-btn-ghost" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>Previous</button><span>Page {currentPage + 1} of {Math.ceil(rows.length / 50)}</span><button className="zf-btn zf-btn-ghost" disabled={(currentPage + 1) * 50 >= rows.length} onClick={() => setPage(currentPage + 1)}>Next</button></div>}
    </>}
  </div>
}
