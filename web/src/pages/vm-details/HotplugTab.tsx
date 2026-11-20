// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState } from 'react'
import { Cpu, MemoryStick, HardDrive, Network, Loader2, AlertCircle } from 'lucide-react'
import type { VM } from '../../api/vm'
import {
  hotplugCpu,
  hotplugMemory,
  hotplugDisk,
  hotplugNic,
  hotremoveDisk,
  hotremoveNic,
  type HotplugApplyResult,
} from '../../api/hotplug'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { usePermissions } from '../../hooks/usePermissions'

export default function HotplugTab({ vm }: { vm: VM }) {
  const toast = useToastContext()
  const { canWrite } = usePermissions()
  const [cpuCount, setCpuCount] = useState(vm.cpus + 1)
  const [memMb, setMemMb] = useState(512)
  const [diskPath, setDiskPath] = useState('')
  const [nicBridge, setNicBridge] = useState('')
  const [diskDeviceId, setDiskDeviceId] = useState('')
  const [nicDeviceId, setNicDeviceId] = useState('')
  const [busy, setBusy] = useState<string | null>(null)

  const running = vm.state === 'running'

  const run = async (key: string, fn: () => Promise<unknown>, success: string) => {
    setBusy(key)
    try {
      await fn()
      toast.success(success)
    } catch (e) {
      toastFailure(toast, 'Hotplug failed', e)
    } finally {
      setBusy(null)
    }
  }

  // CPU/memory hotplug is asynchronous on KubeVirt's side: the request not
  // throwing only means the patch was accepted, not that the guest is
  // actually running with it. Route these two through the response's
  // hotplug_converged/restart_required flags instead of a fixed "success"
  // string, so a staged-for-restart or still-applying change doesn't read
  // as a confident, completed success.
  const runHotplugApply = async (
    key: string,
    fn: () => Promise<HotplugApplyResult>,
    convergedMessage: string,
    pendingLabel: string,
  ) => {
    setBusy(key)
    try {
      const result = await fn()
      if (result.hotplug_converged) {
        toast.success(convergedMessage)
      } else if (result.restart_required) {
        toast.warning(`${pendingLabel} staged -- restart the VM to apply it`)
      } else {
        toast.warning(`${pendingLabel} accepted, still applying -- check back shortly`)
      }
    } catch (e) {
      toastFailure(toast, 'Hotplug failed', e)
    } finally {
      setBusy(null)
    }
  }

  if (!running) {
    return (
      <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-8 text-center">
        <AlertCircle className="w-10 h-10 text-[var(--zf-warning)] mx-auto mb-3" />
        <p className="text-[var(--zf-ink)] font-medium mb-1">VM must be running</p>
        <p className="text-[var(--zf-muted)] text-sm">Start the VM to hotplug CPU, memory, disks, or NICs.</p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {!canWrite && (
        <p className="text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/20 rounded-lg px-3 py-2">
          Viewer accounts cannot change live resources.
        </p>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <Section icon={Cpu} title="CPU">
          <div className="flex gap-2 items-end">
            <div className="flex-1">
              <label className="block text-xs text-[var(--zf-muted)] mb-1">Target vCPU count</label>
              <input
                type="number"
                min={1}
                max={256}
                value={cpuCount}
                onChange={(e) => setCpuCount(parseInt(e.target.value) || 1)}
                disabled={!canWrite}
                className="w-full bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50"
              />
            </div>
            <ActionButton
              disabled={!canWrite || busy !== null}
              loading={busy === 'cpu'}
              onClick={() =>
                runHotplugApply(
                  'cpu',
                  () => hotplugCpu(vm.name, { count: cpuCount }),
                  `CPU count set to ${cpuCount}`,
                  `CPU change to ${cpuCount}`,
                )
              }
              label="Apply"
            />
          </div>
        </Section>

        <Section icon={MemoryStick} title="Memory">
          <div className="flex gap-2 items-end">
            <div className="flex-1">
              <label className="block text-xs text-[var(--zf-muted)] mb-1">Add memory (MB)</label>
              <input
                type="number"
                min={128}
                step={128}
                value={memMb}
                onChange={(e) => setMemMb(parseInt(e.target.value) || 128)}
                disabled={!canWrite}
                className="w-full bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50"
              />
            </div>
            <ActionButton
              disabled={!canWrite || busy !== null}
              loading={busy === 'mem'}
              onClick={() =>
                runHotplugApply(
                  'mem',
                  () => hotplugMemory(vm.name, { size_mb: memMb }),
                  `Added ${memMb} MB memory`,
                  `Memory increase of ${memMb} MB`,
                )
              }
              label="Add"
            />
          </div>
        </Section>

        <Section icon={HardDrive} title="Disk">
          <div className="space-y-2">
            <input
              type="text"
              placeholder="Existing PVC or DataVolume name"
              value={diskPath}
              onChange={(e) => setDiskPath(e.target.value)}
              disabled={!canWrite}
              className="w-full bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] font-mono disabled:opacity-50"
            />
            <ActionButton
              disabled={!canWrite || busy !== null || !diskPath.trim()}
              loading={busy === 'disk'}
              onClick={() => run('disk', () => hotplugDisk(vm.name, { path: diskPath.trim() }), 'Disk attached')}
              label="Attach disk"
            />
            <div className="flex gap-2 items-end pt-1">
              <input
                type="text"
                placeholder="Device ID to remove"
                value={diskDeviceId}
                onChange={(e) => setDiskDeviceId(e.target.value)}
                disabled={!canWrite}
                className="flex-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] font-mono disabled:opacity-50"
              />
              <ActionButton
                disabled={!canWrite || busy !== null || !diskDeviceId.trim()}
                loading={busy === 'disk-rm'}
                onClick={() => run('disk-rm', () => hotremoveDisk(vm.name, diskDeviceId.trim()), 'Disk removed')}
                label="Remove"
                variant="danger"
              />
            </div>
          </div>
        </Section>

        <Section icon={Network} title="Network">
          <div className="space-y-2">
            <input
              type="text"
              placeholder="Multus network attachment name"
              value={nicBridge}
              onChange={(e) => setNicBridge(e.target.value)}
              disabled={!canWrite}
              className="w-full bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] disabled:opacity-50"
            />
            <ActionButton
              disabled={!canWrite || busy !== null || !nicBridge.trim()}
              loading={busy === 'nic'}
              onClick={() => run('nic', () => hotplugNic(vm.name, { bridge: nicBridge.trim() }), 'NIC attached')}
              label="Attach NIC"
            />
            <div className="flex gap-2 items-end pt-1">
              <input
                type="text"
                placeholder="NIC device ID to remove"
                value={nicDeviceId}
                onChange={(e) => setNicDeviceId(e.target.value)}
                disabled={!canWrite}
                className="flex-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg px-3 py-2 text-sm text-[var(--zf-ink)] font-mono disabled:opacity-50"
              />
              <ActionButton
                disabled={!canWrite || busy !== null || !nicDeviceId.trim()}
                loading={busy === 'nic-rm'}
                onClick={() => run('nic-rm', () => hotremoveNic(vm.name, nicDeviceId.trim()), 'NIC removed')}
                label="Remove"
                variant="danger"
              />
            </div>
          </div>
        </Section>
      </div>
    </div>
  )
}

function Section({ icon: Icon, title, children }: { icon: typeof Cpu; title: string; children: React.ReactNode }) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-5">
      <h3 className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-4">
        <Icon className="w-4 h-4 text-[var(--zf-link)]" />
        {title}
      </h3>
      {children}
    </div>
  )
}

function ActionButton({
  onClick,
  label,
  disabled,
  loading,
  variant = 'primary',
}: {
  onClick: () => void
  label: string
  disabled?: boolean
  loading?: boolean
  variant?: 'primary' | 'danger'
}) {
  const cls =
    variant === 'danger' ? 'zf-btn zf-btn-danger zf-btn-sm' : 'zf-btn zf-btn-primary zf-btn-sm'
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled || loading}
      className={cls}
    >
      {loading && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
      {label}
    </button>
  )
}
