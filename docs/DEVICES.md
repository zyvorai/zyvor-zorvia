# GPUs, SR-IOV and other passthrough devices

Zorvia can create VMs that hold a physical or mediated device, and checks *before* creating the VM that
the cluster can actually give it that device.

> **Status: Experimental, not run on real hardware.** The VM spec generation, the inventory and the
> preflight are unit-tested, and the API behaviour was checked on a lab that has **no GPU and no SR-IOV
> NIC**. A trial on your own GPU/SR-IOV hardware is still needed before relying on it.

## Creating a VM with a device

`POST /api/vms` takes `devices`:

```json
{ "name": "gpu-vm", "image": "...", "cpus": 8, "memory": 16384,
  "devices": [
    { "name": "gpu0", "device_name": "nvidia.com/GA102GL_A10", "kind": "gpu" },
    { "name": "nic0", "device_name": "intel.com/x710", "kind": "host-device" } ] }
```

`device_name` is the extended resource your **device plugin** advertises. `kind` `gpu` (default) is emitted
as KubeVirt `spec.domain.devices.gpus[]`, `host-device` as `hostDevices[]` (any other PCI, mediated or USB
device). At most 16 per VM; names are DNS labels and unique; resource names look like `vendor.com/name`.
The Create VM page has a **Passthrough devices** section (suggestions come from the inventory when you are a
cluster admin).

**SR-IOV networks** use the existing `interfaces` field: `{"name":"vf0","network":"vf-net","model":"virtio",
"network_type":{"sriov":{"name":"vf-net"}}}` (the **Additional NICs** section's SR-IOV option), a Multus
`NetworkAttachmentDefinition` in the VM's namespace. KubeVirt gets `interfaces[].sriov` on a `multus` network.

## Preflight (and why creation can answer 422)

A VM that asks for hardware nobody offers would be created and then sit `Pending`. Zorvia checks first and
answers **422 `DEVICE_PREFLIGHT_FAILED`** with the reasons and an `issues` list. It checks:

| check | error when |
|---|---|
| KubeVirt permission | the resource is not in the KubeVirt CR's `spec.configuration.permittedHostDevices` (KubeVirt would refuse it) |
| availability | no node advertises it (device plugin missing, hardware absent), or every unit is already in use |
| SR-IOV: Multus | the `NetworkAttachmentDefinition` CRD is not installed |
| SR-IOV: network | the attachment does not exist in the VM's namespace |
| SR-IOV: VF pool | the attachment's `k8s.v1.cni.cncf.io/resourceName` pool has no free virtual functions on any node |

Warnings (never block): the attachment has no `resourceName` annotation (the pool cannot be checked), and
**a VM with a passthrough device or SR-IOV interface is pinned to its node**: it cannot be live-migrated and a
node drain will stop it ([MAINTENANCE.md](MAINTENANCE.md) flags such VMs). If the KubeVirt CR cannot be read the
permission check is skipped rather than guessed.

`POST /api/v1/devices/preflight` runs the same check without creating anything
(`{"devices":[...], "sriov_networks":["vf-net"], "namespace":"..."}` -> `{"ok":false,"issues":[...]}`).
`GET /api/v1/devices` (cluster.admin) is the inventory: per node the device-plugin resources and free counts,
KubeVirt's builtin pseudo-devices, the permitted devices and feature gates, and whether Multus is installed.

## What was checked live (2026-10-02, lab with KubeVirt 1.9 and Multus, no GPU, no SR-IOV NIC)

`GET /api/v1/devices` listed the node's KubeVirt builtins (kvm, tun, vhost-net, sev), no passthrough devices, an empty
`permittedHostDevices`, and `multus_installed: true`. A fake GPU failed the preflight with both errors (not permitted, no node
advertises it); `POST /api/vms` with it, and with an SR-IOV NIC on a network that does not exist, answered **422
`DEVICE_PREFLIGHT_FAILED`** and **left no VM behind**; an invalid device name answered 400. Permissions: a Viewer gets 403 on both
the inventory and the preflight, a User gets 403 on the inventory and 200 on the preflight. Not exercised: the path where the
NetworkAttachmentDefinition CRD is absent (it relies on the API server's 404 wording and is untested on a cluster without Multus),
and anything with real devices.

## CPU, NUMA and hugepages (what GPU VMs usually need)

`POST /api/vms` also takes `cpu_dedicated_placement` (pin vCPUs), `memory_hugepages_page_size` (`2Mi` or `1Gi`) and
`cpu_numa_passthrough` (give the guest the host's NUMA topology, KubeVirt `cpu.numa.guestMappingPassthrough`). KubeVirt's
admission webhook refuses NUMA passthrough without dedicated CPUs and without hugepages (verified against KubeVirt 1.9 with a
server-side dry run), so Zorvia answers 400/422 with that reason first. Placement is also preflighted against the nodes:

| check | error when |
|---|---|
| dedicated CPUs | no node carries `kubevirt.io/cpumanager=true` (the static CPU manager policy) |
| hugepages | no node has free pages of that size |
| NUMA | (warning) the VM follows the host node's topology and cannot move to a node with a different layout |

`POST /api/v1/devices/preflight` accepts `dedicated_cpus`, `numa_passthrough` and `hugepages` too. The inventory reports
`cpu_manager` and free `hugepages` per node, the SR-IOV pools (every `NetworkAttachmentDefinition` that names a pool, with free
virtual functions across nodes), and a `kind_hints` map (`gpu` / `host-device`) for forms. The console has a
**Passthrough devices** page (Compute menu, cluster admins) for all of it.

## Permitting a device from Zorvia (cluster.admin)

`POST /api/v1/devices/permitted` adds a device to the KubeVirt CR's `spec.configuration.permittedHostDevices`;
`DELETE /api/v1/devices/permitted?resource_name=vendor.com/name` removes it. Both are audited as configuration changes.

```json
{ "kind": "pci",      "resource_name": "nvidia.com/GA102GL_A10", "pci_vendor_selector": "10DE:2236" }
{ "kind": "mediated", "resource_name": "nvidia.com/GRID_T4-2Q",  "mdev_name_selector": "GRID T4-2Q" }
{ "kind": "usb",      "resource_name": "kubevirt.io/storage",    "vendor": "46f4", "product": "0001" }
```

Selectors are validated and normalised (PCI ids upper case, USB ids lower case); `external_resource_provider: true` marks a
resource whose device plugin runs outside KubeVirt. Asking for the same entry again answers `changed: false`; the same resource name
with a different selector or kind is a 409 (remove it first). A merge patch replaces a whole list, so Zorvia sends the list it read
together with the CR's `resourceVersion`: a concurrent change makes the API server answer 409 instead of being overwritten.

This needs the service account to **patch** the `kubevirts` resource, which is off by default (it lets a Zorvia admin change which
hardware every VM may take). Helm: `devices.managePermitted: true`; plain manifests: add `patch` to the `kubevirts` rule. Without it the
API answers 403 `RBAC_DENIED` and says so.

## What you have to set up (Zorvia does not)

1. A **device plugin** for the hardware (NVIDIA GPU operator / KubeVirt GPU device plugin, the SR-IOV network
   device plugin, ...) so nodes advertise the resource.
2. The device in the KubeVirt CR's `permittedHostDevices`: from the console or API above (with `devices.managePermitted`), or yourself.
   Lab without NVIDIA hardware: `scripts/lab-fake-gpu-device.sh` +
   `deploy/lab-fake-gpu-device-plugin.yaml` advertise `lab.zorvia.io/fake-gpu`
   (permit with `external_resource_provider: true`, then preflight/create).
3. For SR-IOV: Multus and a `NetworkAttachmentDefinition` whose annotation `k8s.v1.cni.cncf.io/resourceName`
   names the VF pool. On older KubeVirt versions, the `GPU` / `HostDevices` feature gates.
4. Host prerequisites for passthrough (IOMMU enabled, VFIO binding, the CPU manager static policy, reserved hugepages) are the node's job.

Zorvia's service account needs read access to the `kubevirts` resource (added to the Helm chart and the
manifests: `get`, `list`), and `get`/`list` on `network-attachment-definitions` (pool listing).

## Not covered

GPU sharing/MIG policy beyond naming the resource your device plugin exposes, hotplugging a device into a running VM (KubeVirt does
not support it), choosing the node that holds a specific unit (KubeVirt schedules by the extended-resource request), and any
validation on real hardware: the permit/unpermit patches and the NUMA/hugepage spec were checked against a real KubeVirt API
(server-side dry run and live), but nothing here has run with a GPU or an SR-IOV NIC.
