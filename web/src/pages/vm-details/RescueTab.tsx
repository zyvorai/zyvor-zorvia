// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, ReactNode } from 'react'
import { Wrench, Key, Tag, Lock, Package, Search, AlertTriangle, Loader2 } from 'lucide-react'
import type { VM } from '../../api/vm'
import { rescueVM, pollRescueJob } from '../../api/guestRescue'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { usePermissions } from '../../hooks/usePermissions'

export default function RescueTab({ vm }: { vm: VM }) {
  const toast = useToastContext()
  const { canWrite } = usePermissions()
  const stopped = vm.state === 'stopped'
  const disabled = !canWrite || !stopped

  const [sshUser, setSshUser] = useState('')
  const [sshKey, setSshKey] = useState('')
  const [hostname, setHostname] = useState('')
  const [busy, setBusy] = useState<string | null>(null)

  const run = async (
    key: string,
    actionLabel: string,
    req: Parameters<typeof rescueVM>[1],
  ) => {
    setBusy(key)
    try {
      const { job_name } = await rescueVM(vm.name, req)
      const status = await pollRescueJob(vm.name, job_name)
      if (status.state === 'succeeded') {
        toast.success(status.result?.message ?? `${actionLabel} succeeded`)
      } else if (status.state === 'failed') {
        toast.error(status.result?.message ?? `${actionLabel} failed`)
      } else {
        toast.error(`${actionLabel} is still running -- check back shortly`)
      }
    } catch (err) {
      toastFailure(toast, `${actionLabel} failed`, err)
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="w-full space-y-4">
      <div className="flex items-start gap-2 text-sm text-[var(--zf-muted)] bg-[var(--zf-canvas)] rounded-lg border border-[var(--zf-hairline)] px-4 py-3">
        <Wrench className="w-4 h-4 text-purple-400 shrink-0 mt-0.5" />
        <div>
          Offline guest configuration via GuestKit — runs a privileged Kubernetes Job that mounts
          this VM&apos;s disk directly, no network or in-guest agent needed. SSH key injection
          requires the target user to already exist on the image (e.g.{' '}
          <code className="text-[var(--zf-muted)]">root</code>, or a user your image/cloud-init
          already created).
        </div>
      </div>

      {!stopped && (
        <div className="flex items-center gap-2 text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/20 rounded-lg px-4 py-3">
          <AlertTriangle className="w-4 h-4 shrink-0" />
          Stop this VM first — the rescue Job needs exclusive access to the disk, which a running VM already holds.
        </div>
      )}
      {!canWrite && (
        <p className="text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/20 rounded-lg px-3 py-2">
          Viewer accounts cannot run rescue operations.
        </p>
      )}

      <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-5 space-y-3">
        <div className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)]">
          <Key className="w-4 h-4 text-emerald-600" />
          Inject SSH Key
        </div>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <input
            value={sshUser} onChange={(e) => setSshUser(e.target.value)} disabled={disabled}
            placeholder="Existing Linux user (e.g. root)"
            className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50"
          />
          <input
            value={sshKey} onChange={(e) => setSshKey(e.target.value)} disabled={disabled}
            placeholder="ssh-ed25519 AAAA..."
            className="md:col-span-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] font-mono disabled:opacity-50"
          />
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => run('inject-key', 'Inject SSH key', { operation: 'inject-ssh-key', user: sshUser, key: sshKey })}
            disabled={disabled || !sshUser || !sshKey || busy !== null}
            className="zf-btn zf-btn-primary zf-btn-sm"
          >
            {busy === 'inject-key' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Inject Key
          </button>
          <button
            onClick={() => run('enable-ssh', 'Enable SSH', { operation: 'enable-ssh' })}
            disabled={disabled || busy !== null}
            className="zf-btn zf-btn-ghost zf-btn-sm"
          >
            {busy === 'enable-ssh' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Enable SSH Service
          </button>
        </div>
      </div>

      <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-5 space-y-3">
        <div className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)]">
          <Tag className="w-4 h-4 text-cyan-400" />
          Set Hostname
        </div>
        <div className="flex gap-2">
          <input
            value={hostname} onChange={(e) => setHostname(e.target.value)} disabled={disabled}
            placeholder="new-hostname"
            className="flex-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50"
          />
          <button
            onClick={() => run('hostname', 'Set hostname', { operation: 'set-hostname', hostname })}
            disabled={disabled || !hostname || busy !== null}
            className="zf-btn zf-btn-primary zf-btn-sm"
          >
            {busy === 'hostname' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Apply
          </button>
        </div>
      </div>

      <NotYetAvailable
        icon={<Lock className="w-4 h-4 text-red-600" />}
        title="Reset Password"
        note="Linux has no direct password mutator in GuestKit yet -- needs a chroot+chpasswd follow-up, not just wiring."
      />
      <NotYetAvailable
        icon={<Package className="w-4 h-4 text-amber-400" />}
        title="Install Packages"
        note="Needs guest network egress from inside a privileged rescue Job -- deferred pending that design."
      />
      <NotYetAvailable
        icon={<Search className="w-4 h-4 text-[var(--zf-muted)]" />}
        title="Pull Guest Info / Inspect Disk"
        note="Needs its own research into GuestKit's inspection API -- not yet investigated."
      />
    </div>
  )
}

function NotYetAvailable({ icon, title, note }: { icon: ReactNode; title: string; note: string }) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-5 space-y-2 opacity-60">
      <div className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)]">
        {icon}
        {title}
        <span className="ml-2 px-1.5 py-0.5 rounded text-[10px] font-medium border border-[var(--zf-hairline)] text-[var(--zf-muted)]">
          Not yet available
        </span>
      </div>
      <p className="text-xs text-[var(--zf-muted)]">{note}</p>
    </div>
  )
}
