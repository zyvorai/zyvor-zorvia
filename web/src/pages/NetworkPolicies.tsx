// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { ShieldCheck, Plus, Trash2, Loader2 } from 'lucide-react'
import { listNetworkPolicies, createNetworkPolicy, deleteNetworkPolicy, NetworkPolicy } from '../api/networkPolicies'
import { listVMs, VM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

function ruleSummary(rules: NetworkPolicy['ingress']): string {
  if (rules.length === 0) return 'deny all'
  return rules.map(r => `${r.cidr || 'any'}${r.port ? `:${r.port}` : ''}${r.protocol ? `/${r.protocol}` : ''}`).join(', ')
}

export default function NetworkPolicies() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [policies, setPolicies] = useState<NetworkPolicy[]>([])
  const [vms, setVms] = useState<VM[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [name, setName] = useState('')
  const [vmName, setVmName] = useState('')
  const [ingressCidr, setIngressCidr] = useState('')
  const [ingressPort, setIngressPort] = useState('')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState('')
  const [deletingName, setDeletingName] = useState<string | null>(null)

  const fetchAll = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [policyList, vmList] = await Promise.all([listNetworkPolicies(), listVMs()])
      setPolicies(policyList)
      setVms(vmList)
    } catch (err) {
      setLoadError(formatUserError(err))
      toastFailure(toast, 'Failed to load network policies', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchAll() }, [fetchAll])

  const handleCreate = async () => {
    if (!name.trim()) { setCreateError('Name is required'); return }
    setCreating(true)
    setCreateError('')
    try {
      await createNetworkPolicy({
        name: name.trim(),
        vm_name: vmName || undefined,
        ingress: ingressCidr.trim() || ingressPort.trim()
          ? [{ cidr: ingressCidr.trim() || undefined, port: ingressPort.trim() ? parseInt(ingressPort, 10) : undefined }]
          : undefined,
      })
      toast.success(`Policy "${name}" created`)
      setName(''); setVmName(''); setIngressCidr(''); setIngressPort(''); setShowCreate(false)
      fetchAll()
    } catch (err) {
      setCreateError(formatUserError(err))
      toastFailure(toast, 'Failed to create policy', err)
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async (policy: NetworkPolicy) => {
    if (!await confirm('Delete Policy', `Delete network policy "${policy.name}"? This removes its ingress/egress restrictions immediately.`, { variant: 'danger', confirmLabel: 'Delete' })) return
    setDeletingName(policy.name)
    try {
      await deleteNetworkPolicy(policy.name)
      setPolicies(prev => prev.filter(p => p.name !== policy.name))
      toast.success('Policy deleted')
    } catch (err) {
      toastFailure(toast, 'Failed to delete policy', err)
    } finally {
      setDeletingName(null)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Network Policies"
        description="Real Kubernetes NetworkPolicy objects scoped to VMs via KubeVirt's pod labels"
        onRefresh={fetchAll}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowCreate(!showCreate)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> New Policy
          </button>
        }
      />

      {loadError && (
        <ErrorBanner title="Could not load network policies" headline={loadError} hints={hintsForError(loadError, 'network')} onRetry={fetchAll} />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading policies…
        </div>
      ) : !loadError ? (
        <>
          {showCreate && (
            <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
              <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New Policy</h3>
              <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Name</label>
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="allow-web-ingress" className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Applies to (optional)</label>
                  <select value={vmName} onChange={(e) => setVmName(e.target.value)} className="input-field text-sm w-full">
                    <option value="">Every pod in namespace</option>
                    {vms.map(vm => <option key={vm.name} value={vm.name}>{vm.name}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Allow ingress from (CIDR)</label>
                  <input type="text" value={ingressCidr} onChange={(e) => setIngressCidr(e.target.value)} placeholder="10.0.0.0/8" className="input-field text-sm w-full" />
                </div>
                <div>
                  <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">On port</label>
                  <input type="number" value={ingressPort} onChange={(e) => setIngressPort(e.target.value)} placeholder="443" className="input-field text-sm w-full" />
                </div>
              </div>
              <p className="text-xs text-[var(--zf-muted)]">Leaving both ingress fields empty creates a default-deny policy for the selected scope.</p>
              {createError && <p className="text-sm text-[var(--zf-danger)]">{createError}</p>}
              <div className="flex gap-2">
                <button onClick={handleCreate} disabled={creating} className="zf-btn zf-btn-primary zf-btn-sm">
                  {creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                  {creating ? 'Creating...' : 'Create'}
                </button>
                <button onClick={() => { setShowCreate(false); setCreateError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
              </div>
            </div>
          )}

          <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <DataTable
              columns={[
                { key: 'name', header: 'Name', className: 'px-5', render: (p) => <span className="font-medium text-[var(--zf-ink)]">{p.name}</span> },
                { key: 'applies_to', header: 'Applies To', render: (p) => <span className="text-[var(--zf-muted)]">{p.vm_name || 'Every pod'}</span> },
                { key: 'ingress', header: 'Ingress', render: (p) => <span className="text-xs text-[var(--zf-muted)]">{ruleSummary(p.ingress)}</span> },
                { key: 'egress', header: 'Egress', render: (p) => <span className="text-xs text-[var(--zf-muted)]">{ruleSummary(p.egress)}</span> },
                {
                  key: 'actions',
                  header: 'Actions',
                  className: 'text-right',
                  render: (p) => (
                    <button onClick={() => handleDelete(p)} disabled={deletingName === p.name} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors disabled:opacity-50" title="Delete policy">
                      {deletingName === p.name ? <Loader2 className="w-4 h-4 animate-spin" /> : <Trash2 className="w-4 h-4" />}
                    </button>
                  ),
                },
              ]}
              rows={policies}
              getRowKey={(p) => p.name}
              bordered={false}
              emptyState={
                <EmptyState icon={<ShieldCheck className="w-8 h-8" />} title="No network policies" description="Create one to restrict ingress/egress traffic for a VM or the whole namespace." />
              }
            />
          </div>
        </>
      ) : null}

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel ?? 'Delete'}
          variant={confirmState.variant ?? 'danger'}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}
