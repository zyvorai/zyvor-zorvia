# Zorvia documentation

Start here: root [README.md](../README.md) (hero, gallery, quick start) · live site [zyvorai.github.io/zyvor-zorvia](https://zyvorai.github.io/zyvor-zorvia/).

## Guides (moved from the root README)

| Doc | Topic |
|-----|--------|
| [getting-started.md](getting-started.md) | Install, Helm, first VM, ports, health/login/audit examples |
| [whats-inside.md](whats-inside.md) | Why Zorvia and every capability, grouped by job |
| [console-and-api.md](console-and-api.md) | Web console routes, WebSocket endpoints, Fabric HTTP API |
| [profiles-blueprints-templates.md](profiles-blueprints-templates.md) | 8 profiles, 5 blueprints, 43 templates |
| [day-2-ops.md](day-2-ops.md) | The everyday CLI loop |
| [operator-toolkit.md](operator-toolkit.md) | Drift, change plans, guest insight, golden images, Terraform, placement |
| [platform-surface.md](platform-surface.md) | Security, cost, tenancy, backup/DR, HA, automation, observability |
| [config-and-library.md](config-and-library.md) | VM config files and the Rust library |
| [develop.md](develop.md) | The make-first dev loop |
| [roadmap.md](roadmap.md) | Feature maturity and what is not shipped yet |
| [leave-openshift.md](leave-openshift.md) | Zorvia vs OpenShift Virtualization vs Zorvia with commercial support, honestly |
| [adopt-existing-kubevirt-cluster.md](adopt-existing-kubevirt-cluster.md) | Point Zorvia at an existing KubeVirt cluster, read-only first |
| [PRICING.md](PRICING.md) | Launch pricing, billing rules and examples |
| [SUPPORT_SCOPE.md](SUPPORT_SCOPE.md) | What paid support covers and excludes |
| [COMMERCIAL_FAQ.md](COMMERCIAL_FAQ.md) | Licensing, contracts and expiry questions |
| [OPENSHIFT_COMPARISON.md](OPENSHIFT_COMPARISON.md) | How to compare against OpenShift Virtualization Engine |
| [COMMERCIAL_OFFERINGS.md](COMMERCIAL_OFFERINGS.md) | Offerings, quote → contract flow and API |

## Core

| Doc | Topic |
|-----|--------|
| [LAB.md](LAB.md) | Remote deploy, auth Secret, NodePort **30152**, `lab-smoke.sh` |
| [WEB_CONSOLE.md](WEB_CONSOLE.md) | SPA, Fabric HTTP API, console/VNC/SSH/pod WebSockets, Disk Images, hotplug, migration, Rook, RBAC, audit export |
| [PODS.md](PODS.md) | Pods page: all-namespace inventory, Terminal.app logs + exec, WebSocket protocol, RBAC, audit |
| [RESCUE.md](RESCUE.md) | Rescue mode: offline hostname/SSH-key/enable-SSH via a Job-mounted disk, GuestKit, RBAC, audit |
| [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) | Validated versions and measured E2E results; what is not validated |
| [DEVICES.md](DEVICES.md) | GPU / SR-IOV / host-device passthrough: create, preflight, inventory (Experimental) |
| [KEY_PROVIDER.md](KEY_PROVIDER.md) | Encrypting TOTP secrets at rest (local key, Vault Transit), rotation, 2FA reset |
| [TLS_ROTATION.md](TLS_ROTATION.md) | Certificate hot reload (cert-manager, rotated Secrets) |
| [GUEST_AGENT.md](GUEST_AGENT.md) | In-guest agent (GuestKit / qemu-guest-agent): install at creation, freeze fix, read-only guest views, snapshot hooks, drill probe |
| [MAINTENANCE.md](MAINTENANCE.md) | Pre-drain plan: what a node drain would do to each VM |
| [HA.md](HA.md) | Supported control-plane topology (one replica, or two on PostgreSQL), failover behaviour, node-loss test |
| [POSTGRES.md](POSTGRES.md) | PostgreSQL store: what moves (users, operations, audit, JSON documents), import from SQLite, limits (Beta) |
| [TENANCY.md](TENANCY.md) | Per-user namespace allow-lists |
| [OFFCLUSTER_BACKUP.md](OFFCLUSTER_BACKUP.md) | Encrypted, verified S3 backups, restore, recovery drills, retention (Atlas bucket target) |
| [VM_IMPORT.md](VM_IMPORT.md) | VMware import with h2kvm: waves, preflight, rollback |
| [ATLAS_INTEGRATION.md](ATLAS_INTEGRATION.md) | Atlas storage control plane integration |
| [OIDC.md](OIDC.md) | Opt-in OIDC SSO (PKCE + token exchange + JWKS) |
| [OIDC_LAB.md](OIDC_LAB.md) | Lab IdP bake-off (Dex/Keycloak) |
| [FEATURE_MATURITY.md](FEATURE_MATURITY.md) | GA / Beta / Experimental / Model-only (`GET /api/v1/features`) |
| [PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) | Enterprise plan APIs (S3, Transiva, DR, GPU, fleet) |
| [UPGRADE.md](UPGRADE.md) | 0.3.2 → 0.3.3+ operator notes |
| [OS_TEMPLATES.md](OS_TEMPLATES.md) | 43 OS templates |
| [INNOVATIVE_FEATURES.md](INNOVATIVE_FEATURES.md) | Profiles, blueprints, health, recommendations |
| [THEME.md](THEME.md) | CLI/TUI theme |

## Day-2 ops

| Doc | Topic |
|-----|--------|
| [SNAPSHOTS.md](SNAPSHOTS.md) | Snapshot create/restore/retention |
| [DISK_MANAGEMENT.md](DISK_MANAGEMENT.md) | Disks and volumes |
| [NETWORK_MANAGEMENT.md](NETWORK_MANAGEMENT.md) | Networking |
| [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) | Platform `zorvia vm status` + advanced CLI |
| [GUEST_INSIGHT.md](GUEST_INSIGHT.md) | QEMU Guest Agent readiness |
| [GOLDEN_IMAGES.md](GOLDEN_IMAGES.md) | quay.io containerdisks + CDI `image-bundle` |

## Drift, plan & inventory

| Doc | Topic |
|-----|--------|
| [DRIFT_GUARD.md](DRIFT_GUARD.md) | Semantic desired-vs-live drift (`zorvia change drift`) |
| [CHANGE_PLANNER.md](CHANGE_PLANNER.md) | Operational change plan (`zorvia change plan`) |
| [VCENTER_FEATURE_MATRIX.md](VCENTER_FEATURE_MATRIX.md) | Inventory, activity, maintenance, placement |

## Integrations

| Doc | Topic |
|-----|--------|
| [KRYTON_INTEGRATION.md](KRYTON_INTEGRATION.md) | Windows via Kryton — Create VM wizard + `/app/windows` |
| [ATLAS_INTEGRATION.md](ATLAS_INTEGRATION.md) | Storage via Atlas — backend/RBD/object-store, DR, AI insights, governance, DataBridge |
| [TERRAFORM.md](TERRAFORM.md) | Terraform scaffold + Fabric API module |

## TUI

| Doc | Topic |
|-----|--------|
| [INTERACTIVE_TUI.md](INTERACTIVE_TUI.md) | Canonical interactive TUI guide |
| [INTERACTIVE_TUI_README.md](INTERACTIVE_TUI_README.md) | Pointer → interactive TUI |
| [TUI_FEATURES_DEMO.md](TUI_FEATURES_DEMO.md) | Demo walkthrough notes |

## Presentations & social

| Doc | Topic |
|-----|--------|
| [client-presentations/](client-presentations/) | HTML decks for client demos |
| [social/](social/README.md) | Share / OG card (1200×630) and LinkedIn/X card (1600×900) |

Also: [QUICK_REFERENCE.md](../QUICK_REFERENCE.md), [DEVELOPMENT.md](../DEVELOPMENT.md), [CHANGELOG.md](../CHANGELOG.md), [CONTRIBUTING.md](../CONTRIBUTING.md), [SECURITY.md](../SECURITY.md).

## Lab at a glance

```text
https://<HOST>:30152/                 # UI (NodePort)
https://<HOST>:30152/api/v1/health
https://<HOST>:30152/api/v1/features
./scripts/deploy-remote.sh <host> sus --quick
ZORVIA_E2E_PASSWORD=… ./scripts/lab-smoke.sh
```

Full steps: [LAB.md](LAB.md).
