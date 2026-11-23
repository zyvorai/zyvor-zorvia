# Research: the KubeVirt VM-platform field, late 2026

Scope: what leading products and open-source projects around KubeVirt offer that Zorvia lacks, what operators
complain about, and what is new in the last year. Method: web search plus fetched pages. **A claim marked [U] was
not read from a fetched page** (search-result snippet or memory) and should be verified before it is relied on.
Date of research: 2026-11-23. Existing Zorvia docs read first: FEATURE_MATURITY.md, MAINTENANCE.md,
DRIFT_GUARD.md, VCENTER_FEATURE_MATRIX.md, so nothing below repeats them.

## Findings

Read from fetched pages:

- KubeVirt v1.9.0 (2026): migration stream compression (zstd), stall detection that can switch to post-copy,
  `MigrationPriorityQueue` GA, vGPU migration (one device); incremental backup pull mode that skips zero
  blocks; `VMExport`, `ExpandDisk`, Persistent Reservation GA; all Beta feature gates on by default; new guest
  OS panic metrics and a `VMNonRecoverableOSPanic` alert; guest-agent ping failures become events; Intel TDX
  attestation. Source: https://kubevirt.io/2026/changelog-v1.9.0.html
- Production challenges (Spectro Cloud, a vendor): 45% of surveyed users struggle with persistent storage, 43%
  report manual effort converting VMs, live migration fails or drops traffic with `bridge` binding, Prometheus
  covers KubeVirt components but not what happens inside the guest, correlating Kubernetes/KubeVirt/guest
  problems is hard, and density past ~110 VMs/node needs tuning. https://www.spectrocloud.com/blog/kubevirt-in-the-real-world

From search snippets only [U]:

- KubeVirt 1.8 (March 2026) added a hypervisor abstraction layer, ContainerPath volumes, incremental backup with
  CBT and TDX attestation [U]. https://www.cncf.io/blog/2026/03/25/announcing-the-release-of-kubevirt-v1-8/
- Live migration needs RWX storage; bridge binding blocks it; ports 49152/49153 must stay free in virt-launcher
  and listing them in masquerade breaks it; migration traffic should use a dedicated network [U].
  https://portworx.com/knowledge-hub/kubevirt-live-migration-vs-vmware-vmotion/ ,
  https://www.kubermatic.com/learn/kubevirt/vmware-to-kubevirt-migration-planning/
- Red Hat ships a free OpenShift migration advisor that scores workload migration readiness and produces a
  report, and OpenShift Virtualization has destination-node selection, storage live migration and cross-cluster
  live migration (GA in 4.21) [U]. https://redhat.com/en/blog/virtualization-2026-building-platform-vms-containers-and-ai
- Harvester/SUSE Virtualization: I found no readiness-check or compatibility-report feature [U] (absence of
  evidence only). https://www.suse.com/products/harvester/
- Most sources are vendor blogs: treat pain-point statistics as marketing-grade, not neutral surveys.

## Ranked ideas

Fit = against Zorvia's boundaries (recommendation-only by default, opt-in Experimental gates for anything
mutating, no secrets in responses, RBAC and audit on cluster-wide data, no claim of live verification without it).

| # | Idea | Rationale | Source | Fit | Effort | Status |
|---|------|-----------|--------|-----|--------|--------|
| 1 | **Static VM manifest lint** with a "why this VM cannot live-migrate" verdict and secret-in-cloud-init detection | Every migration pitfall above is visible in the manifest before apply; competitors only offer post-hoc advice or whole-estate migration advisors | Spectro, Portworx, Kubermatic (above) | Read-only, offline, no secrets echoed | S | **Built** (docs/VM_LINT.md) |
| 2 | Migration readiness report for imported VMware inventory (guest OS, disks, NIC mapping scored) | Red Hat's advisor is the closest competitor feature [U] | redhat.com blog [U] | Read-only; needs vCenter data | L | Not built: needs a live vCenter to test |
| 3 | KubeVirt version and feature-gate audit ("which Zorvia features does this cluster's KubeVirt support") | 1.9 moved many gates to GA / on by default | kubevirt.io changelog | Read-only, but needs a per-version gate table I cannot verify | M | Not built: table unverifiable here |
| 4 | Guest OS panic and guest-agent-ping alerts surfaced in the Zorvia alert hooks | KubeVirt 1.9 emits the metric/alert/event | kubevirt.io changelog | Read-only | M | Not built: needs a live Prometheus |
| 5 | Migration tuning surface (zstd compression policy, priority queue) | KubeVirt 1.9 features | kubevirt.io changelog | Mutating a MigrationPolicy: Experimental gate, audit | M | Not built: cluster-mutating, cannot verify |
| 6 | Incremental (CBT) backup execution | Zorvia's `incremental-backup` row is Model only | CNCF 1.8 post [U] | Mutating, needs CSI/CBT support | L | Not built |
| 7 | Node-density report (VMs per node vs the ~110 virt-handler default) | Spectro lists it as a scaling cliff | Spectro | Read-only | S | Not built: low value alone |

Takeaway: the field's loudest complaints are storage setup and live-migration surprises; Zorvia already has the
runtime half (maintenance plan, preflight). The missing half was shift-left: catching the cause in the manifest.
