// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiFetch, apiGet, apiPost } from './client'
import { formatHttpErrorBody } from '../utils/apiError'

const API_BASE = '/api'

/** Only these three are wired to a real backend (see docs/RESCUE.md) --
    each runs GuestKit directly against a `Guestfs` mutator or, for
    enable-ssh, the same systemd-enable symlink trick GuestKit's own CLI
    uses. `reset-password` and `install-packages` are real UI fields with
    no backend yet: Linux has no direct password mutator in GuestKit
    (Windows needs a libhivex FFI feature), and package install needs guest
    network egress from inside a privileged Job -- both need their own
    follow-up, not just wiring. */
export type SupportedRescueRequest =
  | { operation: 'inject-ssh-key'; user: string; key: string }
  | { operation: 'enable-ssh' }
  | { operation: 'set-hostname'; hostname: string }

export interface RescueJobEnvelope {
  job_name: string
  state: 'queued'
}

export interface RescueJobStatus {
  state: 'pending' | 'running' | 'succeeded' | 'failed'
  result?: { success: boolean; operation: string; message: string } | null
}

/** Offline guest configuration via GuestKit -- mounts the VM's disk
    directly in a privileged Kubernetes Job, no in-guest agent or guest
    network needed. The VM must be stopped first (a running VM's PVC is
    already attached to virt-launcher). Returns immediately with a job to
    poll via `getRescueJob`/`pollRescueJob`, not a finished result. */
export async function rescueVM(vmName: string, req: SupportedRescueRequest): Promise<RescueJobEnvelope> {
  return apiPost<RescueJobEnvelope>(`${API_BASE}/vms/${encodeURIComponent(vmName)}/rescue`, req)
}

export async function getRescueJob(vmName: string, jobName: string): Promise<RescueJobStatus> {
  return apiGet<RescueJobStatus>(`${API_BASE}/vms/${encodeURIComponent(vmName)}/rescue/${encodeURIComponent(jobName)}`)
}

export async function deleteRescueJob(vmName: string, jobName: string): Promise<void> {
  const res = await apiFetch(`${API_BASE}/vms/${encodeURIComponent(vmName)}/rescue/${encodeURIComponent(jobName)}`, {
    method: 'DELETE',
  })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
  }
}

/** Polls a rescue Job to a terminal state (or gives up after `timeoutMs`,
    returning the last-seen status) -- same shape as `pollAtlasJob`. */
export async function pollRescueJob(
  vmName: string,
  jobName: string,
  opts?: { intervalMs?: number; timeoutMs?: number },
): Promise<RescueJobStatus> {
  const intervalMs = opts?.intervalMs ?? 1500
  const deadline = Date.now() + (opts?.timeoutMs ?? 60_000)
  for (;;) {
    const status = await getRescueJob(vmName, jobName)
    if (status.state === 'succeeded' || status.state === 'failed' || Date.now() >= deadline) {
      return status
    }
    await new Promise((r) => setTimeout(r, intervalMs))
  }
}
