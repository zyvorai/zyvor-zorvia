<div align="center">

<img src="docs/social/zorvia-hero-dark.jpg" alt="Zorvia - Kubernetes VMs, run like a platform, not YAML." width="100%">

# Zorvia

### Kubernetes VMs, run like a platform — not a pile of YAML.

KubeVirt gives you `VirtualMachine` objects. Zorvia gives you the control plane around them.<br>
CLI, interactive TUI and a signed-in web console — one API, live cluster objects, not a parallel mock store.

[![CI](https://img.shields.io/github/actions/workflow/status/zyvorai/zyvor-zorvia/ci.yml?branch=main&style=flat-square&labelColor=1d1d1f&label=CI)](https://github.com/zyvorai/zyvor-zorvia/actions)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-0071e3?style=flat-square&labelColor=1d1d1f)](LICENSE)
[![Version](https://img.shields.io/github/v/release/zyvorai/zyvor-zorvia?label=version&style=flat-square&color=0071e3&labelColor=1d1d1f)](CHANGELOG.md)
[![KubeVirt-native](https://img.shields.io/badge/KubeVirt-native-0071e3?style=flat-square&labelColor=1d1d1f)](https://kubevirt.io/)
[![Rust 1.89+](https://img.shields.io/badge/rust-1.89%2B-0071e3?style=flat-square&labelColor=1d1d1f&logo=rust&logoColor=white)](https://www.rust-lang.org/)

[**Quick start**](#quick-start) · [**Console**](#console-gallery) · [**Docs**](docs/README.md) · [**Leaving OpenShift?**](docs/leave-openshift.md) · [**Book a demo**](https://zyvor.dev/schedule?utm_source=github&utm_medium=zorvia&utm_campaign=readme_hero) · [**30-day PoC**](https://zyvor.dev/poc?utm_source=github&utm_medium=zorvia&utm_campaign=readme_hero) · [**Star on GitHub**](https://github.com/zyvorai/zyvor-zorvia)

</div>

---

## Create. Operate. Reach. Govern. Prove.

**Create** from a template or blueprint instead of hand-writing CRDs. **Operate** day-2 — hotplug, live migrate, snapshot — without downtime. **Reach** the guest over an authenticated console, VNC or SSH. **Govern** with real RBAC and quotas. **Prove** it afterward with an exportable audit trail. Full docs: [zyvorai.github.io/zyvor-zorvia](https://zyvorai.github.io/zyvor-zorvia/).

| Step | Surface | What it does |
|------|---------|--------------|
| **1 · Create** | Templates · profiles · blueprints | 43 OS templates, workload profiles, multi-VM stacks — no hand-written VM CRDs |
| **2 · Day-2** | Hotplug · migrate · snapshot | CPU/memory/disk/NIC hotplug, live migration, snapshots, disk resize |
| **3 · Access** | Console · VNC · SSH | Authenticated WebSockets into the guest from the signed-in console |
| **4 · Guard** | RBAC · namespaces · quotas · NetworkPolicy | Server-side roles, per-user [namespace allow-lists](docs/TENANCY.md), `ResourceQuota`, `NetworkPolicy` — not client-only checks |
| **5 · Audit** | Trail · JSONL export | Persistent audit DB + `GET /api/audit/export` for SIEM shippers |

## What is in the box

<table>
<tr>
<td valign="top" width="33%">
<b>Ship it</b><br>
43 named OS templates, 8 resource profiles and 5 multi-VM blueprints (LAMP, 3-tier, k8s-cluster, CI/CD, dev-stack). No hand-written VM CRDs.<br>
<a href="docs/profiles-blueprints-templates.md">Profiles, blueprints, templates</a>
</td>
<td valign="top" width="33%">
<b>Run day-2 without downtime</b><br>
Hotplug CPU, memory, disk and NIC; live migration; disk resize. <code>zorvia change drift</code> and <code>zorvia change plan</code> catch problems before they ship.<br>
<a href="docs/day-2-ops.md">Day-2 commands</a>
</td>
<td valign="top" width="33%">
<b>Reach the guest</b><br>
Serial, VNC and in-browser SSH with a real pty, all over authenticated WebSockets, from the same signed-in console.<br>
<a href="docs/console-and-api.md">Web console and API</a>
</td>
</tr>
<tr>
<td valign="top" width="33%">
<b>Govern who can</b><br>
Admin, user and viewer accounts; server-side <code>ResourceQuota</code> and <code>NetworkPolicy</code> CRUD; opt-in OIDC SSO (Beta).<br>
<a href="docs/OIDC.md">OIDC SSO</a>
</td>
<td valign="top" width="33%">
<b>Prove it, protect it</b><br>
Persistent audit trail with JSONL export, compliance posture against actual VM specs (PCI-DSS, HIPAA, SOC2), VolumeSnapshots and scheduled backups.<br>
<a href="docs/whats-inside.md">What is inside</a>
</td>
<td valign="top" width="33%">
<b>Fit more VMs, spend less</b><br>
Placement Advisor and Capacity Planning against real Node capacity, a Resource Optimizer, and Warm Pools for burst capacity.<br>
<a href="docs/platform-surface.md">Platform surface</a>
</td>
</tr>
</table>

One Fabric API sits behind the CLI, the interactive TUI and the web console, and every call ends in real Kubernetes objects: `VirtualMachine`, `VirtualMachineSnapshot`, `NetworkPolicy`, `ResourceQuota`, PVCs and DataVolumes. If a page shows a number, it came from the cluster. Diagram: [docs/whats-inside.md](docs/whats-inside.md).

## Console gallery

One continuous session on a real lab cluster (HTTPS NodePort **30152**) — dashboard → VM list → in-browser console → live metrics. Not mockups.

<div align="center">

<img src="docs/screenshots/readme-demo.gif" alt="Zorvia demo" width="860">

**Dashboard → VM list → open a VM's console → watch a real guest boot.** ~15–20s, no audio.

</div>

## Install

One binary, one Helm chart — bring your own KubeVirt cluster. Requires Rust **1.89+**, a kubeconfig, and a cluster with **KubeVirt** (CDI optional for golden images / CDI clone).

```bash
git clone https://github.com/zyvorai/zyvor-zorvia.git && cd zorvia
cargo install --path . --features web   # `web` is the default feature

# cluster install
helm upgrade --install zorvia charts/zorvia -n zorvia-system --create-namespace \
  -f charts/zorvia/values-lab.yaml
```

Production values: `charts/zorvia/values-production.yaml`. Lab remote deploy: [docs/LAB.md](docs/LAB.md).

## Quick start

Profile → create → start → reach the guest. One real VM.

```bash
zorvia profile show database
zorvia vm create prod-db --template ubuntu-22.04 --cpus 6 --memory 16Gi --disk-size 200Gi
zorvia vm start prod-db
zorvia guest wait-ready prod-db --timeout 120
zorvia vm status prod-db --watch
zorvia blueprint deploy lamp --prefix myapp --start     # or a whole multi-VM stack
```

The web console is served on HTTPS NodePort **30152** (self-signed): `open https://<HOST>:30152/app`. Ports, the health/login/audit `curl` examples and the lab password Secret are in [Install and quick start](docs/getting-started.md#quick-start).

## How it stacks up

|  | Hand-rolled `kubectl`/`virtctl` | Generic K8s dashboards | OpenShift Virtualization | **Zorvia** |
|---|---|---|---|---|
| VM create/day-2 without hand-written YAML | No | Partial: view-only for VMs | Yes | Yes |
| Same capability from CLI, TUI, *and* web | No | No (web only) | Partial: web + `virtctl`, no TUI | Yes |
| Cost/right-sizing, compliance, HA built in | No | No | Partial: add-ons | Yes: real, on by default |
| Drift detection + change-plan gating | No | No | No | Yes (`zorvia change drift` / `zorvia change plan`) |
| Runs on any KubeVirt cluster | Yes | Yes | No: OpenShift only | Yes |
| Open source, Apache-2.0 | Yes | Varies | No | Yes |

Zorvia isn't a general Kubernetes dashboard — it's opinionated about one thing: VMs on KubeVirt, done like a platform.

## Leaving OpenShift?

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/social/compare/lanes-1200x630-dark.png">
  <img src="docs/social/compare/lanes-1200x630.png" alt="Same KubeVirt VMs, pick your control plane: OpenShift (Red Hat subscription), Zorvia (open source, Apache-2.0) and ZeusOS (proprietary, commercial) compared side by side." width="820">
</picture>

</div>

OpenShift Virtualization runs KubeVirt, so your VMs are already `VirtualMachine` objects. Leaving means changing the control plane around them, not rewriting them.

- **Zorvia** — free, Apache-2.0, no subscription. Point it at your KubeVirt cluster, read-only first: [adoption runbook](docs/adopt-existing-kubevirt-cluster.md).
- **Coming from VMware?** Zorvia has no production importer of its own. Use the Zyvor suite: [Transiva](https://github.com/zyvorai/zyvor-transiva) (export) → [h2kvm](https://github.com/zyvorai/zyvor-h2kvm) (convert, deploy) → [GuestKit](https://github.com/zyvorai/zyvor-guestkit) (assure) → Zorvia (operate). Community tiers are free; h2kvm needs a paid licence for production.
- **Commercial support** — production support and SLAs by contract, on top of the free Apache-2.0 code. Scope and terms are agreed with sales: [talk to sales](https://zyvor.dev/contact?intent=sales&utm_source=github&utm_medium=zorvia&utm_campaign=readme_edition), or [book a demo](https://zyvor.dev/schedule?utm_source=github&utm_medium=zorvia&utm_campaign=readme_edition).

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/social/compare/scorecard-1200x630-dark.png">
  <img src="docs/social/compare/scorecard-1200x630.png" alt="Scorecard comparing OpenShift, Zorvia and ZeusOS across nine rows. OpenShift leads on support contract, multi-cluster and integrated VMware migration; Zorvia leads on licence cost, running on any KubeVirt cluster, TUI and built-in drift gating." width="820">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/social/compare/migration-1200x630-dark.png">
  <img src="docs/social/compare/migration-1200x630.png" alt="VMware to KubeVirt, four tools one path: Transiva exports, h2kvm converts and deploys, GuestKit assures, Zorvia operates, each with Community and Enterprise tiers." width="820">
</picture>

</div>

> **Running this for a business?** Commercial support gives you support and SLAs you can put in a contract. [Book a demo](https://zyvor.dev/schedule?utm_source=github&utm_medium=zorvia&utm_campaign=readme_edition) or start a [30-day PoC](https://zyvor.dev/poc?utm_source=github&utm_medium=zorvia&utm_campaign=readme_edition) on a real cluster, then follow the [adoption runbook](docs/adopt-existing-kubevirt-cluster.md).

## Production support and services

Everything in this repository stays Apache-2.0, free for commercial use, and works without a subscription. Paid service contracts add support coverage and services (deployment, migration, training, managed operations) around it. They are not a software licence. A contract records what was purchased. It never stops VMs, disables APIs or blocks access to your data.

| Topic | Doc |
|---|---|
| Launch pricing, billing rules, examples | [docs/PRICING.md](docs/PRICING.md) · [docs/COMMERCIAL_FAQ.md](docs/COMMERCIAL_FAQ.md) |
| Positioning against OpenShift Virtualization Engine | [docs/OPENSHIFT_COMPARISON.md](docs/OPENSHIFT_COMPARISON.md) |
| Offerings, quote → contract flow, API and maturity | [docs/COMMERCIAL_OFFERINGS.md](docs/COMMERCIAL_OFFERINGS.md) |
| What support covers, response targets and timers | [docs/SUPPORT_SCOPE.md](docs/SUPPORT_SCOPE.md) |
| Managed operations, access and remediation rules | [docs/MANAGED_OPERATIONS.md](docs/MANAGED_OPERATIONS.md) |
| What a diagnostic bundle contains and excludes | [docs/DIAGNOSTIC_PRIVACY.md](docs/DIAGNOSTIC_PRIVACY.md) |
| Billing unit and capacity records | [docs/BILLING_UNITS.md](docs/BILLING_UNITS.md) |

Delivery is phased. Catalog, quote requests, contracts and coverage are implemented (Beta). The other areas are documented as design and labelled as not yet available in each doc.

## Documentation

| Topic | Doc |
|---|---|
| Every guide, by area | [docs/README.md](docs/README.md) |
| Install, Helm, first VM, ports and API examples | [docs/getting-started.md](docs/getting-started.md) |
| Why Zorvia, and every capability | [docs/whats-inside.md](docs/whats-inside.md) |
| Web console routes and the Fabric HTTP API | [docs/console-and-api.md](docs/console-and-api.md) · [docs/WEB_CONSOLE.md](docs/WEB_CONSOLE.md) |
| Profiles, blueprints, templates | [docs/profiles-blueprints-templates.md](docs/profiles-blueprints-templates.md) · [docs/OS_TEMPLATES.md](docs/OS_TEMPLATES.md) |
| Day-2 commands and the operator toolkit | [docs/day-2-ops.md](docs/day-2-ops.md) · [docs/operator-toolkit.md](docs/operator-toolkit.md) |
| Security, cost, tenancy, DR, HA, automation | [docs/platform-surface.md](docs/platform-surface.md) |
| Validated versions, measured results, what is not validated | [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) |
| Per-user namespace restriction | [docs/TENANCY.md](docs/TENANCY.md) |
| Control-plane HA, PostgreSQL store, two replicas | [docs/HA.md](docs/HA.md) · [docs/POSTGRES.md](docs/POSTGRES.md) |
| In-guest agent: install, guest views, snapshot hooks | [docs/GUEST_AGENT.md](docs/GUEST_AGENT.md) |
| Verified off-cluster backup, restore, recovery drills | [docs/OFFCLUSTER_BACKUP.md](docs/OFFCLUSTER_BACKUP.md) |
| VMware import (h2kvm) and Atlas storage | [docs/VM_IMPORT.md](docs/VM_IMPORT.md) · [docs/ATLAS_INTEGRATION.md](docs/ATLAS_INTEGRATION.md) |
| Config file and the Rust library | [docs/config-and-library.md](docs/config-and-library.md) |
| Leaving OpenShift: comparison and adoption | [docs/leave-openshift.md](docs/leave-openshift.md) · [docs/adopt-existing-kubevirt-cluster.md](docs/adopt-existing-kubevirt-cluster.md) |
| Feature maturity and roadmap | [docs/roadmap.md](docs/roadmap.md) · [docs/FEATURE_MATURITY.md](docs/FEATURE_MATURITY.md) |
| Lab deploy, pods, OIDC | [docs/LAB.md](docs/LAB.md) · [docs/PODS.md](docs/PODS.md) · [docs/OIDC_LAB.md](docs/OIDC_LAB.md) |
| Development | [docs/develop.md](docs/develop.md) · [DEVELOPMENT.md](DEVELOPMENT.md) · [QUICK_REFERENCE.md](QUICK_REFERENCE.md) |

<a id="whats-inside"></a><a id="why-teams-pick-zorvia"></a><a id="platform-surface"></a><a id="day-2-commands"></a><a id="web-console--api"></a><a id="profiles--blueprints--templates"></a><a id="operator-toolkit"></a><a id="config--library"></a><a id="develop"></a><a id="roadmap"></a>

**Roadmap:** a feature-maturity registry, not a marketing slide. Only **GA** and documented **Beta** paths are production promises: [docs/roadmap.md](docs/roadmap.md), or `GET /api/v1/features` on a running API.

## Project security

No `unsafe` on the product path. CORS off unless configured. TLS verification enforced. Console, VNC and SSH sockets are permission-checked, origin-checked and audited; users can be confined to namespaces; sign-in is throttled, MFA re-enrolment needs proof, and sessions can be revoked (`POST /api/v1/auth/logout`). Lab mode is off by default. Profile/blueprint storage blocks path traversal. API errors are sanitized; auth inputs bounded. Report privately via [GitHub Security Advisories](https://github.com/zyvorai/zyvor-zorvia/security/advisories) or `info@zyvor.dev` — [SECURITY.md](SECURITY.md).

## Get involved

- **Running this in production?** Production support and SLAs are available by contract; other Zyvor products are licensed separately. [Book a demo](https://zyvor.dev/schedule?utm_source=github&utm_medium=zorvia&utm_campaign=readme_footer) or start a [30-day PoC](https://zyvor.dev/poc?utm_source=github&utm_medium=zorvia&utm_campaign=readme_footer). Tell us your cluster size and what's on your [roadmap](docs/roadmap.md) list, and we'll tell you what's already possible today. Fallback: sales@zyvor.dev.
- **Evaluating it?** Clone it and run the [quick start](#quick-start); every claim in this README maps to a route or command you can hit right now. Questions, bug reports and feature requests are welcome as [GitHub issues](https://github.com/zyvorai/zyvor-zorvia/issues); a [star on the repo](https://github.com/zyvorai/zyvor-zorvia) helps others find it.
- **Contributing code?** PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Commercial subscriptions and support: see [docs/SUBSCRIPTION-MODEL.md](docs/SUBSCRIPTION-MODEL.md).

Open source under the [Apache License, Version 2.0](LICENSE). You may use, modify, and run it for personal, lab, and commercial production use at no charge, subject to Apache-2.0 (preserve notices / NOTICE where required). See [NOTICE](NOTICE) — Apache-2.0 only (not dual-licensed with MIT).

<div align="center">

Built on [KubeVirt](https://kubevirt.io/) and [kube-rs](https://github.com/kube-rs/kube). Part of the Zyvor platform — more at **[zyvor.dev](https://zyvor.dev/?utm_source=github&utm_medium=zorvia&utm_campaign=readme_footer)**.

</div>
