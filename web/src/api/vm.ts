// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost, apiPostVoid, apiPut, apiPutVoid, apiDelete } from './client'

export interface PortForwardSpec {
  host_port: number
  guest_port: number
  protocol: 'tcp' | 'udp'
  /** Externally-reachable host for this forward (from ZORVIA_EXPOSE_HOST/HOST on the API pod). Only present on responses, never sent on create. */
  expose_host?: string
}

export interface VM {
  name: string
  state: 'running' | 'stopped' | 'paused' | 'starting' | 'stopping' | 'failed' | 'unknown'
  cpus: number
  memory: number
  image: string
  ip?: string
  pid?: number
  tags?: string[]
  port_forwards?: PortForwardSpec[]
  network_tap?: boolean
  network_static_ip?: boolean
  /** Why the VM ended up in the 'failed' state, if it did. Cleared on the next successful start. */
  last_error?: string
}

export interface CreateVMRequest {
  name: string
  image: string
  cpus: number
  memory: number
  /** Disk size in GB (daemon default: 20). */
  disk?: number
  port_forwards?: PortForwardSpec[]
  network_tap?: boolean
  network_static_ip?: boolean
  tenant?: string
  labels?: Record<string, string>
  guest_os?: 'linux' | 'windows'
  cloud_init?: {
    user_data?: string
    hostname?: string
    username?: string
    ssh_authorized_keys?: string[]
    password?: string
  }
  /** Install an in-guest agent through cloud-init (server default: none). */
  guest_agent?: 'zyvor' | 'qemu' | 'none'
  /** GPUs / host devices to pass through. Checked against the cluster before creation (422 with issues if not schedulable). */
  devices?: { name: string; device_name: string; kind?: 'gpu' | 'host-device' }[]
  expose_ssh?: boolean
  expose_vnc?: boolean
  expose_rdp?: boolean
  start?: boolean

  // ── Advanced (optional): full VM config surface ──
  cpu_max_sockets?: number
  memory_max_guest_mb?: number
  cpu_model?: string
  cpu_dedicated_placement?: boolean
  cpu_isolate_emulator_thread?: boolean
  /** Guest NUMA topology; needs cpu_dedicated_placement and memory_hugepages_page_size. */
  cpu_numa_passthrough?: boolean
  memory_hugepages_page_size?: string
  firmware?: CreateVMFirmware
  features?: CreateVMFeatures
  machine_type?: string
  enable_tpm?: boolean
  enable_rng?: boolean

  /** When present (non-empty), completely overrides `image`/`disk` -- the
   * server ignores the shorthand fields entirely once this is set. */
  disks?: CreateVMDisk[]
  /** When present (non-empty), completely overrides `network_tap` -- same
   * all-or-nothing behavior as `disks`. */
  interfaces?: CreateVMInterface[]
}

/** Matches the Rust `DiskSource` JSON shape (serde tag = "type"). */
export type CreateVMDiskSource =
  | { type: 'blank' }
  | { type: 'pvc'; name: string }
  | { type: 'containerDisk'; image: string }
  | { type: 'dataVolume'; name: string }

/** Matches the Rust `DiskConfig` JSON shape. */
export interface CreateVMDisk {
  name: string
  size: string
  boot_order: number
  source: CreateVMDiskSource
  bus?: string
}

/** Matches the Rust `NetworkType` JSON shape -- externally tagged (no
 * `#[serde(tag = ...)]`, unlike `DiskSource`): unit variants serialize as a
 * plain lowercase string, struct variants as `{ variantName: { ... } }`. */
export type CreateVMNetworkType =
  | 'bridge'
  | 'pod'
  | { multus: { name: string } }
  | { sriov: { name: string } }

/** Matches the Rust `InterfaceConfig` JSON shape. */
export interface CreateVMInterface {
  name: string
  network: string
  model?: string
  network_type: CreateVMNetworkType
}

/** Matches the Rust `FirmwareConfig`/`BootloaderType` JSON shape exactly. */
export type CreateVMFirmware =
  | { bootloader: 'bios' }
  | { bootloader: { efi: { secure_boot: boolean; persistent: boolean } } }

export interface CreateVMHyperV {
  relaxed?: boolean
  vapic?: boolean
  spinlocks?: number
  vpindex?: boolean
  runtime?: boolean
  synic?: boolean
  stimer?: boolean
  reset?: boolean
  frequencies?: boolean
  reenlightenment?: boolean
  tlbflush?: boolean
  ipi?: boolean
}

/** Matches the Rust `FeaturesConfig` JSON shape; every field is optional and
 * defaults server-side (ACPI defaults on, everything else off). */
export interface CreateVMFeatures {
  acpi?: boolean
  apic?: boolean
  hyperv?: CreateVMHyperV
  kvm_hidden?: boolean
  smm?: boolean
}

export interface VMMetricsPoint {
  ts: string
  cpu_usage: number
  memory_usage: number
  disk_usage: number
}

export interface VMMetrics {
  cpu_usage: number
  memory_usage: number
  disk_usage: number
  network_rx: number
  network_tx: number
  hostname?: string
  agent?: boolean
  source?: string
  phase?: string
  history?: VMMetricsPoint[]
}

const API_BASE = '/api'

export async function listVMs(): Promise<VM[]> {
  const data = await apiGet<VM[] | { items: VM[] }>(`${API_BASE}/vms`)
  return Array.isArray(data) ? data : data.items || []
}

export async function getVM(name: string): Promise<VM> {
  return apiGet<VM>(`${API_BASE}/vms/${name}`)
}

export async function createVM(req: CreateVMRequest): Promise<VM> {
  return apiPost<VM>(`${API_BASE}/vms`, req)
}

export async function deleteVM(name: string): Promise<void> {
  return apiDelete(`${API_BASE}/vms/${name}`)
}

export async function startVM(name: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${name}/start`)
}

export async function stopVM(name: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${name}/stop`)
}

export async function restartVM(name: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${name}/restart`)
}

export async function pauseVM(name: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${name}/pause`)
}

export async function resumeVM(name: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${name}/resume`)
}

export async function getMetrics(name: string): Promise<VMMetrics> {
  return apiGet<VMMetrics>(`${API_BASE}/vms/${name}/metrics`)
}

/** Mirrors crate::guest_insight::GuestAgentState (kebab-case on the wire). */
export type GuestAgentState = 'connected' | 'not-detected' | 'not-running' | 'unknown'

export interface GuestInterfaceInsight {
  name?: string
  guest_name?: string
  mac?: string
  primary_ip?: string
  ip_addresses: string[]
}

/** Mirrors crate::guest_insight::GuestInsightReport -- QEMU Guest Agent
 * status derived from KubeVirt VMI status, no guest shell access needed. */
export interface GuestInsightReport {
  vm: string
  namespace: string
  phase: string
  node?: string
  agent_state: GuestAgentState
  readiness_score: number
  os_name?: string
  os_id?: string
  os_version?: string
  kernel_release?: string
  interfaces: GuestInterfaceInsight[]
  recommendations: string[]
}

export async function getGuestInsight(name: string): Promise<GuestInsightReport> {
  return apiGet<GuestInsightReport>(`${API_BASE}/vms/${name}/guest-insight`)
}

/** KubeVirt's real `spec.template.spec.evictionStrategy` values -- there is
 * no vSphere-style lockstep fault tolerance on this platform; this is the
 * real, honest equivalent: what happens to the VM when its node drains. */
export type EvictionStrategy = 'LiveMigrate' | 'LiveMigrateIfPossible' | 'External' | 'None'

export interface HaPolicy {
  vm_name: string
  eviction_strategy: EvictionStrategy | null
}

export async function getHaPolicy(name: string): Promise<HaPolicy> {
  return apiGet<HaPolicy>(`${API_BASE}/vms/${name}/ha`)
}

export async function setHaPolicy(name: string, strategy: EvictionStrategy | null): Promise<HaPolicy> {
  return apiPut<HaPolicy>(`${API_BASE}/vms/${name}/ha`, { eviction_strategy: strategy })
}

export interface VMLogEntry {
  timestamp: string
  hostname: string
  unit: string
  message: string
  priority: string
}

export interface VMLogResponse {
  entries: VMLogEntry[]
  count: number
}

/** Real boot/runtime console output for this VM (FluxVM's captured
    console log), not the audit trail of API actions taken against it. */
export async function getVMLogs(name: string, opts?: { lines?: number; grep?: string }): Promise<VMLogResponse> {
  const params = new URLSearchParams()
  if (opts?.lines) params.set('lines', String(opts.lines))
  if (opts?.grep) params.set('grep', opts.grep)
  const qs = params.toString()
  return apiGet<VMLogResponse>(`${API_BASE}/vms/${name}/logs${qs ? `?${qs}` : ''}`)
}

/**
 * Expose a guest port by creating a Kubernetes NodePort Service selecting the
 * VM's virt-launcher pod. Takes effect immediately -- the VM/VMI itself is
 * never touched, running or not.
 */
export async function addPortForward(
  name: string,
  forward: { hostPort: number; guestPort: number; protocol?: 'tcp' | 'udp' },
): Promise<VM> {
  return apiPost<VM>(`${API_BASE}/vms/${name}/port-forwards`, {
    host_port: forward.hostPort,
    guest_port: forward.guestPort,
    protocol: forward.protocol ?? 'tcp',
  })
}

/**
 * Stop exposing a previously-forwarded guest port by deleting its NodePort
 * Service. Same as addPortForward: no VM/VMI restart involved.
 */
export async function removePortForward(name: string, hostPort: number): Promise<void> {
  return apiDelete(`${API_BASE}/vms/${name}/port-forwards/${hostPort}`)
}

export async function cloneVM(sourceName: string, targetName: string, options?: {
  includeSnapshots?: boolean
  linkedClone?: boolean
}): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${sourceName}/clone`, {
    target_name: targetName,
    include_snapshots: options?.includeSnapshots ?? false,
    linked_clone: options?.linkedClone ?? false,
  })
}

// Tag Management
export async function addTag(vmName: string, tag: string): Promise<void> {
  return apiPostVoid(`${API_BASE}/vms/${vmName}/tags`, { tag })
}

export async function removeTag(vmName: string, tag: string): Promise<void> {
  return apiDelete(`${API_BASE}/vms/${vmName}/tags/${tag}`)
}

export async function updateTags(vmName: string, tags: string[]): Promise<void> {
  return apiPutVoid(`${API_BASE}/vms/${vmName}/tags`, { tags })
}
