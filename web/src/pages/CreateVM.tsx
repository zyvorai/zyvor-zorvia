// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router'
import { createVM, listVMs } from '../api/vm'
import { getDeviceInventory, suggestedResources } from '../api/passthrough'
import type { CreateVMDisk, CreateVMFeatures, CreateVMFirmware, CreateVMInterface, PortForwardSpec, VM } from '../api/vm'
import { listImages, createImageFromVm, getConvertJob, listCloudImages, downloadCloudImage, listDownloads } from '../api/images'
import type { ImageInfo, CloudImage } from '../api/images'
import { getKrytonStatus, getKrytonImages, createKrytonMachine } from '../api/kryton'
import type { KrytonStatus, KrytonImage } from '../api/kryton'
import { formatBytes } from '../utils/format'
import {
  ArrowLeft, ArrowRight, Cpu, HardDrive, ChevronDown, ChevronUp, Shield, Plus, X,
  Network, Server, Sparkles, Check, Download, MonitorCog,
} from 'lucide-react'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, WizardStepper } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

interface DiskRow {
  name: string
  size: string
  sourceType: 'blank' | 'pvc' | 'containerDisk' | 'dataVolume'
  sourceValue: string
  bus: string
}

interface NicRow {
  name: string
  network: string
  networkType: 'pod' | 'bridge' | 'multus' | 'sriov'
  networkTypeValue: string
}

interface AdvancedOptions {
  firmware: 'bios' | 'uefi'
  secureBoot: boolean
  cpuMode: 'host-passthrough' | 'host-model' | 'custom'
  cpuModelName: string
  dedicatedCpuPlacement: boolean
  numaPassthrough: boolean
  hugepages: '' | '2Mi' | '1Gi'
  machineType: string
  enableTpm: boolean
  enableRng: boolean
  windowsHyperv: boolean
}

const defaultAdvanced: AdvancedOptions = {
  firmware: 'uefi',
  secureBoot: false,
  cpuMode: 'host-passthrough',
  cpuModelName: '',
  dedicatedCpuPlacement: false,
  numaPassthrough: false,
  hugepages: '',
  machineType: 'q35',
  enableTpm: false,
  enableRng: true,
  windowsHyperv: true,
}

/** Maps wizard advanced state onto the create request's VMConfig-shaped
 * fields. These are applied when the VM is created (KubeVirt requires
 * firmware/CPU model/machine type to be set upfront, not patched in later). */
function buildAdvancedCreateFields(adv: AdvancedOptions, guestOs: 'linux' | 'windows') {
  const firmware: CreateVMFirmware =
    adv.firmware === 'uefi'
      ? { bootloader: { efi: { secure_boot: adv.secureBoot, persistent: true } } }
      : { bootloader: 'bios' }

  const cpu_model =
    adv.cpuMode === 'custom' ? adv.cpuModelName.trim() || undefined : adv.cpuMode

  let features: CreateVMFeatures | undefined
  if (adv.firmware === 'uefi' && adv.secureBoot) {
    features = { ...features, smm: true }
  }
  if (guestOs === 'windows' && adv.windowsHyperv) {
    features = {
      ...features,
      hyperv: { relaxed: true, vapic: true, spinlocks: 8191, synic: true, stimer: true, vpindex: true },
    }
  }

  return {
    firmware,
    ...(cpu_model ? { cpu_model } : {}),
    ...(adv.dedicatedCpuPlacement ? { cpu_dedicated_placement: true } : {}),
    ...(adv.numaPassthrough ? { cpu_numa_passthrough: true } : {}),
    ...(adv.hugepages ? { memory_hugepages_page_size: adv.hugepages } : {}),
    ...(adv.machineType.trim() ? { machine_type: adv.machineType.trim() } : {}),
    ...(adv.enableTpm ? { enable_tpm: true } : {}),
    ...(adv.enableRng ? { enable_rng: true } : {}),
    ...(features ? { features } : {}),
  }
}

const WIZARD_STEPS = ['Basics', 'Resources', 'Review'] as const
const VM_NAME_REGEX = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/

export default function CreateVM() {
  const navigate = useNavigate()
  const toast = useToastContext()
  const [name, setName] = useState('')
  const [image, setImage] = useState('')
  const [cpus, setCpus] = useState(2)
  const [memory, setMemory] = useState(2048)
  const [diskGb, setDiskGb] = useState(20)
  const [imageOptions, setImageOptions] = useState<ImageInfo[]>([])
  const [imagesLoading, setImagesLoading] = useState(false)
  const [loading, setLoading] = useState(false)
  const [validationError, setValidationError] = useState('')
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [imagesError, setImagesError] = useState<string | null>(null)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [advanced, setAdvanced] = useState<AdvancedOptions>(defaultAdvanced)
  const [wizardStep, setWizardStep] = useState(0)
  const [imagesReload, setImagesReload] = useState(0)
  const [showGoldenImage, setShowGoldenImage] = useState(false)
  const [showDownloadImage, setShowDownloadImage] = useState(false)
  const [networkMode, setNetworkMode] = useState<'nat' | 'bridged'>('nat')
  const [staticIp, setStaticIp] = useState(false)
  const [tenant, setTenant] = useState('')
  const [portForwards, setPortForwards] = useState<{ hostPort: string; guestPort: string; protocol: 'tcp' | 'udp' }[]>([])
  const [extraDisks, setExtraDisks] = useState<DiskRow[]>([])
  const [extraNics, setExtraNics] = useState<NicRow[]>([])
  const [devices, setDevices] = useState<{ name: string; deviceName: string; kind: 'gpu' | 'host-device' }[]>([])
  const [deviceOptions, setDeviceOptions] = useState<string[]>([])
  const [guestOs, setGuestOs] = useState<'linux' | 'windows'>('linux')
  const [ciHostname, setCiHostname] = useState('')
  const [ciUsername, setCiUsername] = useState('zorvia')
  const [ciPassword, setCiPassword] = useState('')
  const [ciSshKeys, setCiSshKeys] = useState('')
  const [ciUserData, setCiUserData] = useState('')
  const [guestAgent, setGuestAgent] = useState<'default' | 'zyvor' | 'qemu' | 'none'>('default')
  const [exposeSsh, setExposeSsh] = useState(true)
  const [exposeVnc, setExposeVnc] = useState(false)
  const [exposeRdp, setExposeRdp] = useState(false)
  const [krytonStatus, setKrytonStatus] = useState<KrytonStatus | null>(null)
  const [krytonImages, setKrytonImages] = useState<KrytonImage[]>([])
  const [krytonLoading, setKrytonLoading] = useState(false)

  const addPortForwardRow = (guestPort = '', hostPort = '') => {
    setPortForwards((rows) => [...rows, { hostPort, guestPort, protocol: 'tcp' }])
  }
  const updatePortForwardRow = (index: number, patch: Partial<{ hostPort: string; guestPort: string; protocol: 'tcp' | 'udp' }>) => {
    setPortForwards((rows) => rows.map((r, i) => (i === index ? { ...r, ...patch } : r)))
  }
  const removePortForwardRow = (index: number) => {
    setPortForwards((rows) => rows.filter((_, i) => i !== index))
  }

  const addDiskRow = () => {
    setExtraDisks((rows) => [
      ...rows,
      { name: `disk${rows.length + 2}`, size: '20Gi', sourceType: 'blank', sourceValue: '', bus: 'virtio' },
    ])
  }
  const updateDiskRow = (index: number, patch: Partial<DiskRow>) => {
    setExtraDisks((rows) => rows.map((r, i) => (i === index ? { ...r, ...patch } : r)))
  }
  const removeDiskRow = (index: number) => {
    setExtraDisks((rows) => rows.filter((_, i) => i !== index))
  }

  const addNicRow = () => {
    setExtraNics((rows) => [
      ...rows,
      { name: `nic${rows.length + 2}`, network: '', networkType: 'pod', networkTypeValue: '' },
    ])
  }
  const updateNicRow = (index: number, patch: Partial<NicRow>) => {
    setExtraNics((rows) => rows.map((r, i) => (i === index ? { ...r, ...patch } : r)))
  }
  const removeNicRow = (index: number) => {
    setExtraNics((rows) => rows.filter((_, i) => i !== index))
  }

  /** Builds the full disks/interfaces arrays for the create request. Only
   * called when the user has added at least one extra row -- since sending
   * these arrays makes the server ignore `image`/`disk`/`network_tap`
   * entirely, the primary disk/NIC (today's single-disk/single-NIC fields)
   * must be synthesized as the first entry or it would silently vanish. */
  const buildDisksField = (): CreateVMDisk[] => [
    { name: 'rootdisk', size: `${diskGb}Gi`, boot_order: 1, source: { type: 'blank' } },
    ...extraDisks.map((row, i) => ({
      name: row.name.trim() || `disk${i + 2}`,
      size: row.size.trim() || '20Gi',
      boot_order: i + 2,
      bus: row.bus || undefined,
      source:
        row.sourceType === 'pvc'
          ? { type: 'pvc' as const, name: row.sourceValue.trim() }
          : row.sourceType === 'containerDisk'
            ? { type: 'containerDisk' as const, image: row.sourceValue.trim() }
            : row.sourceType === 'dataVolume'
              ? { type: 'dataVolume' as const, name: row.sourceValue.trim() }
              : { type: 'blank' as const },
    })),
  ]

  const buildInterfacesField = (): CreateVMInterface[] => [
    {
      name: 'default',
      network: 'default',
      network_type: networkMode === 'bridged' ? 'bridge' : 'pod',
    },
    ...extraNics.map((row, i) => ({
      name: row.name.trim() || `nic${i + 2}`,
      network: row.network.trim() || `nic${i + 2}`,
      network_type:
        row.networkType === 'multus'
          ? { multus: { name: row.networkTypeValue.trim() } }
          : row.networkType === 'sriov'
            ? { sriov: { name: row.networkTypeValue.trim() } }
            : row.networkType === 'bridge'
              ? ('bridge' as const)
              : ('pod' as const),
    })),
  ]

  useEffect(() => {
    if (wizardStep !== 0) return
    void imagesReload
    let cancelled = false
    setImagesLoading(true)
    setImagesError(null)
    listImages()
      .then((data) => {
        if (cancelled) return
        setImageOptions((Array.isArray(data) ? data : []).filter((i) => i.path))
      })
      .catch((err) => {
        if (!cancelled) {
          setImageOptions([])
          const msg = formatUserError(err)
          setImagesError(msg)
          toastFailure(toast, 'Could not load disk images', err)
        }
      })
      .finally(() => {
        if (!cancelled) setImagesLoading(false)
      })
    return () => { cancelled = true }
  }, [wizardStep, imagesReload, toast])

  useEffect(() => {
    if (wizardStep !== 0 || guestOs !== 'windows') return
    let cancelled = false
    setKrytonLoading(true)
    getKrytonStatus()
      .then(async (status) => {
        if (cancelled) return
        setKrytonStatus(status)
        if (!status.enabled || !status.connected || !status.project) {
          setKrytonImages([])
          return
        }
        const page = await getKrytonImages()
        if (cancelled) return
        setKrytonImages(page.items.filter((img) => img.ready))
      })
      .catch((err) => {
        if (!cancelled) {
          setKrytonStatus({ enabled: false, connected: false })
          setKrytonImages([])
          toastFailure(toast, 'Could not reach Kryton', err)
        }
      })
      .finally(() => {
        if (!cancelled) setKrytonLoading(false)
      })
    return () => { cancelled = true }
  }, [wizardStep, guestOs, toast])

  const memoryPresets = [
    { label: '512 MB', value: 512 },
    { label: '1 GB', value: 1024 },
    { label: '2 GB', value: 2048 },
    { label: '4 GB', value: 4096 },
    { label: '8 GB', value: 8192 },
    { label: '16 GB', value: 16384 },
  ]

  const validateStep = (step: number): string | null => {
    if (step === 0) {
      if (!name.trim()) return 'VM name is required'
      if (!VM_NAME_REGEX.test(name)) {
        return 'VM name must start with a letter or number, use only letters, numbers, dots, hyphens, underscores, and be 1–64 characters'
      }
      if (!image.trim()) return 'Image path is required'
    }
    if (step === 1) {
      if (cpus < 1 || cpus > 32) return 'vCPUs must be between 1 and 32'
      if (memory < 256) return 'Memory must be at least 256 MB'
      const hostPorts = new Set<string>()
      for (const row of networkMode === 'nat' ? portForwards : []) {
        const h = parseInt(row.hostPort)
        const g = parseInt(row.guestPort)
        // host 0 = auto NodePort on Zorvia
        if (row.hostPort !== '0' && (!row.hostPort || !Number.isInteger(h) || h < 1 || h > 65535)) {
          return 'Each port forward needs a host port between 1 and 65535 (or 0 for auto)'
        }
        if (!row.guestPort || !Number.isInteger(g) || g < 1 || g > 65535) {
          return 'Each port forward needs a guest port between 1 and 65535'
        }
        if (row.hostPort !== '0') {
          if (hostPorts.has(row.hostPort)) {
            return `Host port ${row.hostPort} is used by more than one port forward`
          }
          hostPorts.add(row.hostPort)
        }
      }
      for (const row of extraDisks) {
        if (!row.name.trim()) return 'Each additional disk needs a name'
        if (row.sourceType !== 'blank' && !row.sourceValue.trim()) {
          return `Disk '${row.name.trim() || '?'}' needs a ${row.sourceType === 'containerDisk' ? 'container image' : 'PVC/DataVolume name'}`
        }
      }
      for (const row of extraNics) {
        if (!row.name.trim()) return 'Each additional NIC needs a name'
        if ((row.networkType === 'multus' || row.networkType === 'sriov') && !row.networkTypeValue.trim()) {
          return `NIC '${row.name.trim() || '?'}' needs a network attachment name`
        }
      }
    }
    return null
  }

  const goNext = () => {
    const msg = validateStep(wizardStep)
    if (msg) {
      setValidationError(msg)
      return
    }
    setValidationError('')
    setWizardStep((s) => Math.min(s + 1, WIZARD_STEPS.length - 1))
  }

  const goBack = () => {
    setValidationError('')
    setWizardStep((s) => Math.max(s - 1, 0))
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    const msg = validateStep(0) || validateStep(1)
    if (msg) {
      setValidationError(msg)
      return
    }

    setLoading(true)
    setValidationError('')
    setSubmitError(null)

    if (guestOs === 'windows') {
      // Kryton is the Windows virtualization control plane (see
      // docs/KRYTON_INTEGRATION.md) — Windows VMs are created there, not via
      // the KubeVirt-backed /api/vms path Linux uses.
      if (!krytonStatus?.project) {
        setSubmitError('Kryton is not configured (KRYTON_PROJECT missing) — see the Windows page for status.')
        setLoading(false)
        return
      }
      try {
        await createKrytonMachine({
          project: krytonStatus.project,
          name,
          image,
          compute: { cpu: cpus, memoryMiB: memory },
          disk: { sizeGiB: diskGb },
        })
        toast.success(`Windows machine '${name}' created in Kryton`)
        navigate('/app/windows')
      } catch (err) {
        const msg = formatUserError(err)
        setSubmitError(msg)
        toastFailure(toast, 'Failed to create Windows machine', err)
      } finally {
        setLoading(false)
      }
      return
    }

    try {
      const port_forwards: PortForwardSpec[] = networkMode === 'nat'
        ? portForwards.map((row) => ({
            host_port: parseInt(row.hostPort) || 0,
            guest_port: parseInt(row.guestPort),
            protocol: row.protocol,
          }))
        : []
      const sshKeys = ciSshKeys
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean)
      // guestOs is narrowed to 'linux' here — Windows returns above via Kryton.
      await createVM({
        name,
        image,
        cpus,
        memory,
        disk: diskGb,
        guest_os: guestOs,
        network_tap: networkMode === 'bridged',
        network_static_ip: networkMode === 'bridged' && staticIp,
        ...(guestOs === 'linux' && guestAgent !== 'default' ? { guest_agent: guestAgent } : {}),
        expose_ssh: exposeSsh,
        expose_vnc: exposeVnc,
        expose_rdp: exposeRdp,
        ...(tenant.trim() ? { tenant: tenant.trim() } : {}),
        ...(port_forwards.length ? { port_forwards } : {}),
        ...(extraDisks.length ? { disks: buildDisksField() } : {}),
        ...(extraNics.length ? { interfaces: buildInterfacesField() } : {}),
        ...(devices.some((d) => d.name.trim() && d.deviceName.trim())
          ? {
              devices: devices
                .filter((d) => d.name.trim() && d.deviceName.trim())
                .map((d) => ({ name: d.name.trim(), device_name: d.deviceName.trim(), kind: d.kind })),
            }
          : {}),
        cloud_init: {
          hostname: ciHostname.trim() || name,
          username: ciUsername.trim() || 'zorvia',
          ...(ciPassword ? { password: ciPassword } : {}),
          ...(sshKeys.length ? { ssh_authorized_keys: sshKeys } : {}),
          ...(ciUserData.trim() ? { user_data: ciUserData } : {}),
        },
        ...(showAdvanced ? buildAdvancedCreateFields(advanced, guestOs) : {}),
      })
      toast.success(`VM '${name}' created`)
      navigate(`/app/vms/${name}`)
    } catch (err) {
      const msg = formatUserError(err)
      setSubmitError(msg)
      toastFailure(toast, 'Failed to create VM', err)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div>
      <button
        onClick={() => navigate('/app/vms')}
        className="flex items-center gap-2 mb-6 text-[var(--zf-muted)] hover:text-[var(--zf-ink)] transition-colors text-sm"
      >
        <ArrowLeft className="w-4 h-4" />
        Back to VMs
      </button>

      <div className="w-full">
        <PageHeader
          title="Create Virtual Machine"
          description="Configure and launch a new VM"
          icon={Server}
        />

        <WizardStepper
          steps={WIZARD_STEPS}
          current={wizardStep}
          onStep={(step) => {
            if (step < wizardStep) {
              setWizardStep(step)
              setValidationError('')
            }
          }}
        />

        {imagesError && wizardStep === 0 && (
          <ErrorBanner
            title="Could not load disk images"
            headline={imagesError}
            hints={hintsForError(imagesError, 'storage')}
            onRetry={() => setImagesReload((n) => n + 1)}
          />
        )}

        {submitError && (
          <ErrorBanner
            title="Could not create virtual machine"
            headline={submitError}
            hints={hintsForError(submitError, 'vm')}
          />
        )}

        <form onSubmit={handleSubmit} className="space-y-4 mt-4">
          {validationError && (
            <div className="p-3 bg-[var(--zf-danger)]/10 border border-[var(--zf-danger)]/25 rounded-lg text-[var(--zf-danger)] text-sm">
              {validationError}
            </div>
          )}

          {wizardStep === 0 && (
            <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-6 space-y-5">
              <h2 className="text-sm font-medium text-[var(--zf-muted)] uppercase tracking-wider">Basic Configuration</h2>

              <div>
                <label htmlFor="vm-name" className="block text-sm font-medium text-[var(--zf-ink)] mb-1.5">
                  VM Name
                </label>
                <input
                  id="vm-name"
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="my-virtual-machine"
                  className="w-full px-3.5 py-2.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-[var(--zf-ink)] placeholder-[var(--zf-muted)] focus:outline-none focus:border-[var(--zf-link)]/50 focus:ring-1 focus:ring-[var(--zf-link)]/20 transition-colors text-sm"
                  required
                  autoFocus
                />
              </div>

              <div>
                <span className="block text-sm font-medium text-[var(--zf-ink)] mb-1.5">Guest OS</span>
                <div className="flex gap-2">
                  {(['linux', 'windows'] as const).map((os) => (
                    <button
                      key={os}
                      type="button"
                      onClick={() => {
                        setGuestOs(os)
                        setImage('')
                        if (os === 'windows') {
                          setExposeSsh(false)
                          setExposeVnc(true)
                        } else {
                          setExposeSsh(true)
                          setExposeVnc(false)
                          setExposeRdp(false)
                        }
                      }}
                      className={`px-4 py-2 rounded-lg text-sm font-medium border transition-colors ${
                        guestOs === os
                          ? 'bg-[var(--zf-ink)] text-[var(--zf-canvas)] border-[var(--zf-ink)]'
                          : 'bg-[var(--zf-surface)] text-[var(--zf-ink)] border-[var(--zf-hairline)]'
                      }`}
                    >
                      {os === 'linux' ? 'Linux' : 'Windows'}
                    </button>
                  ))}
                </div>
              </div>

              {guestOs === 'linux' && (
                <div className="rounded-lg border border-[var(--zf-hairline)] p-4 space-y-3 bg-[var(--zf-surface)]">
                  <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Cloud-init</h3>
                  <div className="grid sm:grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs text-[var(--zf-muted)] mb-1">Hostname</label>
                      <input
                        value={ciHostname}
                        onChange={(e) => setCiHostname(e.target.value)}
                        placeholder={name || 'hostname'}
                        className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm"
                      />
                    </div>
                    <div>
                      <label className="block text-xs text-[var(--zf-muted)] mb-1">Username</label>
                      <input
                        value={ciUsername}
                        onChange={(e) => setCiUsername(e.target.value)}
                        className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm"
                      />
                    </div>
                  </div>
                  <div>
                    <label className="block text-xs text-[var(--zf-muted)] mb-1">Password (optional)</label>
                    <input
                      type="password"
                      value={ciPassword}
                      onChange={(e) => setCiPassword(e.target.value)}
                      className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-[var(--zf-muted)] mb-1">SSH authorized keys (one per line)</label>
                    <textarea
                      value={ciSshKeys}
                      onChange={(e) => setCiSshKeys(e.target.value)}
                      rows={3}
                      className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm font-mono"
                      placeholder="ssh-ed25519 AAAA…"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-[var(--zf-muted)] mb-1">Custom user-data YAML (optional)</label>
                    <textarea
                      value={ciUserData}
                      onChange={(e) => setCiUserData(e.target.value)}
                      rows={4}
                      className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm font-mono"
                      placeholder="#cloud-config"
                    />
                  </div>
                </div>
              )}

              {guestOs === 'linux' && (
                <div className="rounded-lg border border-[var(--zf-hairline)] p-4 space-y-2 bg-[var(--zf-surface)]">
                  <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Guest agent</h3>
                  <select
                    value={guestAgent}
                    onChange={(e) => setGuestAgent(e.target.value as typeof guestAgent)}
                    className="w-full px-3 py-2 border border-[var(--zf-hairline)] rounded-lg text-sm"
                    aria-label="Guest agent"
                  >
                    <option value="default">Server default</option>
                    <option value="zyvor">Zyvor guest agent (GuestKit; Debian/Ubuntu guests)</option>
                    <option value="qemu">qemu-guest-agent (package install)</option>
                    <option value="none">None</option>
                  </select>
                  <p className="text-xs text-[var(--zf-muted)]">
                    Installed by cloud-init on first boot. An agent gives consistent online snapshots (the guest filesystem is
                    frozen around them) and lets recovery drills see inside the guest. The Zyvor agent is downloaded by the guest
                    and its SHA-256 is verified.
                  </p>
                </div>
              )}

              <div className="rounded-lg border border-[var(--zf-hairline)] p-4 space-y-2 bg-[var(--zf-surface)]">
                <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Expose</h3>
                {guestOs === 'linux' && (
                  <label className="flex items-center gap-2 text-sm">
                    <input type="checkbox" checked={exposeSsh} onChange={(e) => setExposeSsh(e.target.checked)} />
                    Expose SSH (guest 22 → NodePort)
                  </label>
                )}
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={exposeVnc} onChange={(e) => setExposeVnc(e.target.checked)} />
                  Expose VNC (guest 5900 → NodePort)
                </label>
                {guestOs === 'linux' && (
                  <label className="flex items-center gap-2 text-sm">
                    <input type="checkbox" checked={exposeRdp} onChange={(e) => setExposeRdp(e.target.checked)} />
                    Expose RDP (guest 3389 → NodePort)
                  </label>
                )}
              </div>

              <div>
                <label htmlFor="vm-tenant" className="block text-sm font-medium text-[var(--zf-ink)] mb-1.5">
                  Tenant <span className="text-[var(--zf-muted)] font-normal">(optional)</span>
                </label>
                <input
                  id="vm-tenant"
                  type="text"
                  value={tenant}
                  onChange={(e) => setTenant(e.target.value)}
                  placeholder="acme"
                  className="w-full px-3.5 py-2.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-[var(--zf-ink)] placeholder-[var(--zf-muted)] focus:outline-none focus:border-[var(--zf-link)]/50 focus:ring-1 focus:ring-[var(--zf-link)]/20 transition-colors text-sm"
                />
              </div>

            {guestOs === 'windows' ? (
              <div>
                <label className="block text-sm font-medium text-[var(--zf-ink)] mb-1.5">
                  Windows golden image
                </label>
                {krytonLoading ? (
                  <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 mb-2">
                    {[0, 1, 2].map((i) => (
                      <div key={i} className="h-[4.25rem] rounded-lg bg-[var(--zf-canvas)] animate-pulse" />
                    ))}
                  </div>
                ) : !krytonStatus?.enabled || !krytonStatus?.connected ? (
                  <p className="text-xs text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/25 rounded-lg px-3 py-2">
                    Kryton (the Windows control plane) is not reachable from this Zorvia server. Configure{' '}
                    <code className="font-mono">KRYTON_URL</code> and see the{' '}
                    <a href="/app/windows" className="underline">Windows page</a> for status.
                  </p>
                ) : !krytonStatus.project ? (
                  <p className="text-xs text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/25 rounded-lg px-3 py-2">
                    Kryton is reachable, but <code className="font-mono">KRYTON_PROJECT</code> is not configured on
                    the Zorvia server — Windows VM creation is blocked until it is.
                  </p>
                ) : krytonImages.length === 0 ? (
                  <p className="text-xs text-[var(--zf-muted)] mb-2">
                    No certified Windows golden images are available in this Kryton project.
                  </p>
                ) : (
                  <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 mb-2">
                    {krytonImages.map((img) => {
                      const selected = image === img.id
                      return (
                        <button
                          key={img.id}
                          type="button"
                          onClick={() => setImage(img.id)}
                          className={`relative text-left p-3 rounded-lg border transition-colors ${
                            selected
                              ? 'bg-[var(--zf-link)]/15 border-[var(--zf-link)]/40 ring-1 ring-[var(--zf-link)]/30'
                              : 'bg-[var(--zf-surface)] border-[var(--zf-hairline)] hover:border-[var(--zf-hairline)]'
                          }`}
                        >
                          {selected && (
                            <div className="absolute top-2 right-2 w-4 h-4 rounded-full bg-[var(--zf-link)] flex items-center justify-center">
                              <Check className="w-2.5 h-2.5 text-[var(--zf-ink)]" />
                            </div>
                          )}
                          <div className="flex items-center gap-2 mb-1.5">
                            <div className="icon-tile icon-tile-sm icon-tile-blue shrink-0">
                              <MonitorCog className="w-3.5 h-3.5" />
                            </div>
                            <span className="text-sm font-medium text-[var(--zf-ink)] truncate">{img.name}</span>
                          </div>
                          <p className="text-xs text-[var(--zf-muted)]">
                            {img.version}
                            {img.certified ? ' · certified' : ''}
                          </p>
                        </button>
                      )
                    })}
                  </div>
                )}
              </div>
            ) : (
            <div>
              <div className="flex items-center justify-between mb-1.5 gap-3 flex-wrap">
                <label htmlFor="vm-image" className="block text-sm font-medium text-[var(--zf-ink)]">
                  Disk image
                </label>
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    onClick={() => setShowDownloadImage(true)}
                    className="flex items-center gap-1.5 text-xs font-medium text-[var(--zf-link)] hover:text-[var(--zf-link-hover)] transition-colors shrink-0"
                  >
                    <Download className="w-3.5 h-3.5" />
                    Download an OS image
                  </button>
                  <button
                    type="button"
                    onClick={() => setShowGoldenImage(true)}
                    className="flex items-center gap-1.5 text-xs font-medium text-[var(--zf-link)] hover:text-[var(--zf-link-hover)] transition-colors shrink-0"
                  >
                    <Sparkles className="w-3.5 h-3.5" />
                    Create golden image from a VM
                  </button>
                </div>
              </div>

              {imagesLoading ? (
                <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 mb-2">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="h-[4.25rem] rounded-lg bg-[var(--zf-canvas)] animate-pulse" />
                  ))}
                </div>
              ) : imageOptions.length > 0 ? (
                <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 mb-2">
                  {imageOptions.map((opt) => {
                    const selected = image === opt.path
                    return (
                      <button
                        key={opt.path}
                        type="button"
                        onClick={() => setImage(opt.path)}
                        className={`relative text-left p-3 rounded-lg border transition-colors ${
                          selected
                            ? 'bg-[var(--zf-link)]/15 border-[var(--zf-link)]/40 ring-1 ring-[var(--zf-link)]/30'
                            : 'bg-[var(--zf-surface)] border-[var(--zf-hairline)] hover:border-[var(--zf-hairline)]'
                        }`}
                      >
                        {selected && (
                          <div className="absolute top-2 right-2 w-4 h-4 rounded-full bg-[var(--zf-link)] flex items-center justify-center">
                            <Check className="w-2.5 h-2.5 text-[var(--zf-ink)]" />
                          </div>
                        )}
                        <div className="flex items-center gap-2 mb-1.5">
                          <div className="icon-tile icon-tile-sm icon-tile-blue shrink-0">
                            <HardDrive className="w-3.5 h-3.5" />
                          </div>
                          <span className="text-sm font-medium text-[var(--zf-ink)] truncate">{opt.name}</span>
                        </div>
                        <p className="text-xs text-[var(--zf-muted)]">
                          {opt.format.toUpperCase()} · {formatBytes(opt.size_bytes)}
                        </p>
                      </button>
                    )
                  })}
                </div>
              ) : (
                <p className="text-xs text-[var(--zf-muted)] mb-2">
                  No catalog images found — enter a path below, or create a golden image from an existing VM.
                </p>
              )}

              <input
                id="vm-image"
                type="text"
                value={image}
                onChange={(e) => setImage(e.target.value)}
                placeholder="Or enter a path on the host, e.g. /var/lib/zyvor-fabricd/images/ubuntu-24.04.qcow2"
                className="w-full px-3.5 py-2.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-[var(--zf-ink)] placeholder-[var(--zf-muted)] focus:outline-none focus:border-[var(--zf-link)]/50 focus:ring-1 focus:ring-[var(--zf-link)]/20 transition-colors text-sm font-mono"
                required
              />
            </div>
            )}
            </div>
          )}

          {showGoldenImage && (
            <GoldenImageModal
              onClose={() => setShowGoldenImage(false)}
              onCreated={(path) => {
                setImage(path)
                setShowGoldenImage(false)
                setImagesReload((n) => n + 1)
              }}
            />
          )}

          {showDownloadImage && (
            <DownloadImageModal
              existingImages={imageOptions}
              onClose={() => setShowDownloadImage(false)}
              onDownloaded={(path) => {
                setImage(path)
                setShowDownloadImage(false)
                setImagesReload((n) => n + 1)
              }}
            />
          )}

          {wizardStep === 1 && (
            <>
              <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-6 space-y-5">
                <h2 className="text-sm font-medium text-[var(--zf-muted)] uppercase tracking-wider">Resources</h2>

                <div>
                  <label htmlFor="vm-cpus" className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                    <Cpu className="w-4 h-4 text-[var(--zf-muted)]" />
                    vCPUs
                  </label>
                  <div className="flex items-center gap-3">
                    <input
                      id="vm-cpus"
                      type="range"
                      min={1}
                      max={32}
                      value={cpus}
                      onChange={(e) => setCpus(parseInt(e.target.value))}
                      className="flex-1 accent-[var(--zf-link)]"
                    />
                    <div className="w-16 text-center">
                      <input
                        type="number"
                        value={cpus}
                        onChange={(e) => setCpus(Math.max(1, Math.min(32, parseInt(e.target.value) || 1)))}
                        min={1}
                        max={32}
                        className="w-full px-2 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-center text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                      />
                    </div>
                  </div>
                </div>

                <div>
                  <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                    <HardDrive className="w-4 h-4 text-[var(--zf-muted)]" />
                    Memory
                  </label>
                  <div className="grid grid-cols-3 sm:grid-cols-6 gap-2 mb-3">
                    {memoryPresets.map((preset) => (
                      <button
                        key={preset.value}
                        type="button"
                        onClick={() => setMemory(preset.value)}
                        className={`px-3 py-2 rounded-lg text-sm font-medium transition-colors ${
                          memory === preset.value
                            ? 'bg-[var(--zf-link)]/20 text-[var(--zf-link)] border border-[var(--zf-link)]/30'
                            : 'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-muted)] hover:text-[var(--zf-ink)] hover:border-[var(--zf-hairline)]'
                        }`}
                      >
                        {preset.label}
                      </button>
                    ))}
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      id="vm-memory"
                      type="number"
                      value={memory}
                      onChange={(e) => setMemory(parseInt(e.target.value) || 512)}
                      min={256}
                      step={256}
                      className="w-28 px-3 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                    />
                    <span className="text-sm text-[var(--zf-muted)]">MB</span>
                    <span className="text-sm text-[var(--zf-muted)] ml-2">
                      ({(memory / 1024).toFixed(1)} GB)
                    </span>
                  </div>
                </div>
              </div>

              <div>
              <label htmlFor="vm-disk" className="block text-sm font-medium text-[var(--zf-ink)] mb-1.5">
                Root disk (GB)
              </label>
              <input
                id="vm-disk"
                type="number"
                min={1}
                max={2048}
                value={diskGb}
                onChange={(e) => setDiskGb(Math.max(1, parseInt(e.target.value) || 20))}
                className="w-28 px-3 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
              />
            </div>

            {guestOs === 'linux' && (
              <div>
                <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                  <HardDrive className="w-4 h-4 text-[var(--zf-muted)]" />
                  Additional disks
                </label>
                <div className="space-y-3">
                  {extraDisks.map((row, i) => (
                    <div key={i} className="flex flex-wrap items-center gap-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg p-3">
                      <input
                        type="text"
                        value={row.name}
                        onChange={(e) => updateDiskRow(i, { name: e.target.value })}
                        placeholder="Name"
                        className="w-28 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                      />
                      <input
                        type="text"
                        value={row.size}
                        onChange={(e) => updateDiskRow(i, { size: e.target.value })}
                        placeholder="Size (e.g. 20Gi)"
                        className="w-28 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                      />
                      <select
                        value={row.sourceType}
                        onChange={(e) => updateDiskRow(i, { sourceType: e.target.value as DiskRow['sourceType'], sourceValue: '' })}
                        className="px-2 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                      >
                        <option value="blank">Blank</option>
                        <option value="pvc">Existing PVC</option>
                        <option value="dataVolume">Existing DataVolume</option>
                        <option value="containerDisk">Container image</option>
                      </select>
                      {row.sourceType !== 'blank' && (
                        <input
                          type="text"
                          value={row.sourceValue}
                          onChange={(e) => updateDiskRow(i, { sourceValue: e.target.value })}
                          placeholder={row.sourceType === 'containerDisk' ? 'quay.io/...' : 'PVC/DataVolume name'}
                          className="flex-1 min-w-[10rem] px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                        />
                      )}
                      <button
                        type="button"
                        onClick={() => removeDiskRow(i)}
                        className="p-1.5 rounded-md text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 transition-colors"
                        title="Remove"
                      >
                        <X className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
                <button
                  type="button"
                  onClick={addDiskRow}
                  className="mt-2 flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-ink)] hover:border-[var(--zf-hairline)] transition-colors"
                >
                  <Plus className="w-3.5 h-3.5" />
                  Add disk
                </button>
              </div>
            )}

              <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
            <button
              type="button"
              onClick={() => setShowAdvanced(!showAdvanced)}
              className="w-full flex items-center justify-between px-6 py-4 text-sm font-medium text-[var(--zf-muted)] hover:text-[var(--zf-ink)] transition-colors"
            >
              <span className="uppercase tracking-wider">Advanced Options</span>
                  {showAdvanced ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                </button>

            {showAdvanced && (
              <div className="px-6 pb-6 space-y-5 border-t border-[var(--zf-hairline)] pt-5">
                <p className="text-xs zf-chip rounded-lg px-3 py-2">
                  Applied when the VM is created — KubeVirt requires firmware, CPU model, and machine type to be set upfront.
                </p>
                <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Shield className="w-4 h-4 text-[var(--zf-muted)]" />
                        Firmware
                      </label>
                      <div className="flex gap-2">
                        {(['bios', 'uefi'] as const).map((fw) => (
                          <button
                            key={fw}
                            type="button"
                            onClick={() =>
                              setAdvanced({
                                ...advanced,
                                firmware: fw,
                                secureBoot: fw === 'bios' ? false : advanced.secureBoot,
                              })
                            }
                            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                              advanced.firmware === fw
                                ? 'bg-[var(--zf-link)]/20 text-[var(--zf-link)] border border-[var(--zf-link)]/30'
                                : 'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-muted)] hover:text-[var(--zf-ink)]'
                            }`}
                          >
                            {fw.toUpperCase()}
                          </button>
                        ))}
                      </div>
                      {advanced.firmware === 'uefi' && (
                        <label className="flex items-center gap-2 mt-3 text-sm text-[var(--zf-ink)]">
                          <input
                            type="checkbox"
                            checked={advanced.secureBoot}
                            onChange={(e) => setAdvanced({ ...advanced, secureBoot: e.target.checked })}
                          />
                          Secure Boot
                        </label>
                      )}
                    </div>

                    <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Cpu className="w-4 h-4 text-[var(--zf-muted)]" />
                        CPU model
                      </label>
                      <div className="flex gap-2 flex-wrap">
                        {(['host-passthrough', 'host-model', 'custom'] as const).map((mode) => (
                          <button
                            key={mode}
                            type="button"
                            onClick={() => setAdvanced({ ...advanced, cpuMode: mode })}
                            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                              advanced.cpuMode === mode
                                ? 'bg-[var(--zf-link)]/20 text-[var(--zf-link)] border border-[var(--zf-link)]/30'
                                : 'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-muted)] hover:text-[var(--zf-ink)]'
                            }`}
                          >
                            {mode}
                          </button>
                        ))}
                      </div>
                      {advanced.cpuMode === 'custom' && (
                        <input
                          type="text"
                          value={advanced.cpuModelName}
                          onChange={(e) => setAdvanced({ ...advanced, cpuModelName: e.target.value })}
                          placeholder="e.g. Haswell, EPYC-Rome"
                          className="mt-2 w-full px-3 py-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-sm"
                        />
                      )}
                      <label className="flex items-center gap-2 mt-3 text-sm text-[var(--zf-ink)]">
                        <input
                          type="checkbox"
                          checked={advanced.dedicatedCpuPlacement}
                          onChange={(e) => setAdvanced({ ...advanced, dedicatedCpuPlacement: e.target.checked })}
                        />
                        Dedicated CPU placement (pin vCPUs to physical cores)
                      </label>
                      <label className="flex items-center gap-2 mt-2 text-sm text-[var(--zf-ink)]">
                        <input
                          type="checkbox"
                          checked={advanced.numaPassthrough}
                          onChange={(e) =>
                            setAdvanced({
                              ...advanced,
                              numaPassthrough: e.target.checked,
                              // KubeVirt refuses NUMA passthrough without both of these.
                              ...(e.target.checked
                                ? { dedicatedCpuPlacement: true, hugepages: advanced.hugepages || '2Mi' }
                                : {}),
                            })
                          }
                        />
                        Guest NUMA topology (needs dedicated CPUs and hugepages; typical for GPU workloads)
                      </label>
                      <label className="flex items-center gap-2 mt-2 text-sm text-[var(--zf-ink)]">
                        Hugepages
                        <select
                          value={advanced.hugepages}
                          onChange={(e) => setAdvanced({ ...advanced, hugepages: e.target.value as AdvancedOptions['hugepages'] })}
                          className="px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-sm"
                          aria-label="Hugepage size"
                        >
                          <option value="">none</option>
                          <option value="2Mi">2Mi</option>
                          <option value="1Gi">1Gi</option>
                        </select>
                      </label>
                    </div>

                    <div>
                      <label className="block text-sm font-medium text-[var(--zf-ink)] mb-2">Machine type</label>
                      <input
                        type="text"
                        value={advanced.machineType}
                        onChange={(e) => setAdvanced({ ...advanced, machineType: e.target.value })}
                        placeholder="q35"
                        className="w-40 px-3 py-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg text-sm font-mono"
                      />
                    </div>

                    <div className="space-y-2">
                      <label className="flex items-center gap-2 text-sm text-[var(--zf-ink)]">
                        <input
                          type="checkbox"
                          checked={advanced.enableTpm}
                          onChange={(e) => setAdvanced({ ...advanced, enableTpm: e.target.checked })}
                        />
                        Attach TPM 2.0 device
                      </label>
                      <label className="flex items-center gap-2 text-sm text-[var(--zf-ink)]">
                        <input
                          type="checkbox"
                          checked={advanced.enableRng}
                          onChange={(e) => setAdvanced({ ...advanced, enableRng: e.target.checked })}
                        />
                        Attach virtio-rng device
                      </label>
                      {guestOs === 'windows' && (
                        <label className="flex items-center gap-2 text-sm text-[var(--zf-ink)]">
                          <input
                            type="checkbox"
                            checked={advanced.windowsHyperv}
                            onChange={(e) => setAdvanced({ ...advanced, windowsHyperv: e.target.checked })}
                          />
                          Enable HyperV enlightenments (recommended for Windows performance)
                        </label>
                      )}
                    </div>

                    <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Network className="w-4 h-4 text-[var(--zf-muted)]" />
                        Networking
                      </label>
                      <div className="flex gap-2">
                        <button
                          type="button"
                          onClick={() => setNetworkMode('nat')}
                          className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                            networkMode === 'nat'
                              ? 'bg-[var(--zf-link)]/20 text-[var(--zf-link)] border border-[var(--zf-link)]/30'
                              : 'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-muted)] hover:text-[var(--zf-ink)]'
                          }`}
                        >
                          NAT (default)
                        </button>
                        <button
                          type="button"
                          onClick={() => setNetworkMode('bridged')}
                          className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                            networkMode === 'bridged'
                              ? 'bg-[var(--zf-link)]/20 text-[var(--zf-link)] border border-[var(--zf-link)]/30'
                              : 'bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-muted)] hover:text-[var(--zf-ink)]'
                          }`}
                        >
                          Bridged (DHCP)
                        </button>
                      </div>
                      <p className="text-xs text-[var(--zf-muted)] mt-2">
                        {networkMode === 'nat'
                          ? 'No host-routable IP — reach anything inside this VM (like SSH) by forwarding a port below.'
                          : 'This VM gets its own real IP (visible on its Network tab once booted) — no port forwards needed.'}
                      </p>
                      {networkMode === 'bridged' && (
                        <label className="flex items-center gap-2 mt-3 text-sm text-[var(--zf-ink)]">
                          <input
                            type="checkbox"
                            checked={staticIp}
                            onChange={(e) => setStaticIp(e.target.checked)}
                            className="rounded border-[var(--zf-hairline)] bg-[var(--zf-surface)] text-[var(--zf-link)] focus:ring-[var(--zf-link)]/50"
                          />
                          Assign the IP statically via cloud-init
                        </label>
                      )}
                      <p className="text-xs text-[var(--zf-muted)] mt-1">
                        {networkMode === 'bridged' && staticIp
                          ? 'The address is configured directly at boot — no dependency on the guest running a working DHCP client, but the image must support cloud-init.'
                          : networkMode === 'bridged'
                            ? "The guest's own DHCP client requests the address — works without cloud-init, but only if the image actually runs one automatically on boot."
                            : ''}
                      </p>
                    </div>

                    {networkMode === 'nat' && (
                    <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Network className="w-4 h-4 text-[var(--zf-muted)]" />
                        Expose ports (host port → guest port)
                      </label>
                      <p className="text-xs text-[var(--zf-muted)] mb-2">
                        This VM uses NAT networking with no host-routable IP — a port must be forwarded here to
                        reach anything inside it (like SSH) from outside the host.
                      </p>
                      {portForwards.length === 0 && (
                        <button
                          type="button"
                          onClick={() => addPortForwardRow('22')}
                          className="mb-2 px-3 py-1.5 rounded-lg text-xs font-medium bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-ink)] hover:text-[var(--zf-ink)] hover:border-[var(--zf-hairline)] transition-colors"
                        >
                          + Expose SSH (22)
                        </button>
                      )}
                      <div className="space-y-2">
                        {portForwards.map((row, i) => (
                          <div key={i} className="flex items-center gap-2">
                            <input
                              type="number"
                              value={row.hostPort}
                              onChange={(e) => updatePortForwardRow(i, { hostPort: e.target.value })}
                              placeholder="Host port"
                              min={1}
                              max={65535}
                              className="w-28 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                            />
                            <span className="text-[var(--zf-muted)] text-sm">→</span>
                            <input
                              type="number"
                              value={row.guestPort}
                              onChange={(e) => updatePortForwardRow(i, { guestPort: e.target.value })}
                              placeholder="Guest port"
                              min={1}
                              max={65535}
                              className="w-28 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                            />
                            <select
                              value={row.protocol}
                              onChange={(e) => updatePortForwardRow(i, { protocol: e.target.value as 'tcp' | 'udp' })}
                              className="px-2 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                            >
                              <option value="tcp">TCP</option>
                              <option value="udp">UDP</option>
                            </select>
                            <button
                              type="button"
                              onClick={() => removePortForwardRow(i)}
                              className="p-1.5 rounded-md text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 transition-colors"
                              title="Remove"
                            >
                              <X className="w-3.5 h-3.5" />
                            </button>
                          </div>
                        ))}
                      </div>
                      <button
                        type="button"
                        onClick={() => addPortForwardRow()}
                        className="mt-2 flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-ink)] hover:text-[var(--zf-ink)] hover:border-[var(--zf-hairline)] transition-colors"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        Add port forward
                      </button>
                    </div>
                    )}

                    <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Cpu className="w-4 h-4 text-[var(--zf-muted)]" />
                        Passthrough devices (GPU / host device)
                      </label>
                      <div className="space-y-3">
                        {devices.map((row, i) => (
                          <div key={i} className="flex flex-wrap items-center gap-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg p-3">
                            <input
                              type="text"
                              value={row.name}
                              onChange={(e) => setDevices((d) => d.map((r, j) => (j === i ? { ...r, name: e.target.value } : r)))}
                              placeholder="Name (e.g. gpu0)"
                              aria-label="Device name"
                              className="w-32 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                            />
                            <input
                              type="text"
                              list="zf-device-resources"
                              value={row.deviceName}
                              onChange={(e) => setDevices((d) => d.map((r, j) => (j === i ? { ...r, deviceName: e.target.value } : r)))}
                              placeholder="Resource (vendor.com/name)"
                              aria-label="Device resource"
                              className="flex-1 min-w-[12rem] px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                            />
                            <select
                              value={row.kind}
                              onChange={(e) => setDevices((d) => d.map((r, j) => (j === i ? { ...r, kind: e.target.value as 'gpu' | 'host-device' } : r)))}
                              aria-label="Device kind"
                              className="px-2 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                            >
                              <option value="gpu">GPU</option>
                              <option value="host-device">Host device</option>
                            </select>
                            <button
                              type="button"
                              onClick={() => setDevices((d) => d.filter((_, j) => j !== i))}
                              aria-label="Remove device"
                              className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-ink)]"
                            >
                              <X className="w-4 h-4" />
                            </button>
                          </div>
                        ))}
                        <datalist id="zf-device-resources">
                          {deviceOptions.map((o) => (
                            <option key={o} value={o} />
                          ))}
                        </datalist>
                        <button
                          type="button"
                          onClick={() => {
                            setDevices((d) => [...d, { name: `dev${d.length}`, deviceName: '', kind: 'gpu' }])
                            if (!deviceOptions.length) {
                              getDeviceInventory()
                                .then((inv) => setDeviceOptions(suggestedResources(inv)))
                                .catch(() => {})
                            }
                          }}
                          className="inline-flex items-center gap-1.5 text-sm text-[var(--zf-link)] hover:underline"
                        >
                          <Plus className="w-4 h-4" /> Add device
                        </button>
                        <p className="text-xs text-[var(--zf-muted)]">
                          The device must be advertised by a node and permitted in the KubeVirt CR; creation is refused with the reason
                          otherwise. A VM with a passthrough device cannot be live-migrated.
                        </p>
                      </div>
                    </div>

                    <div>
                      <label className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)] mb-2">
                        <Network className="w-4 h-4 text-[var(--zf-muted)]" />
                        Additional NICs
                      </label>
                      <div className="space-y-3">
                        {extraNics.map((row, i) => (
                          <div key={i} className="flex flex-wrap items-center gap-2 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-lg p-3">
                            <input
                              type="text"
                              value={row.name}
                              onChange={(e) => updateNicRow(i, { name: e.target.value })}
                              placeholder="Name"
                              className="w-28 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                            />
                            <input
                              type="text"
                              value={row.network}
                              onChange={(e) => updateNicRow(i, { network: e.target.value })}
                              placeholder="Network"
                              className="w-32 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                            />
                            <select
                              value={row.networkType}
                              onChange={(e) => updateNicRow(i, { networkType: e.target.value as NicRow['networkType'], networkTypeValue: '' })}
                              className="px-2 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)]"
                            >
                              <option value="pod">Pod</option>
                              <option value="bridge">Bridge</option>
                              <option value="multus">Multus</option>
                              <option value="sriov">SR-IOV</option>
                            </select>
                            {(row.networkType === 'multus' || row.networkType === 'sriov') && (
                              <input
                                type="text"
                                value={row.networkTypeValue}
                                onChange={(e) => updateNicRow(i, { networkTypeValue: e.target.value })}
                                placeholder="Network attachment name"
                                className="flex-1 min-w-[10rem] px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm text-[var(--zf-ink)] focus:outline-none focus:border-[var(--zf-link)]/50"
                              />
                            )}
                            <button
                              type="button"
                              onClick={() => removeNicRow(i)}
                              className="p-1.5 rounded-md text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 transition-colors"
                              title="Remove"
                            >
                              <X className="w-3.5 h-3.5" />
                            </button>
                          </div>
                        ))}
                      </div>
                      <button
                        type="button"
                        onClick={addNicRow}
                        className="mt-2 flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-[var(--zf-surface)] border border-[var(--zf-hairline)] text-[var(--zf-ink)] hover:border-[var(--zf-hairline)] transition-colors"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        Add NIC
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </>
          )}

          {wizardStep === 2 && (
            <div className="bg-[var(--zf-canvas)] rounded-xl border border-[var(--zf-hairline)] p-6 space-y-4">
              <h2 className="text-sm font-medium text-[var(--zf-muted)] uppercase tracking-wider">Review</h2>
              <dl className="grid grid-cols-1 gap-3 text-sm">
                <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                  <dt className="text-[var(--zf-muted)]">Name</dt>
                  <dd className="text-[var(--zf-ink)] font-medium">{name}</dd>
                </div>
                {tenant.trim() && (
                  <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                    <dt className="text-[var(--zf-muted)]">Tenant</dt>
                    <dd className="text-[var(--zf-ink)] font-medium">{tenant.trim()}</dd>
                  </div>
                )}
                <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                  <dt className="text-[var(--zf-muted)]">{guestOs === 'windows' ? 'Windows image' : 'Image'}</dt>
                  <dd className="text-[var(--zf-ink)] font-mono text-xs text-right break-all">
                    {guestOs === 'windows' ? krytonImages.find((i) => i.id === image)?.name ?? image : image}
                  </dd>
                </div>
                {guestOs === 'windows' && (
                  <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                    <dt className="text-[var(--zf-muted)]">Provisioned by</dt>
                    <dd className="text-[var(--zf-ink)]">Kryton{krytonStatus?.project ? ` (project: ${krytonStatus.project})` : ''}</dd>
                  </div>
                )}
                <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                  <dt className="text-[var(--zf-muted)]">vCPUs</dt>
                  <dd className="text-[var(--zf-ink)]">{cpus}</dd>
                </div>
                <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                  <dt className="text-[var(--zf-muted)]">Memory</dt>
                  <dd className="text-[var(--zf-ink)]">
                    {memory} MB ({(memory / 1024).toFixed(1)} GB)
                  </dd>
                </div>
                <div className={`flex justify-between gap-4 ${guestOs === 'linux' ? 'border-b border-[var(--zf-hairline)] pb-2' : ''}`}>
                  <dt className="text-[var(--zf-muted)]">Root disk</dt>
                  <dd className="text-[var(--zf-ink)]">
                    {diskGb} GB{extraDisks.length ? ` + ${extraDisks.length} more` : ''}
                  </dd>
                </div>
                {guestOs === 'linux' && extraNics.length > 0 && (
                  <div className="flex justify-between gap-4 border-b border-[var(--zf-hairline)] pb-2">
                    <dt className="text-[var(--zf-muted)]">Additional NICs</dt>
                    <dd className="text-[var(--zf-ink)]">{extraNics.length}</dd>
                  </div>
                )}
                {guestOs === 'linux' && (
                  <div className={`flex justify-between gap-4 ${networkMode === 'nat' && portForwards.length ? 'border-b border-[var(--zf-hairline)] pb-2' : ''}`}>
                    <dt className="text-[var(--zf-muted)]">Networking</dt>
                    <dd className="text-[var(--zf-ink)]">
                      {networkMode === 'nat' ? 'NAT' : staticIp ? 'Bridged (static IP)' : 'Bridged (DHCP)'}
                    </dd>
                  </div>
                )}
                {guestOs === 'linux' && networkMode === 'nat' && portForwards.length > 0 && (
                  <div className="flex justify-between gap-4">
                    <dt className="text-[var(--zf-muted)]">Exposed ports</dt>
                    <dd className="text-[var(--zf-ink)] text-right">
                      {portForwards.map((row, i) => (
                        <div key={i} className="font-mono text-xs">
                          {row.hostPort} → {row.guestPort}/{row.protocol}
                        </div>
                      ))}
                    </dd>
                  </div>
                )}
                {guestOs === 'linux' && showAdvanced && (
                  <div className="flex justify-between gap-4 border-t border-[var(--zf-hairline)] pt-2">
                    <dt className="text-[var(--zf-muted)]">Advanced</dt>
                    <dd className="text-[var(--zf-ink)] text-right text-xs space-y-0.5">
                      <div>
                        {advanced.firmware.toUpperCase()}
                        {advanced.firmware === 'uefi' && advanced.secureBoot ? ' + Secure Boot' : ''}
                      </div>
                      <div>
                        CPU: {advanced.cpuMode === 'custom' ? advanced.cpuModelName.trim() || 'custom' : advanced.cpuMode}
                        {advanced.dedicatedCpuPlacement ? ' (dedicated)' : ''}
                      </div>
                      {advanced.machineType.trim() && <div>Machine: {advanced.machineType.trim()}</div>}
                      {(advanced.enableTpm || advanced.enableRng) && (
                        <div>
                          {[advanced.enableTpm && 'TPM', advanced.enableRng && 'RNG'].filter(Boolean).join(' + ')}
                        </div>
                      )}
                    </dd>
                  </div>
                )}
              </dl>
            </div>
          )}

          <div className="flex gap-3 pt-2">
            {wizardStep > 0 && (
              <button
                type="button"
                onClick={goBack}
                className="flex-1 zf-btn zf-btn-ghost"
              >
                <ArrowLeft className="w-4 h-4" />
                Back
              </button>
            )}
            {wizardStep < WIZARD_STEPS.length - 1 ? (
              // Distinct keys matter: without them React reuses this DOM node for the submit
              // button below, flips it to type="submit" mid-click, and the browser then submits
              // the form on "Next" -- creating the VM before the Review step is ever shown.
              <button
                key="wizard-next"
                type="button"
                onClick={goNext}
                className="flex-1 zf-btn zf-btn-primary"
              >
                Next
                <ArrowRight className="w-4 h-4" />
              </button>
            ) : (
              <button
                key="wizard-create"
                type="submit"
                disabled={loading || !name || !image}
                className="flex-1 zf-btn zf-btn-primary"
              >
                {loading ? (
                  <>
                    <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Creating...
                  </>
                ) : (
                  'Create Virtual Machine'
                )}
              </button>
            )}
          </div>
        </form>
      </div>
    </div>
  )
}

function GoldenImageModal({ onClose, onCreated }: { onClose: () => void; onCreated: (path: string) => void }) {
  const toast = useToastContext()
  const [vms, setVms] = useState<VM[]>([])
  const [vmsLoading, setVmsLoading] = useState(true)
  const [vmName, setVmName] = useState('')
  const [name, setName] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [progress, setProgress] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    listVMs()
      .then((data) => {
        if (cancelled) return
        setVms(data)
        setVmName((prev) => prev || data[0]?.name || '')
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setVmsLoading(false)
      })
    return () => { cancelled = true }
  }, [])

  const handleSubmit = async () => {
    if (!VM_NAME_REGEX.test(name)) {
      setError('Image name must start with a letter or number, use only letters, numbers, dots, hyphens, underscores, and be 1–64 characters')
      return
    }
    setError(null)
    setSubmitting(true)
    setProgress(0)
    try {
      const { job_id } = await createImageFromVm(vmName, name)
      // Poll until the qemu-img convert job (backend: /images/from-vm/:name)
      // reaches a terminal state -- disk conversion can take a while for
      // large or busy VMs, so this isn't a fire-and-forget POST.
      let settledTicks = 0
      for (;;) {
        await new Promise((r) => setTimeout(r, 1200))
        const job = await getConvertJob(job_id)
        setProgress(job.progress)
        if (job.status === 'completed') {
          // The backend certifies the image with GuestKit's offline `doctor`
          // scoring right after conversion finishes, as a short follow-up
          // step -- give it a few extra ticks to attach before finalizing,
          // but don't block forever (e.g. guestkit missing in dev).
          if (job.boot_score === undefined && settledTicks < 4) {
            settledTicks += 1
            continue
          }
          const scoreNote = job.boot_score !== undefined
            ? ` — boot readiness ${Math.round(job.boot_score)}/100`
            : ''
          toast.success(`Golden image '${name}' created from '${vmName}'${scoreNote}`)
          onCreated(job.output_path)
          return
        }
        if (job.status === 'failed') {
          setError(job.error || 'Image conversion failed')
          setSubmitting(false)
          setProgress(null)
          return
        }
      }
    } catch (err) {
      toastFailure(toast, 'Failed to start golden image build', err)
      setError(formatUserError(err))
      setSubmitting(false)
      setProgress(null)
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <div className="flex items-center gap-3">
        <div className="icon-tile icon-tile-md icon-tile-purple">
          <Sparkles className="w-5 h-5" />
        </div>
        <div>
          <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Create Golden Image</h3>
          <p className="text-xs text-[var(--zf-muted)]">Materialize a VM's current disk as a reusable catalog image</p>
        </div>
      </div>

      {error && (
        <div className="p-3 bg-[var(--zf-danger)]/10 border border-[var(--zf-danger)]/25 rounded-lg text-[var(--zf-danger)] text-sm">
          {error}
        </div>
      )}

      {vmsLoading ? (
        <div className="h-10 rounded-lg bg-[var(--zf-canvas)] animate-pulse" />
      ) : vms.length === 0 ? (
        <p className="text-sm text-[var(--zf-muted)]">No VMs exist yet — create a VM first, then come back here to save its disk as a golden image.</p>
      ) : (
        <>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Source VM</label>
              <select
                value={vmName}
                onChange={(e) => setVmName(e.target.value)}
                disabled={submitting}
                className="input-field text-sm"
              >
                {vms.map((v) => (
                  <option key={v.name} value={v.name}>{v.name} ({v.state})</option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Image Name</label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && name && vmName && !submitting) handleSubmit()
                }}
                placeholder="e.g. web-server-golden"
                disabled={submitting}
                className="input-field text-sm font-mono"
                required
                autoFocus
              />
            </div>
          </div>
          <p className="text-sm text-[var(--zf-muted)]">
            The image is an independent copy — the source VM can change or be deleted afterward without affecting it.
          </p>
          {submitting && progress !== null && (
            <div>
              <div className="h-1.5 rounded-full bg-[var(--zf-canvas-alt)] overflow-hidden">
                <div
                  className="h-full bg-[var(--zf-link)] transition-all duration-500"
                  style={{ width: `${Math.max(5, progress)}%` }}
                />
              </div>
              <p className="text-xs text-[var(--zf-muted)] mt-1.5">Converting disk image…</p>
            </div>
          )}
          <div className="flex gap-2">
            <button
              type="button"
              onClick={handleSubmit}
              disabled={submitting || !name || !vmName}
              className="zf-btn zf-btn-primary zf-btn-sm"
            >
              {submitting && <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />}
              {submitting ? 'Creating…' : 'Create Image'}
            </button>
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="zf-btn zf-btn-ghost zf-btn-sm"
            >
              Cancel
            </button>
          </div>
        </>
      )}
    </div>
  )
}

const DISTRO_TILE_COLOR: Record<string, 'blue' | 'green' | 'purple' | 'orange' | 'red' | 'cyan'> = {
  ubuntu: 'orange',
  fedora: 'blue',
  debian: 'red',
  almalinux: 'purple',
  flatcar: 'cyan',
}

function DownloadImageModal({ existingImages, onClose, onDownloaded }: {
  existingImages: ImageInfo[]
  onClose: () => void
  onDownloaded: (path: string) => void
}) {
  const toast = useToastContext()
  const [catalog, setCatalog] = useState<CloudImage[]>([])
  const [catalogLoading, setCatalogLoading] = useState(true)
  const [selected, setSelected] = useState<CloudImage | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [statusLabel, setStatusLabel] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    listCloudImages()
      .then((data) => {
        if (cancelled) return
        setCatalog(data)
        setSelected((prev) => prev || data[0] || null)
      })
      .catch((err) => {
        if (!cancelled) setError(formatUserError(err))
      })
      .finally(() => {
        if (!cancelled) setCatalogLoading(false)
      })
    return () => { cancelled = true }
  }, [])

  const handleDownload = async () => {
    if (!selected) return
    setError(null)

    // Already sitting on disk from a previous download -- use it directly
    // instead of fetching it over the network again.
    const onDisk = existingImages.find((img) => img.name === selected.name)
    if (onDisk) {
      toast.success(`'${selected.name}' is already on disk — using the existing image`)
      onDownloaded(onDisk.path)
      return
    }

    setSubmitting(true)
    setStatusLabel('Starting download…')
    try {
      const job = await downloadCloudImage(selected.name)
      for (;;) {
        await new Promise((r) => setTimeout(r, 1500))
        const downloads = await listDownloads()
        const current = downloads.find((d) => d.id === job.id)
        if (!current) continue
        if (current.state === 'completed' && current.output_path) {
          toast.success(`Downloaded '${selected.name}'`)
          onDownloaded(current.output_path)
          return
        }
        if (current.state === 'failed') {
          setError(current.error || 'Download failed')
          setSubmitting(false)
          setStatusLabel('')
          return
        }
        setStatusLabel(current.state === 'building' ? 'Downloading…' : 'Starting download…')
      }
    } catch (err) {
      toastFailure(toast, 'Failed to start download', err)
      setError(formatUserError(err))
      setSubmitting(false)
      setStatusLabel('')
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <div className="flex items-center gap-3">
        <div className="icon-tile icon-tile-md icon-tile-cyan">
          <Download className="w-5 h-5" />
        </div>
        <div>
          <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Download an OS Image</h3>
          <p className="text-xs text-[var(--zf-muted)]">Fetch a ready-made distro image straight into the catalog</p>
        </div>
      </div>

      {error && (
        <div className="p-3 bg-[var(--zf-danger)]/10 border border-[var(--zf-danger)]/25 rounded-lg text-[var(--zf-danger)] text-sm">
          {error}
        </div>
      )}

      {catalogLoading ? (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {[0, 1, 2, 3, 4, 5].map((i) => (
            <div key={i} className="h-16 rounded-lg bg-[var(--zf-canvas)] animate-pulse" />
          ))}
        </div>
      ) : (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {catalog.map((img) => {
            const isSelected = selected?.name === img.name
            const onDisk = existingImages.some((e) => e.name === img.name)
            return (
              <button
                key={img.name}
                type="button"
                onClick={() => setSelected(img)}
                disabled={submitting}
                className={`relative text-left p-3 rounded-lg border transition-colors disabled:opacity-50 ${
                  isSelected
                    ? 'bg-[var(--zf-link)]/15 border-[var(--zf-link)]/40 ring-1 ring-[var(--zf-link)]/30'
                    : 'bg-[var(--zf-canvas)] border-[var(--zf-hairline)] hover:border-[var(--zf-hairline)]'
                }`}
              >
                {isSelected && (
                  <div className="absolute top-2 right-2 w-4 h-4 rounded-full bg-[var(--zf-link)] flex items-center justify-center">
                    <Check className="w-2.5 h-2.5 text-[var(--zf-ink)]" />
                  </div>
                )}
                <div className={`icon-tile icon-tile-sm icon-tile-${DISTRO_TILE_COLOR[img.distro] || 'blue'} mb-1.5`}>
                  <Server className="w-3.5 h-3.5" />
                </div>
                <p className="text-sm font-medium text-[var(--zf-ink)] truncate capitalize">{img.distro}</p>
                <p className="text-xs text-[var(--zf-muted)]">{img.version} · {img.arch}</p>
                {onDisk && (
                  <p className="text-[10px] font-medium text-[var(--zf-success)] mt-1">Already on disk</p>
                )}
              </button>
            )
          })}
        </div>
      )}

      {submitting && (
        <div className="flex items-center gap-2 text-sm text-[var(--zf-muted)]">
          <div className="w-3.5 h-3.5 border-2 border-[var(--zf-hairline)] border-t-[var(--zf-link)] rounded-full animate-spin" />
          {statusLabel}
        </div>
      )}

      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleDownload}
          disabled={submitting || !selected}
          className="zf-btn zf-btn-primary zf-btn-sm"
        >
          {submitting && <div className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />}
          {submitting
            ? 'Downloading…'
            : selected && existingImages.some((e) => e.name === selected.name)
              ? 'Use Existing'
              : 'Download'}
        </button>
        <button
          type="button"
          onClick={onClose}
          disabled={submitting}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
