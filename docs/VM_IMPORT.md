# VMware → KubeVirt import (h2kvm)

Zorvia drives [h2kvm](https://github.com/zyvorai/h2kvm) to move VMs from
vCenter into KubeVirt. For each VM it runs
`h2kvmctl --cmd vsphere … --deploy-k8s` as a Kubernetes Job: export from vCenter,
GuestKit offline repair (drivers, fstab, boot), convert to qcow2, upload through
CDI, create the `VirtualMachine`. Zorvia adds the parts around it: parameters,
credentials, scratch space, waves with a concurrency cap, progress, boot
verification, and cancellation that preserves imported data.

h2kvm is separately licensed and **not shipped with Zorvia**: you supply the
image (`ZORVIA_H2KVM_IMAGE`). Zorvia only runs it as a container; it never links
it.

## Set up

0. Set `ZORVIA_EXPERIMENTAL=1` — the feature is Experimental and routes refuse
   otherwise.
1. Set `ZORVIA_H2KVM_IMAGE` on the Zorvia deployment to your h2kvm CLI image
   (optional: `ZORVIA_H2KVM_INIT_IMAGE`, default `fedora:43`, used to load the
   `nbd` kernel module). Not required for `mode=stub|local` lab Jobs.
2. Prepare each target namespace (see `deploy/h2kvm-import-rbac.yaml`): label it
   `pod-security.kubernetes.io/enforce=privileged`, create the vCenter
   credentials Secret (`username` / `password` keys) and the `zorvia-h2kvm`
   ServiceAccount + Role. Zorvia has no access to Secrets or ServiceAccounts, so
   preflight can only remind you of these. Stub/local Jobs are unprivileged and
   skip the Secret.
3. KubeVirt and CDI must be installed, and (for vsphere) the cluster needs
   scratch space: the Job gets a per-import ephemeral volume
   (`scratch_size`, default 200Gi). It must hold the largest disk about twice
   over (the export and the converted image).

## Lab stub / local modes (no vCenter)

Send `"mode": "stub"` or `"mode": "local"` on the import body with an empty
`source.vcenter` / `source.secret_name`. Zorvia starts an unprivileged Job that
emits h2kvm-shaped stage logs so follow/cancel and wave progress can be tested.

| Mode | After Job success |
|---|---|
| `stub` | No VirtualMachine (`vm_created: false`) |
| `local` | Creates a **stopped** emptyDisk VirtualMachine (`vm_created: true`, `disk: emptyDisk`) |

`local` still does not talk to vCenter or pull disks; it is for lab cutover/UI flows that need a real halted VM object.

## Use

All routes are cluster.admin (vsphere starts **privileged** Jobs and reads
vCenter credentials) and are audited.

```
POST /api/vm-imports/preflight   # dry run: blockers and warnings
POST /api/vm-imports             # queue a wave (one operation per VM)
GET  /api/vm-imports             # list imports
GET  /api/vm-imports/waves/{id}  # counts by state + per-VM progress
GET  /api/operations/{op}        # one import; POST .../cancel requests Job deletion
```

```json
{
  "source": {"vcenter": "vc.example.com", "datacenter": "DC1", "insecure": false,
             "secret_name": "vcenter-creds"},
  "namespace": "vms",
  "scratch_size": "300Gi",
  "vms": [
    {"source_vm": "web-prod-01", "storage_class": "longhorn", "pvc_size": "60Gi",
     "cpu": 4, "memory": "8Gi", "start": true},
    {"source_vm": "db-prod-01", "target_vm_name": "db-01"}
  ]
}
```

- `POST /api/vm-imports` runs preflight first and answers **409** with the
  blockers if anything would stop it (missing KubeVirt/CDI, namespace enforcing
  a Pod Security level that rejects privileged pods, unknown StorageClass, target
  VM/root PVC/DataVolume name already taken, no image configured). API discovery,
  resource inspection and permission failures are blockers, not proof of absence.
- A wave runs `ZORVIA_IMPORT_CONCURRENCY` imports at a time (default 2); the rest
  stay `queued`.
- The h2kvm Job always creates a stopped VM. Zorvia applies any target network
  mapping before it starts the guest. Without `start`, it remains stopped.
- With `start`, Zorvia performs a resourceVersion-guarded start and waits for
  Running. `require_guest_agent: true` additionally requires a connected guest
  agent; it requires `start: true`. This checks VM readiness, not application health.
- `boot_timeout_secs` overrides `ZORVIA_IMPORT_BOOT_TIMEOUT_SECS` per VM. Both
  use a 30-3600 second range; the default is 600. An invalid global value falls
  back to the default; an invalid request value is rejected.

## Web console

Admins can open **Ops → VMware Imports** (`/app/vmware-imports`) to enter the
source, destination and up to 50 source VM names. The page checks readiness,
shows blockers/warnings, optionally maps a target network, queues a wave and
polls real operation progress every five seconds. Editing any field invalidates
the previous readiness check. The create endpoint repeats preflight server-side.
The form uses one network layout for the entire wave; the API supports different
layouts per VM. Existing server-side `cluster.admin` permission checks remain in
force, including cancellation through the operations API.

## Explicit network mapping

Each VM may include `networks`. An omitted/empty array keeps h2kvm's default
networking. A nonempty array **replaces the complete NIC layout**, in order:

```json
{
  "source_vm": "web-prod-01",
  "start": true,
  "require_guest_agent": true,
  "boot_timeout_secs": 900,
  "networks": [
    {"name": "default"},
    {"name": "prod", "source_network": "Production Port Group",
     "attachment": "prod-vlan", "mac_address": "52:54:00:12:34:56"}
  ]
}
```

`attachment` names an existing NetworkAttachmentDefinition in the **target
namespace**. It uses a virtio NIC with bridge binding. Omitting it selects a
pod network with masquerade binding (at most one). NIC names must be unique;
explicit MACs must be nonzero, unicast and unique within the VM. Up to 16 NICs
are supported. `source_network` records the operator's source port-group label;
Zorvia does not discover source NICs, preserve their ordering automatically or
verify that the supplied source label matches vCenter.

Preflight and execution both check the attachment's inline CNI JSON. Node-local
CNI configurations cannot be verified and are rejected by this workflow. These
checks do not prove VLAN reachability or plugin compatibility. Zorvia verifies
there is no VMI before applying the resourceVersion-guarded network patch.
Prepare Multus, bridges/VLANs and guest drivers first; no network infrastructure
is created by this feature.

The core Zorvia ServiceAccount needs **get only** on
`k8s.cni.cncf.io/network-attachment-definitions`. This read-only rule is included
in the chart's cluster and namespace modes and the two web deployment manifests.
Upgrade the chart/manifests before using mappings. No Secret read permission or
network-attachment create permission is added.

## What "succeeded" means, and cancellation

An import succeeds only when the Job finished **and** the `VirtualMachine`
exists in the cluster (Zorvia checks the API, not just h2kvm's log). If the Job
fails, times out, or you cancel, Zorvia requests foreground deletion of the
exact Job UID it created. **VMs, PVCs and DataVolumes are preserved.** Names
being free at preflight are not sufficient proof of ownership for deletion;
another actor could create a resource during the import. Inspect leftovers and
explicitly remove the verified target resources before retrying the same name.
This replaces the previous automatic name-based data cleanup.

Cancellation is checked before Job creation, network configuration, cutover and
during boot verification. It is cooperative, not an atomic fence: a request can
race an API call already in progress. Cancellation after cutover does not stop
the VM or undo network changes. Job deletion is asynchronous; cleanup errors
are logged and the Job must be inspected if the API is unavailable.

If a VM was created but did not satisfy its boot policy, it remains in place
and the operation is marked failed. A guest-agent check is not a database or
application health check. Stop/isolate the source VMs before cutover to avoid
two active copies; Zorvia does not shut down vCenter VMs automatically.

Progress is coarse: h2kvm logs stage boundaries, not bytes, so the percentage
advances per completed stage.

## Limits

- Not yet exercised against a real vCenter (it needs one plus the h2kvm image).
- Source inventory discovery and automatic port-group mapping are not implemented.
- This workflow has unit and mock-API coverage, but still needs real vCenter,
  KubeVirt/CDI and Multus validation before a production readiness claim.
- Credentials reach h2kvm through `secretKeyRef` environment variables and are
  never stored in the operation record.
