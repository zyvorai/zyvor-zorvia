# Leaving OpenShift Virtualization

OpenShift Virtualization runs KubeVirt. Its VMs are ordinary KubeVirt `VirtualMachine` objects. So moving off OpenShift's control plane does not mean rewriting your VMs. It means choosing a different platform around the same objects.

Zorvia, with optional commercial support, is one way to do that. This page compares them with OpenShift, including where OpenShift is the better choice.

> Every Zorvia "Yes" below maps to a feature in [FEATURE_MATURITY.md](FEATURE_MATURITY.md). Only **GA** and **Beta** items are production promises. Anything **Experimental** is labelled as such.

## Free code, optional commercial support

| | **Zorvia** | **Commercial support** |
|---|---|---|
| Licence | Apache-2.0, free for production | Same Apache-2.0 code, plus a support contract |
| Best for | Teams that run KubeVirt themselves and want a real control plane without a subscription | Teams that need production support and SLAs they can put in a contract |
| Surfaces | CLI, TUI, web console, one Fabric API | Same |
| Support | Community (GitHub issues) | Production support and SLAs |
| Start | [Quick start](../README.md#quick-start) | [Book a demo](https://zyvor.dev/schedule?utm_source=github&utm_medium=zorvia&utm_campaign=docs_leave_openshift) or [30-day PoC](https://zyvor.dev/poc?utm_source=github&utm_medium=zorvia&utm_campaign=docs_leave_openshift) |

**Start with Zorvia** to prove the model against your own cluster at no cost. **Add commercial support** when you need support you can put in a contract. Tell sales your cluster size and what you need, and they will say what is included today.

ZeusOS is a separate Zyvor product. It is proprietary (not open source) and is not the enterprise edition of Zorvia. See [zyvor.dev/zeus-os](https://zyvor.dev/zeus-os) if you want to look at it.

## Honest comparison

| Capability | OpenShift Virtualization | Zorvia | Commercial support |
|---|---|---|---|
| VM create and day-2 without hand-written YAML | Yes | Yes (`vm-lifecycle`, GA) | Same as Zorvia |
| Live migration between nodes | Yes | Yes (`live-migration`, GA; needs shared storage) | Same as Zorvia |
| Snapshots and restore | Yes | Yes (`snapshots-restore`, GA) | Same as Zorvia |
| Web console with VNC and serial | Yes | Yes (`web-console`, GA) | Same as Zorvia |
| Terminal UI | No | Yes | Same as Zorvia |
| Audit trail with export | Cluster audit log | Yes (`audit-trail`, GA, JSONL export) | Same as Zorvia |
| SSO | Built-in OAuth server | OIDC (`oidc`, Beta) | Same as Zorvia |
| Drift detection and change-plan gating | No | Yes (`zorvia change drift` / `zorvia change plan`) | Same as Zorvia |
| Runs on any KubeVirt cluster | No, OpenShift only | Yes | Same as Zorvia |
| Software cost | Red Hat subscription | $0 (Apache-2.0) | $0 for the software; a paid service contract for support ([pricing](PRICING.md)) |
| Vendor support contract | Yes (Red Hat) | No, community | Yes (production support and SLAs) |
| Multi-cluster management | Yes (Advanced Cluster Management) | Experimental (`fleet-multicluster`, live read-only inventory; lab dual-enrollment supported) | Ask sales |
| VM migration from VMware | Yes (Migration Toolkit for Virtualization) | Experimental in-product importer (`vm-import-h2kvm`, plus lab `mode=stub`); production path is still Transiva → h2kvm → GuestKit (separate tools). `transiva-migration` plan API is superseded | Migration Factory (Transiva, h2kvm); ask sales |
| Operator lifecycle and certified ecosystem | Yes (OLM, certified operators) | No | No |
| Telco-grade networking (SR-IOV, NFV, encrypted mesh) | Mature | Experimental (`gpu-sriov-numa`: create/preflight/permit + live placement plan; not proven on real GPU/SR-IOV hardware) | Ask sales |

## Choose OpenShift when

- You need a Red Hat support contract or certified-stack requirement written into a procurement rule.
- You already run OpenShift for containers and want one platform, one vendor.
- You depend on operators from the certified catalogue, or on telco-grade networking today.
- You need cluster-to-cluster VM migration tooling that is already GA.

## Choose Zorvia when

- You want KubeVirt without a per-node subscription and are happy to run it yourself.
- You want the same actions from a terminal, a TUI and a browser.
- You want drift and change-plan checks before a change ships.

## Choose commercial support when

- You want the same KubeVirt-native approach with a vendor behind it.
- You need production support and SLAs in a contract.
- You have multi-cluster or migration needs. Ask sales what is covered today; the Experimental items in the table above are not promises.

## What does not change

Your `VirtualMachine`, `DataVolume` and `PersistentVolumeClaim` objects stay as they are. The [adoption runbook](adopt-existing-kubevirt-cluster.md) shows how to point Zorvia at an existing KubeVirt cluster, read-only first.

## What is not done yet

- **VMware import in production:** `vm-import-h2kvm` is Experimental. Lab
  `mode=stub|local` exercises Jobs without vCenter; `mode=vsphere` needs a
  reachable vCenter, `ZORVIA_H2KVM_IMAGE`, and `scripts/lab-vsphere-preflight.sh`.
  The older Transiva plan API remains plan-only. External suite (Transiva /
  h2kvm / GuestKit) still applies for many migrations.
- **On OpenShift:** Helm now ships Route (passthrough) + optional SCC
  (`charts/zorvia/values-openshift.yaml`). A live OpenShift Virtualization
  install has **not** been recorded yet — see the [adoption runbook](adopt-existing-kubevirt-cluster.md).
- **Multi-cluster:** fleet dual-enroll of one API is lab-proven; a second
  Kubernetes API is scripted via `scripts/lab-fleet-second-cluster.sh` (k3d/kind).
  ACM-style site failover is not shipped (`cross-cluster-dr` is same-cluster
  object-store rehearsal).
- Format conversion (VMDK, VHD to qcow2) is not shipped.
- OLM / OperatorHub / HyperShift / MachineConfig: not planned.

## Cost

Red Hat does not publish a list price for OpenShift Virtualization. It states that OpenShift Virtualization Engine is licensed per bare-metal node ([Red Hat](https://www.redhat.com/en/technologies/cloud-computing/openshift/virtualization-engine)). Put your own quote against the licence line above. A calculator that takes your figure, and assumes none, is planned.

## Scope of this comparison and who is responsible for what

This page compares control planes over the same KubeVirt objects. It is not a like-for-like price or support comparison; use a customer-specific quote for that ([OPENSHIFT_COMPARISON.md](OPENSHIFT_COMPARISON.md)).

- Zorvia runs on **KubeVirt infrastructure you operate**. Unless a Managed contract says otherwise, you own Kubernetes, KubeVirt, storage, networking, hardware and guest operations. Commercial support gives diagnostic guidance for those, not ownership ([SUPPORT_SCOPE.md](SUPPORT_SCOPE.md)).
- OpenShift bundles a supported platform stack; Zorvia does not replace that stack, and does not claim equivalent certification or support coverage.
- Commercial support is a paid service contract around the Apache-2.0 software, not a software licence. Nothing in the product enforces it.

## Talk to us

- **Commercial support:** [talk to sales](https://zyvor.dev/contact?intent=sales&utm_source=github&utm_medium=zorvia&utm_campaign=docs_leave_openshift) (fallback sales@zyvor.dev). Tell us your node count and your current subscription, and we will tell you what is possible today.
- **Also from Zyvor:** [ZeusOS](https://zyvor.dev/zeus-os), a separate proprietary product.
