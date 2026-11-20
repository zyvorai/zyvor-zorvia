// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useCallback, useEffect, useState } from 'react'
import { Bot, Loader2, RefreshCw } from 'lucide-react'
import type { VM } from '../../api/vm'
import {
  getGuestAgent,
  getGuestInventory,
  GUEST_INVENTORY_KINDS,
  type GuestAgentInfo,
  type GuestInventory,
  type GuestInventoryKind,
} from '../../api/guestAgent'
import ErrorBanner from '../../components/ErrorBanner'
import { formatUserError } from '../../utils/apiError'

const CONSISTENCY_TEXT: Record<string, string> = {
  application: 'Application-consistent: the guest hooks ran and the filesystem can be frozen',
  filesystem: 'Filesystem-consistent: freezing works but a guest hook failed',
  crash: 'Crash-consistent: the agent cannot quiesce this guest',
}

/** Read-only views from inside the guest through the Zyvor guest agent. */
export default function GuestAgentTab({ vm }: { vm: VM }) {
  const [info, setInfo] = useState<GuestAgentInfo | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [kind, setKind] = useState<GuestInventoryKind>('packages')
  const [inv, setInv] = useState<GuestInventory | null>(null)
  const [invLoading, setInvLoading] = useState(false)
  const [invError, setInvError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setInfo(await getGuestAgent(vm.name))
    } catch (e) {
      setInfo(null)
      setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [vm.name])

  const loadInventory = useCallback(
    async (k: GuestInventoryKind) => {
      setInvLoading(true)
      setInvError(null)
      try {
        setInv(await getGuestInventory(vm.name, k))
      } catch (e) {
        setInv(null)
        setInvError(formatUserError(e))
      } finally {
        setInvLoading(false)
      }
    },
    [vm.name],
  )

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    if (info) void loadInventory(kind)
  }, [info, kind, loadInventory])

  if (loading) {
    return (
      <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-8 text-center">
        <Loader2 className="w-6 h-6 text-[var(--zf-muted)] mx-auto mb-2 animate-spin" />
        <p className="text-[var(--zf-muted)] text-sm">Asking the guest agent...</p>
      </div>
    )
  }

  if (!info) {
    return (
      <div className="space-y-3">
        <ErrorBanner title="Guest agent unavailable" headline={error ?? 'No answer from the guest agent'} onRetry={() => void load()} />
        <p className="text-sm text-[var(--zf-muted)]">
          This tab needs the Zyvor guest agent. Create the VM with <code>guest_agent: zyvor</code> (Debian-family guests) and
          wait for it to connect; the stock qemu-guest-agent does not provide these views.
        </p>
      </div>
    )
  }

  const readiness = info.snapshot_readiness

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-[var(--zf-ink)] font-medium">
          <Bot className="w-4 h-4" />
          Zyvor guest agent {info.version.version}
          <span className="text-xs text-[var(--zf-muted)]">protocol {info.version.protocol}</span>
        </div>
        <button
          onClick={() => void load()}
          className="flex items-center gap-1.5 px-3 py-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] text-sm"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          Refresh
        </button>
      </div>

      {readiness && (
        <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-4 space-y-2">
          <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Snapshot readiness</h3>
          <p className="text-sm text-[var(--zf-muted)]">{CONSISTENCY_TEXT[readiness.consistency] ?? readiness.consistency}</p>
          {readiness.hooks.length > 0 && (
            <ul className="text-sm space-y-1">
              {readiness.hooks.map((h) => (
                <li key={h.name} className="flex gap-2">
                  <span className={h.success ? 'text-[var(--zf-success)]' : 'text-[var(--zf-danger)]'}>{h.success ? 'ok' : 'failed'}</span>
                  <span className="text-[var(--zf-ink)]">{h.name}</span>
                  {h.message && <span className="text-[var(--zf-muted)]">{h.message}</span>}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-4 space-y-3">
        <div className="flex gap-2 flex-wrap" role="tablist" aria-label="Guest inventory">
          {GUEST_INVENTORY_KINDS.map((k) => (
            <button
              key={k.id}
              role="tab"
              aria-selected={kind === k.id}
              onClick={() => setKind(k.id)}
              className={`px-3 py-1.5 rounded-lg text-sm ${
                kind === k.id ? 'bg-[var(--zf-ink)] text-[var(--zf-canvas)]' : 'text-[var(--zf-muted)] hover:text-[var(--zf-ink)]'
              }`}
            >
              {k.label}
            </button>
          ))}
        </div>
        {invLoading && <Loader2 className="w-4 h-4 animate-spin text-[var(--zf-muted)]" />}
        {invError && <ErrorBanner title="Could not read the inventory" headline={invError} onRetry={() => void loadInventory(kind)} />}
        {inv && !invLoading && (
          <pre className="text-xs text-[var(--zf-ink)] overflow-auto max-h-[28rem] whitespace-pre-wrap break-words">
            {JSON.stringify(inv.inventory, null, 2)}
          </pre>
        )}
        <p className="text-xs text-[var(--zf-muted)]">Read from inside the guest by its agent; needs the vm.power permission.</p>
      </div>
    </div>
  )
}
