// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useCallback, useEffect, useState } from 'react'
import { Cpu, Loader2, Plus, Trash2 } from 'lucide-react'
import {
  getDeviceInventory,
  permitDevice,
  unpermitDevice,
  type DeviceInventory,
  type PermitDevice,
} from '../api/passthrough'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'

type Kind = 'pci' | 'mediated' | 'usb'

const inputCls =
  'w-full px-3 py-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-sm font-mono'

function buildDevice(kind: Kind, f: Record<string, string>, external: boolean): PermitDevice {
  const base = { resource_name: f.resource.trim(), ...(external ? { external_resource_provider: true } : {}) }
  if (kind === 'pci') return { kind, ...base, pci_vendor_selector: f.pci.trim() }
  if (kind === 'mediated') return { kind, ...base, mdev_name_selector: f.mdev.trim() }
  return { kind, ...base, vendor: f.vendor.trim(), product: f.product.trim() }
}

/** Cluster-admin view of passthrough hardware: what nodes advertise, what KubeVirt permits, SR-IOV pools. */
export default function PassthroughDevices() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [inv, setInv] = useState<DeviceInventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showAdd, setShowAdd] = useState(false)
  const [kind, setKind] = useState<Kind>('pci')
  const [fields, setFields] = useState<Record<string, string>>({ resource: '', pci: '', mdev: '', vendor: '', product: '' })
  const [external, setExternal] = useState(false)
  const [busy, setBusy] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setInv(await getDeviceInventory())
    } catch (e) {
      setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const set = (k: string, v: string) => setFields((f) => ({ ...f, [k]: v }))

  const submit = async () => {
    setBusy(true)
    try {
      const r = await permitDevice(buildDevice(kind, fields, external))
      toast.success(r.changed ? `Permitted ${fields.resource.trim()}` : 'Already permitted')
      setShowAdd(false)
      await load()
    } catch (e) {
      toastFailure(toast, 'Could not permit the device', e)
    } finally {
      setBusy(false)
    }
  }

  const remove = async (resource: string) => {
    if (
      !(await confirm('Stop permitting device', `Remove "${resource}" from the KubeVirt CR's permittedHostDevices? VMs that use it can no longer be started.`, {
        variant: 'danger',
        confirmLabel: 'Remove',
      }))
    )
      return
    try {
      await unpermitDevice(resource)
      toast.success(`Removed ${resource}`)
      await load()
    } catch (e) {
      toastFailure(toast, 'Could not remove the device', e)
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Passthrough devices"
        description="GPUs, SR-IOV virtual functions and other host devices: what nodes advertise and what KubeVirt permits"
        onRefresh={load}
        refreshing={loading}
        primaryAction={
          <button type="button" onClick={() => setShowAdd(!showAdd)} className="zf-btn zf-btn-primary zf-btn-sm">
            <Plus className="w-4 h-4" /> Permit device
          </button>
        }
      />

      {error && <ErrorBanner title="Could not load the device inventory" headline={error} onRetry={() => void load()} />}

      {showAdd && (
        <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-4 space-y-3">
          <p className="text-sm text-[var(--zf-muted)]">
            Adds the device to the KubeVirt CR&apos;s <code>permittedHostDevices</code>. It needs a device plugin that advertises the same
            resource name, and Zorvia must be allowed to patch the KubeVirt CR (Helm <code>devices.managePermitted</code>).
          </p>
          <div className="flex gap-2" role="tablist" aria-label="Device kind">
            {(['pci', 'mediated', 'usb'] as Kind[]).map((k) => (
              <button
                key={k}
                role="tab"
                aria-selected={kind === k}
                onClick={() => setKind(k)}
                className={`px-3 py-1.5 rounded-lg text-sm ${kind === k ? 'bg-[var(--zf-ink)] text-[var(--zf-canvas)]' : 'text-[var(--zf-muted)]'}`}
              >
                {k === 'pci' ? 'PCI (GPU, NIC)' : k === 'mediated' ? 'Mediated (vGPU)' : 'USB'}
              </button>
            ))}
          </div>
          <label className="block text-sm">
            Resource name
            <input className={inputCls} placeholder="nvidia.com/GA102GL_A10" value={fields.resource} onChange={(e) => set('resource', e.target.value)} />
          </label>
          {kind === 'pci' && (
            <label className="block text-sm">
              PCI vendor:product
              <input className={inputCls} placeholder="10DE:2236" value={fields.pci} onChange={(e) => set('pci', e.target.value)} />
            </label>
          )}
          {kind === 'mediated' && (
            <label className="block text-sm">
              Mediated device type name
              <input className={inputCls} placeholder="GRID T4-2Q" value={fields.mdev} onChange={(e) => set('mdev', e.target.value)} />
            </label>
          )}
          {kind === 'usb' && (
            <div className="flex gap-3">
              <label className="block text-sm flex-1">
                Vendor id
                <input className={inputCls} placeholder="46f4" value={fields.vendor} onChange={(e) => set('vendor', e.target.value)} />
              </label>
              <label className="block text-sm flex-1">
                Product id
                <input className={inputCls} placeholder="0001" value={fields.product} onChange={(e) => set('product', e.target.value)} />
              </label>
            </div>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={external} onChange={(e) => setExternal(e.target.checked)} />
            A device plugin outside KubeVirt manages this resource
          </label>
          <button type="button" disabled={busy || !fields.resource.trim()} onClick={() => void submit()} className="zf-btn zf-btn-primary zf-btn-sm">
            {busy ? <Loader2 className="w-4 h-4 animate-spin" /> : 'Permit'}
          </button>
        </div>
      )}

      {loading && !inv ? (
        <Loader2 className="w-5 h-5 animate-spin text-[var(--zf-muted)]" />
      ) : inv ? (
        <>
          <section className="space-y-2">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Permitted by KubeVirt</h2>
            {!inv.kubevirt ? (
              <p className="text-sm text-[var(--zf-muted)]">The KubeVirt CR could not be read.</p>
            ) : inv.kubevirt.permitted.length === 0 ? (
              <EmptyState icon={<Cpu className="w-6 h-6" />} title="No devices permitted" description="KubeVirt refuses to attach any host device until it is permitted." />
            ) : (
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-[var(--zf-muted)]">
                    <th className="py-1">Resource</th>
                    <th>Kind</th>
                    <th>Selector</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {inv.kubevirt.permitted.map((p) => (
                    <tr key={p.resource_name} className="border-t border-[var(--zf-hairline)]">
                      <td className="py-1.5 font-mono">{p.resource_name}</td>
                      <td>{p.kind}</td>
                      <td className="font-mono">{p.selector}</td>
                      <td className="text-right">
                        <button type="button" aria-label={`Remove ${p.resource_name}`} onClick={() => void remove(p.resource_name)} className="text-[var(--zf-muted)] hover:text-[var(--zf-danger)]">
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section className="space-y-2">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Advertised by nodes</h2>
            {inv.nodes.map((n) => {
              const entries = Object.entries(n.devices)
              const hp = Object.entries(n.hugepages ?? {}).filter(([, v]) => v > 0)
              return (
                <div key={n.node} className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-3 text-sm space-y-1">
                  <div className="font-medium text-[var(--zf-ink)]">
                    {n.node}
                    <span className="ml-2 text-xs text-[var(--zf-muted)]">
                      {n.cpu_manager ? 'static CPU manager' : 'no dedicated CPUs'}
                      {hp.length ? ` · hugepages ${hp.map(([k, v]) => `${k}×${v}`).join(', ')}` : ''}
                    </span>
                  </div>
                  {entries.length === 0 ? (
                    <div className="text-[var(--zf-muted)]">No passthrough devices advertised</div>
                  ) : (
                    entries.map(([r, count]) => (
                      <div key={r} className="font-mono">
                        {r} <span className="text-[var(--zf-muted)]">× {count}</span>
                        {inv.kind_hints?.[r] ? <span className="ml-2 text-xs text-[var(--zf-muted)]">{inv.kind_hints[r]}</span> : null}
                      </div>
                    ))
                  )}
                </div>
              )
            })}
          </section>

          <section className="space-y-2">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">SR-IOV networks</h2>
            {!inv.multus_installed ? (
              <p className="text-sm text-[var(--zf-muted)]">Multus is not installed, so SR-IOV interfaces are unavailable.</p>
            ) : !inv.sriov_pools || inv.sriov_pools.length === 0 ? (
              <p className="text-sm text-[var(--zf-muted)]">No network attachment names a virtual-function pool (k8s.v1.cni.cncf.io/resourceName).</p>
            ) : (
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-[var(--zf-muted)]">
                    <th className="py-1">Network</th>
                    <th>Pool</th>
                    <th>Free VFs</th>
                  </tr>
                </thead>
                <tbody>
                  {inv.sriov_pools.map((p) => (
                    <tr key={`${p.namespace}/${p.network}`} className="border-t border-[var(--zf-hairline)]">
                      <td className="py-1.5 font-mono">
                        {p.namespace}/{p.network}
                      </td>
                      <td className="font-mono">{p.resource_name}</td>
                      <td className={p.free === 0 ? 'text-[var(--zf-danger)]' : ''}>
                        {p.free} on {p.nodes} node{p.nodes === 1 ? '' : 's'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>
        </>
      ) : null}
      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel ?? 'Remove'}
          variant={confirmState.variant ?? 'danger'}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}
