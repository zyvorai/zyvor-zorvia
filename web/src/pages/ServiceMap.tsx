// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { Link } from 'react-router'
import { Network } from 'lucide-react'
import { getServiceMap, ServiceMapEntry } from '../api/serviceMap'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

function groupByVm(entries: ServiceMapEntry[]): Map<string, ServiceMapEntry[]> {
  const map = new Map<string, ServiceMapEntry[]>()
  for (const e of entries) {
    const list = map.get(e.vm_name) ?? []
    list.push(e)
    map.set(e.vm_name, list)
  }
  return map
}

export default function ServiceMap() {
  const toast = useToastContext()
  const [entries, setEntries] = useState<ServiceMapEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const fetchMap = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setEntries(await getServiceMap())
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load service map', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchMap() }, [fetchMap])

  const grouped = groupByVm(entries)

  return (
    <div className="space-y-6">
      <PageHeader
        title="Service Map"
        description="Every exposed VM service across the fleet, backed by the real NodePort/ClusterIP Services KubeVirt creates for VNC, SSH, RDP, and custom port forwards"
        onRefresh={fetchMap}
        refreshing={loading}
      />

      {loadError && (
        <ErrorBanner title="Could not load service map" headline={loadError} hints={hintsForError(loadError, 'network')} onRetry={fetchMap} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading service map…
        </div>
      ) : !loadError && grouped.size === 0 ? (
        <EmptyState icon={<Network className="w-8 h-8" />} title="No exposed services" description="Expose SSH, VNC, RDP, or a custom port forward on a VM to see it here." />
      ) : !loadError ? (
        <div className="space-y-4">
          {Array.from(grouped.entries()).map(([vmName, services]) => (
            <div key={vmName} className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
              <div className="px-5 py-3 border-b border-[var(--zf-hairline)] flex items-center justify-between">
                <Link to={`/app/vms/${encodeURIComponent(vmName)}`} className="font-medium text-[var(--zf-ink)] hover:underline">{vmName}</Link>
                <span className="text-xs text-[var(--zf-muted)]">{services.length} service{services.length !== 1 ? 's' : ''}</span>
              </div>
              <DataTable
                columns={[
                  { key: 'service', header: 'Service', className: 'px-5', render: (svc) => <span className="font-mono text-xs text-[var(--zf-ink)]">{svc.service_name}</span> },
                  { key: 'port', header: 'Guest Port', render: (svc) => <span className="text-[var(--zf-muted)]">{svc.guest_port}</span> },
                  { key: 'protocol', header: 'Protocol', render: (svc) => <span className="text-[var(--zf-muted)] uppercase text-xs">{svc.protocol}</span> },
                  {
                    key: 'connect',
                    header: 'Connect',
                    render: (svc) => <span className="font-mono text-xs text-[var(--zf-ink)]">{svc.host_port ? `${svc.expose_host}:${svc.host_port}` : '—'}</span>,
                  },
                  { key: 'cluster_ip', header: 'Cluster IP', render: (svc) => <span className="text-[var(--zf-muted)] font-mono text-xs">{svc.cluster_ip || '—'}</span> },
                ]}
                rows={services}
                getRowKey={(svc) => svc.service_name}
                bordered={false}
              />
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
