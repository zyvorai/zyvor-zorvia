// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from 'vitest'

// ─── Mock client helpers ────────────────────────────────────────────────────────

vi.mock('../client', () => ({
  apiGet: vi.fn().mockResolvedValue({}),
  apiPost: vi.fn().mockResolvedValue({}),
  apiPostVoid: vi.fn().mockResolvedValue(undefined),
  apiPut: vi.fn().mockResolvedValue({}),
  apiPutVoid: vi.fn().mockResolvedValue(undefined),
  apiDelete: vi.fn().mockResolvedValue(undefined),
  apiFetch: vi.fn().mockResolvedValue({
    ok: true,
    json: () => Promise.resolve({}),
    blob: () => Promise.resolve(new Blob()),
  }),
  getToken: vi.fn().mockReturnValue('test-token'),
  setToken: vi.fn(),
  clearToken: vi.fn(),
}))

// Mock global fetch for auth.ts login()
const fetchMock = vi.fn()
globalThis.fetch = fetchMock

import { apiGet, apiPost, apiPostVoid, apiPutVoid, apiDelete, apiFetch } from '../client'

const mockApiGet = vi.mocked(apiGet)
const mockApiPost = vi.mocked(apiPost)
const mockApiPostVoid = vi.mocked(apiPostVoid)
const mockApiPutVoid = vi.mocked(apiPutVoid)
const mockApiDelete = vi.mocked(apiDelete)
const mockApiFetch = vi.mocked(apiFetch)

beforeEach(() => {
  vi.clearAllMocks()
  fetchMock.mockReset()
})

// ─── auth.ts ──────────────────────────────────────────────────────────────────

describe('auth', () => {
  it('login sends POST with credentials via raw fetch', async () => {
    const { login } = await import('../auth')
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ token: 't', user_id: 'u', role: 'admin', username: 'admin' }),
    })

    const result = await login({ username: 'admin', password: 'pass' })

    expect(fetchMock).toHaveBeenCalledWith('/api/v1/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: 'admin', password: 'pass' }),
    })
    expect(result.token).toBe('t')
  })

  it('login throws on 401', async () => {
    const { login } = await import('../auth')
    fetchMock.mockResolvedValue({ ok: false, status: 401, json: () => Promise.resolve(null) })
    await expect(login({ username: 'x', password: 'y' })).rejects.toThrow('Invalid username or password')
  })

  it('login throws generic error on other failures', async () => {
    const { login } = await import('../auth')
    fetchMock.mockResolvedValue({ ok: false, status: 500, json: () => Promise.resolve(null) })
    await expect(login({ username: 'x', password: 'y' })).rejects.toThrow('Login failed')
  })

  it('getMe calls apiGet', async () => {
    const { getMe } = await import('../auth')
    await getMe()
    expect(mockApiGet).toHaveBeenCalledWith('/api/v1/auth/me')
  })
})

// ─── audit.ts ─────────────────────────────────────────────────────────────────

describe('audit', () => {
  it('listAuditLogs calls apiGet without filters', async () => {
    const { listAuditLogs } = await import('../audit')
    await listAuditLogs()
    expect(mockApiGet).toHaveBeenCalledWith('/api/audit/logs')
  })

  it('listAuditLogs calls apiGet with filters', async () => {
    const { listAuditLogs } = await import('../audit')
    await listAuditLogs({ action: 'create', user: 'admin' })
    expect(mockApiGet).toHaveBeenCalledWith(expect.stringContaining('/api/audit/logs?'))
  })

  it('exportAuditLogs uses apiFetch and returns blob', async () => {
    const { exportAuditLogs } = await import('../audit')
    const blob = new Blob(['csv'])
    mockApiFetch.mockResolvedValue({ ok: true, blob: () => Promise.resolve(blob) } as Response)

    const result = await exportAuditLogs(undefined, 'csv')
    expect(mockApiFetch).toHaveBeenCalledWith(expect.stringContaining('/api/audit/logs/export?'))
    expect(result).toBe(blob)
  })

  it('exportAuditLogs throws on failure', async () => {
    const { exportAuditLogs } = await import('../audit')
    mockApiFetch.mockResolvedValue({ ok: false } as Response)
    await expect(exportAuditLogs()).rejects.toThrow('Failed to export audit logs')
  })

  it('getAuditStats calls apiGet', async () => {
    const { getAuditStats } = await import('../audit')
    await getAuditStats()
    expect(mockApiGet).toHaveBeenCalledWith('/api/audit/stats')
  })
})

// ─── backup.ts ────────────────────────────────────────────────────────────────

describe('backup', () => {
  it('listBackups calls apiGet without vmName', async () => {
    const { listBackups } = await import('../backup')
    await listBackups()
    expect(mockApiGet).toHaveBeenCalledWith('/api/backups')
  })

  it('listBackups calls apiGet with vmName', async () => {
    const { listBackups } = await import('../backup')
    await listBackups('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/backups?vm=vm1')
  })

  it('createBackup calls apiPost', async () => {
    const { createBackup } = await import('../backup')
    const req = { vm_name: 'vm1', backup_type: 'full' as const }
    await createBackup(req)
    expect(mockApiPost).toHaveBeenCalledWith('/api/backups', req)
  })

  it('deleteBackup calls apiDelete', async () => {
    const { deleteBackup } = await import('../backup')
    await deleteBackup('b1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/backups/b1')
  })

  it('restoreBackup calls apiPost', async () => {
    const { restoreBackup } = await import('../backup')
    const opts = { backup_id: 'b1' }
    await restoreBackup(opts)
    expect(mockApiPost).toHaveBeenCalledWith('/api/backups/restore', opts)
  })

  it('getBackupJobs calls apiGet', async () => {
    const { getBackupJobs } = await import('../backup')
    await getBackupJobs()
    expect(mockApiGet).toHaveBeenCalledWith('/api/backups/jobs')
  })

  it('getBackupJob calls apiGet', async () => {
    const { getBackupJob } = await import('../backup')
    await getBackupJob('j1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/backups/jobs/j1')
  })

  it('listBackupPolicies calls apiGet', async () => {
    const { listBackupPolicies } = await import('../backup')
    await listBackupPolicies()
    expect(mockApiGet).toHaveBeenCalledWith('/api/backups/policies')
  })

  it('createBackupPolicy calls apiPost', async () => {
    const { createBackupPolicy } = await import('../backup')
    const policy = { name: 'daily', schedule_type: 'daily' as const, backup_type: 'full' as const, retention_days: 30, enabled: true }
    await createBackupPolicy(policy)
    expect(mockApiPost).toHaveBeenCalledWith('/api/backups/policies', policy)
  })

  it('deleteBackupPolicy calls apiDelete', async () => {
    const { deleteBackupPolicy } = await import('../backup')
    await deleteBackupPolicy('p1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/backups/policies/p1')
  })

  it('enableBackupPolicy calls apiPostVoid', async () => {
    const { enableBackupPolicy } = await import('../backup')
    await enableBackupPolicy('p1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/backups/policies/p1/enable')
  })

  it('snapshot schedule calls hit the /snapshot-schedules routes', async () => {
    const m = await import('../snapshots')
    await m.listVMSnapshotSchedules('web1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/web1/snapshot-schedules')
    const req = { name: 'nightly', schedule_type: 'daily' as const, vm_names: ['web1'] }
    await m.createSnapshotSchedule(req)
    expect(mockApiPost).toHaveBeenCalledWith('/api/snapshot-schedules', req)
    await m.deleteSnapshotSchedule('nightly')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/snapshot-schedules/nightly')
    await m.setSnapshotScheduleEnabled('nightly', true)
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/snapshot-schedules/nightly/enable')
    await m.setSnapshotScheduleEnabled('nightly', false)
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/snapshot-schedules/nightly/disable')
  })

  it('disableBackupPolicy calls apiPostVoid', async () => {
    const { disableBackupPolicy } = await import('../backup')
    await disableBackupPolicy('p1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/backups/policies/p1/disable')
  })

})

// ─── firmware.ts ──────────────────────────────────────────────────────────────

describe('firmware', () => {
  it('getFirmwareStatus calls apiGet', async () => {
    const { getFirmwareStatus } = await import('../firmware')
    await getFirmwareStatus('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/firmware/status')
  })

  it('enableUefi calls apiPostVoid', async () => {
    const { enableUefi } = await import('../firmware')
    await enableUefi('vm1', { secure_boot: true })
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/firmware/uefi', { secure_boot: true })
  })

  it('enableSecureBoot calls apiPostVoid', async () => {
    const { enableSecureBoot } = await import('../firmware')
    await enableSecureBoot('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/firmware/secureboot')
  })

  it('disableSecureBoot calls apiDelete', async () => {
    const { disableSecureBoot } = await import('../firmware')
    await disableSecureBoot('vm1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/vms/vm1/firmware/secureboot')
  })

  it('resetNvram calls apiPostVoid', async () => {
    const { resetNvram } = await import('../firmware')
    await resetNvram('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/firmware/reset')
  })

  it('getFirmwareCapabilities calls apiGet', async () => {
    const { getFirmwareCapabilities } = await import('../firmware')
    await getFirmwareCapabilities()
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/firmware/capabilities')
  })
})

// ─── hotplug.ts ───────────────────────────────────────────────────────────────

describe('hotplug', () => {
  it('hotplugCpu calls apiPost', async () => {
    const { hotplugCpu } = await import('../hotplug')
    await hotplugCpu('vm1', { count: 4 })
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/hotplug/cpu', { count: 4 })
  })

  it('hotplugMemory calls apiPost', async () => {
    const { hotplugMemory } = await import('../hotplug')
    await hotplugMemory('vm1', { size_mb: 2048 })
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/hotplug/memory', { size_mb: 2048 })
  })

  it('hotplugDisk calls apiPost', async () => {
    const { hotplugDisk } = await import('../hotplug')
    await hotplugDisk('vm1', { path: '/dev/sdb' })
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/hotplug/disk', { path: '/dev/sdb' })
  })

  it('hotremoveDisk uses apiFetch with DELETE', async () => {
    const { hotremoveDisk } = await import('../hotplug')
    mockApiFetch.mockResolvedValue({ ok: true } as Response)

    await hotremoveDisk('vm1', 'disk1')
    expect(mockApiFetch).toHaveBeenCalledWith('/api/vms/vm1/hotplug/disk/disk1', { method: 'DELETE' })
  })

  it('hotremoveDisk throws on failure', async () => {
    const { hotremoveDisk } = await import('../hotplug')
    mockApiFetch.mockResolvedValue({ ok: false } as Response)
    await expect(hotremoveDisk('vm1', 'disk1')).rejects.toThrow('Failed to hot-remove disk')
  })

  it('hotplugNic calls apiPost', async () => {
    const { hotplugNic } = await import('../hotplug')
    await hotplugNic('vm1', { bridge: 'br0' })
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/hotplug/nic', { bridge: 'br0' })
  })

  it('hotremoveNic uses apiFetch with DELETE', async () => {
    const { hotremoveNic } = await import('../hotplug')
    mockApiFetch.mockResolvedValue({ ok: true } as Response)

    await hotremoveNic('vm1', 'nic1')
    expect(mockApiFetch).toHaveBeenCalledWith('/api/vms/vm1/hotplug/nic/nic1', { method: 'DELETE' })
  })

  it('hotremoveNic throws on failure', async () => {
    const { hotremoveNic } = await import('../hotplug')
    mockApiFetch.mockResolvedValue({ ok: false } as Response)
    await expect(hotremoveNic('vm1', 'nic1')).rejects.toThrow('Failed to hot-remove NIC')
  })
})

// ─── images.ts ────────────────────────────────────────────────────────────────

describe('images', () => {
  it('listImages calls apiGet', async () => {
    const { listImages } = await import('../images')
    await listImages()
    expect(mockApiGet).toHaveBeenCalledWith('/api/images')
  })
})

// ─── migrations.ts ────────────────────────────────────────────────────────────

describe('migrations', () => {
  it('startMigration calls apiPost against the VM migrate route', async () => {
    const { startMigration } = await import('../migrations')
    await startMigration('vm1')
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/migrate')
  })

  it('listVmMigrations calls apiGet', async () => {
    const { listVmMigrations } = await import('../migrations')
    await listVmMigrations('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/migrations')
  })

  it('getMigration calls apiGet', async () => {
    const { getMigration } = await import('../migrations')
    await getMigration('m1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/migrations/m1')
  })

  it('cancelMigration calls apiPostVoid', async () => {
    const { cancelMigration } = await import('../migrations')
    await cancelMigration('m1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/migrations/m1/cancel')
  })
})

// ─── snapshots.ts ─────────────────────────────────────────────────────────────

describe('passthrough', () => {
  it('permit and unpermit use the admin permitted route', async () => {
    const { permitDevice, unpermitDevice, preflightDevices } = await import('../passthrough')
    const dev = { kind: 'pci' as const, resource_name: 'nvidia.com/A10', pci_vendor_selector: '10DE:2236' }
    await permitDevice(dev)
    expect(mockApiPost).toHaveBeenCalledWith('/api/v1/devices/permitted', dev)
    await unpermitDevice('nvidia.com/A10')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/v1/devices/permitted?resource_name=nvidia.com%2FA10')
    await preflightDevices([], [], 'ns', { dedicated_cpus: true, numa_passthrough: true, hugepages: '2Mi' })
    expect(mockApiPost).toHaveBeenCalledWith('/api/v1/devices/preflight', {
      devices: [],
      sriov_networks: [],
      namespace: 'ns',
      dedicated_cpus: true,
      numa_passthrough: true,
      hugepages: '2Mi',
    })
  })
})

describe('guestAgent', () => {
  it('getGuestAgent and getGuestInventory use the guest routes', async () => {
    const { getGuestAgent, getGuestInventory } = await import('../guestAgent')
    await getGuestAgent('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/guest/agent')
    await getGuestInventory('vm1', 'packages')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/guest/inventory/packages')
  })
})

describe('snapshots', () => {
  it('listSnapshots calls apiGet', async () => {
    const { listSnapshots } = await import('../snapshots')
    await listSnapshots('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/snapshots')
  })

  it('createSnapshot calls apiPost', async () => {
    const { createSnapshot } = await import('../snapshots')
    const req = { name: 'snap1' }
    await createSnapshot('vm1', req)
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/snapshots', req)
  })

  it('createSnapshotWithRetry succeeds on first try', async () => {
    const { createSnapshotWithRetry } = await import('../snapshots')
    const body = JSON.stringify({
      id: '1',
      vm_name: 'vm1',
      name: 'snap1',
      description: null,
      snapshot_type: 'Disk',
      parent_id: null,
      size_bytes: 0,
      created: '2026-01-01T00:00:00Z',
    })
    mockApiFetch.mockResolvedValueOnce({
      ok: true,
      status: 201,
      headers: { get: () => 'application/json' },
      text: async () => body,
    } as unknown as Response)
    const result = await createSnapshotWithRetry('vm1', { name: 'snap1' })
    expect(result.name).toBe('snap1')
  })

  it('createSnapshotWithRetry retries once on 409 then succeeds', async () => {
    const { createSnapshotWithRetry } = await import('../snapshots')
    const body = JSON.stringify({
      id: '1',
      vm_name: 'vm1',
      name: 'snap1',
      description: null,
      snapshot_type: 'Disk',
      parent_id: null,
      size_bytes: 0,
      created: '2026-01-01T00:00:00Z',
    })
    mockApiFetch
      .mockResolvedValueOnce({
        ok: false,
        status: 409,
        statusText: 'Conflict',
        text: async () => JSON.stringify({ error: 'could not reach the VM monitor yet' }),
      } as unknown as Response)
      .mockResolvedValueOnce({
        ok: true,
        status: 201,
        headers: { get: () => 'application/json' },
        text: async () => body,
      } as unknown as Response)
    const result = await createSnapshotWithRetry('vm1', { name: 'snap1' }, { delayMs: 1 })
    expect(result.name).toBe('snap1')
    expect(mockApiFetch).toHaveBeenCalledTimes(2)
  })

  it('getSnapshot calls apiGet', async () => {
    const { getSnapshot } = await import('../snapshots')
    await getSnapshot('vm1', 's1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/snapshots/s1')
  })

  it('deleteSnapshot calls apiDelete', async () => {
    const { deleteSnapshot } = await import('../snapshots')
    await deleteSnapshot('vm1', 's1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/vms/vm1/snapshots/s1')
  })

  it('revertSnapshot calls apiPostVoid', async () => {
    const { revertSnapshot } = await import('../snapshots')
    await revertSnapshot('vm1', 's1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/snapshots/s1/revert')
  })

})

// ─── vm.ts ────────────────────────────────────────────────────────────────────

describe('vm', () => {
  it('listVMs calls apiGet', async () => {
    const { listVMs } = await import('../vm')
    await listVMs()
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms')
  })

  it('getVM calls apiGet', async () => {
    const { getVM } = await import('../vm')
    await getVM('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1')
  })

  it('createVM calls apiPost', async () => {
    const { createVM } = await import('../vm')
    const req = { name: 'vm1', image: 'ubuntu.img', cpus: 2, memory: 2048 }
    await createVM(req)
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms', req)
  })

  it('deleteVM calls apiDelete', async () => {
    const { deleteVM } = await import('../vm')
    await deleteVM('vm1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/vms/vm1')
  })

  it('startVM calls apiPostVoid', async () => {
    const { startVM } = await import('../vm')
    await startVM('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/start')
  })

  it('stopVM calls apiPostVoid', async () => {
    const { stopVM } = await import('../vm')
    await stopVM('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/stop')
  })

  it('restartVM calls apiPostVoid', async () => {
    const { restartVM } = await import('../vm')
    await restartVM('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/restart')
  })

  it('pauseVM calls apiPostVoid', async () => {
    const { pauseVM } = await import('../vm')
    await pauseVM('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/pause')
  })

  it('resumeVM calls apiPostVoid', async () => {
    const { resumeVM } = await import('../vm')
    await resumeVM('vm1')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/resume')
  })

  it('getMetrics calls apiGet', async () => {
    const { getMetrics } = await import('../vm')
    await getMetrics('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/metrics')
  })

  it('cloneVM calls apiPostVoid with defaults', async () => {
    const { cloneVM } = await import('../vm')
    await cloneVM('vm1', 'vm1-clone')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/clone', {
      target_name: 'vm1-clone',
      include_snapshots: false,
      linked_clone: false,
    })
  })

  it('cloneVM calls apiPostVoid with options', async () => {
    const { cloneVM } = await import('../vm')
    await cloneVM('vm1', 'vm1-clone', { includeSnapshots: true, linkedClone: true })
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/clone', {
      target_name: 'vm1-clone',
      include_snapshots: true,
      linked_clone: true,
    })
  })

  it('addTag calls apiPostVoid', async () => {
    const { addTag } = await import('../vm')
    await addTag('vm1', 'prod')
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/tags', { tag: 'prod' })
  })

  it('removeTag calls apiDelete', async () => {
    const { removeTag } = await import('../vm')
    await removeTag('vm1', 'prod')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/vms/vm1/tags/prod')
  })

  it('updateTags calls apiPutVoid', async () => {
    const { updateTags } = await import('../vm')
    await updateTags('vm1', ['prod', 'web'])
    expect(mockApiPutVoid).toHaveBeenCalledWith('/api/vms/vm1/tags', { tags: ['prod', 'web'] })
  })
})

// ─── system.ts ────────────────────────────────────────────────────────────────

describe('system', () => {
  it('getCpuTopology calls apiGet', async () => {
    const { getCpuTopology } = await import('../system')
    await getCpuTopology()
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/cpu/topology')
  })

  it('getNumaTopology calls apiGet', async () => {
    const { getNumaTopology } = await import('../system')
    await getNumaTopology()
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/numa/topology')
  })

  it('getNumaNode calls apiGet', async () => {
    const { getNumaNode } = await import('../system')
    await getNumaNode(0)
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/numa/nodes/0')
  })

  it('getNumaPlacement calls apiGet', async () => {
    const { getNumaPlacement } = await import('../system')
    await getNumaPlacement(4096, 4)
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/numa/placement?memory_mb=4096&cpus=4')
  })

  it('setCpuPinning calls apiPostVoid', async () => {
    const { setCpuPinning } = await import('../system')
    await setCpuPinning('vm1', { pinning: { type: 'Auto' } })
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/cpu/pin', { pinning: { type: 'Auto' } })
  })

  it('removeCpuPinning calls apiDelete', async () => {
    const { removeCpuPinning } = await import('../system')
    await removeCpuPinning('vm1')
    expect(mockApiDelete).toHaveBeenCalledWith('/api/vms/vm1/cpu/pin')
  })

  it('getCpuAffinity calls apiGet', async () => {
    const { getCpuAffinity } = await import('../system')
    await getCpuAffinity('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/cpu/affinity')
  })

  it('setMemoryLimit calls apiPutVoid', async () => {
    const { setMemoryLimit } = await import('../system')
    await setMemoryLimit('vm1', { limit_bytes: 4294967296 })
    expect(mockApiPutVoid).toHaveBeenCalledWith('/api/vms/vm1/memory/limit', { limit_bytes: 4294967296 })
  })

  it('getMemoryUsage calls apiGet', async () => {
    const { getMemoryUsage } = await import('../system')
    await getMemoryUsage('vm1')
    expect(mockApiGet).toHaveBeenCalledWith('/api/vms/vm1/memory/usage')
  })

  it('setMemoryBallooning calls apiPostVoid', async () => {
    const { setMemoryBallooning } = await import('../system')
    await setMemoryBallooning('vm1', true)
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/vms/vm1/memory/balloon', { enabled: true })
  })

  it('getHugepageStats calls apiGet', async () => {
    const { getHugepageStats } = await import('../system')
    await getHugepageStats('Size2MB')
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/memory/hugepages?size=Size2MB')
  })

  it('allocateHugepages calls apiPostVoid', async () => {
    const { allocateHugepages } = await import('../system')
    await allocateHugepages({ size: 'Size1GB', count: 4 })
    expect(mockApiPostVoid).toHaveBeenCalledWith('/api/system/memory/hugepages', { size: 'Size1GB', count: 4 })
  })

  it('getSystemMemory calls apiGet', async () => {
    const { getSystemMemory } = await import('../system')
    await getSystemMemory()
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/memory')
  })

  it('getOptimizationRecommendations calls apiGet', async () => {
    const { getOptimizationRecommendations } = await import('../system')
    await getOptimizationRecommendations()
    expect(mockApiGet).toHaveBeenCalledWith('/api/system/optimization/recommendations')
  })

  it('optimizeVM calls apiPost', async () => {
    const { optimizeVM } = await import('../system')
    await optimizeVM('vm1')
    expect(mockApiPost).toHaveBeenCalledWith('/api/vms/vm1/optimize')
  })
})
