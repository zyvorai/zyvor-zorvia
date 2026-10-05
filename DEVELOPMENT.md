# Development Status

## Project Statistics

Approximate counts from the current tree (regenerate when stats matter):

- **Lines of Code**: ~116,000 (`src/**/*.rs`)
- **Top-level modules**: 62 `pub mod` entries in `src/lib.rs` (plus nested modules under handlers/API)
- **Templates**: 43 named OS templates (including aliases)
- **CLI Commands**: ~178 top-level `Commands` variants
- **Resource Profiles**: 8 built-in
- **Deployment Blueprints**: 5 built-in
- **Tests**: ~2,478 `#[test]` / `#[tokio::test]` attributes
- **Compiler Warnings**: 0 on `make ci` when green
- **Feature flags**: default `web` (Axum SPA/API); `pam` exists but is a placeholder reject

Honest scope: core VM craft, Fabric SPA, snapshots, drift/plan, golden images, and Terraform scaffold are the day-2 product surface. Larger libraries under FinOps, AI/ML, edge, service-mesh-style modules, etc. ship as CLI/library code — treat them as advanced surfaces, not every-cluster guarantees. See root [README.md](README.md) for the product front door.

## Architecture

```
src/
├── lib.rs              (575 lines - thin routing layer)
├── main.rs             (binary entry point)
├── handlers/           (11 modules - command implementations)
│   ├── vm.rs           (core VM CRUD operations)
│   ├── profiles.rs     (profiles, blueprints, deploy, health)
│   ├── infra.rs        (snapshots, monitoring, disk, network)
│   ├── backup.rs       (backup, disaster recovery, migration)
│   ├── security.rs     (scanning, hardening, compliance, audit)
│   ├── cost.rs         (cost analysis, budgets, optimization)
│   ├── automation.rs   (rules, workflows, schedules)
│   ├── observability.rs (logs, metrics, alerts, insights)
│   ├── multitenancy.rs (tenants, users, roles, quotas, groups)
│   ├── devexp.rs       (completions, config templates, diff, init)
│   └── api.rs          (REST API, webhooks, TUI)
├── cli/                (command-line parsing with clap)
├── config/             (VM configuration types, builder, validator)
├── kube/               (Kubernetes client, CRD types, converter, expose)
├── templates/          (43 OS templates)
├── tui/                (terminal UI with ratatui)
├── profiles/           (resource profile system)
├── blueprints/         (multi-VM deployment templates)
├── snapshots/          (KubeVirt snapshot management)
├── monitoring/         (metrics collection and analysis)
├── health/             (VM health checks and scoring)
├── backup/             (backup, recovery, scheduling)
├── migration/          (live migration, HA, evacuation)
├── security/           (scanning, hardening, compliance, audit)
├── cost/               (cost tracking, budgets, optimization)
├── automation/         (rules, workflows, schedules)
├── observability/      (logs, metrics, alerts, insights)
├── multitenancy/       (tenants, RBAC, quotas)
├── api/                (REST API, OpenAPI, webhooks, http_server + SPA)
├── devexp/             (completions, config templates, diff, init)
├── rook/               (Rook-Ceph CRD client, manifests, operator bootstrap, health)
├── golden_images/      (CDI DataVolume/DataSource bundle generation + download jobs)
├── kryton/             (Kryton Windows control-plane proxy client)
├── platform_status/    (Cilium-style `zorvia vm status` logo + component probe)
└── [10 more modules]   (networking, finops, edge, secrets, etc.)
```

## Completed Features

### Core Infrastructure
- [x] KubeVirt CRD definitions and Kubernetes client
- [x] VM CRUD operations (create, list, get, delete, start, stop, restart)
- [x] VM cloning, export, batch operations
- [x] Interactive creation wizard
- [x] Configuration validation (46 tests)
- [x] 43 OS templates (Ubuntu, Fedora, CentOS Stream, Debian, RHEL, Windows, etc.)
- [x] Builder pattern for VMConfig
- [x] YAML/JSON output formatting

### Resource Profiles & Blueprints
- [x] 8 built-in profiles (minimal, dev, test, web, prod, database, microservice, high-perf)
- [x] Custom profile CRUD with filesystem persistence
- [x] 5 built-in blueprints (LAMP, K8s cluster, 3-tier, CI/CD, dev-stack)
- [x] Custom blueprint CRUD with validation
- [x] Blueprint deployment with dependency ordering
- [x] Topological sort and cycle detection

### Snapshots & Backup
- [x] KubeVirt snapshot CRD integration
- [x] Snapshot create/list/get/delete/restore
- [x] Retention policies with enforcement
- [x] Backup create/list/verify with compression
- [x] Backup scheduling (daily, hourly, weekly)
- [x] Disaster recovery plans and execution

### Monitoring & Health
- [x] Live VM monitoring with metrics
- [x] Health checks with scoring (0-100)
- [x] Resource recommendations
- [x] Performance comparison across VMs
- [x] Top resource consumers view

### Network & Migration
- [x] Network interface management
- [x] Traffic analysis and bandwidth monitoring
- [x] Network policies (Kubernetes + Cilium)
- [x] Live migration with progress tracking (CLI + `POST /api/vms/:name/migrate`, `/app/migrations`)
- [x] HA configuration and failover
- [x] Node evacuation planning

### Day-2 Hotplug & Distributed Storage
- [x] CPU/memory hotplug (`POST /api/vms/:name/hotplug/cpu|memory`) — requires cluster-side KubeVirt `VMLiveUpdateFeatures` to actually reach the running guest, not just Zorvia's API
- [x] Disk hotplug attach/detach (`POST/DELETE /api/vms/:name/hotplug/disk[/:id]`), verified live end-to-end
- [x] NIC hotplug attach/detach (`POST/DELETE /api/vms/:name/hotplug/nic[/:id]`), reports 501 when Multus/HotplugNICs unavailable
- [x] Disk resize for PVC-backed disks (`POST /api/vms/:name/disks/:disk_name/resize`), grow-only
- [x] Rook-Ceph: pools, filesystems, object stores, StorageClass/VolumeSnapshotClass provisioning, operator bootstrap (`/api/storage/rook/*`, `/app/storage`)

### Security & Compliance
- [x] Vulnerability scanning (quick/standard/deep/compliance)
- [x] Security assessment with scoring
- [x] CIS and STIG hardening profiles
- [x] Compliance checking (PCI-DSS, HIPAA, SOC2, GDPR, NIST)
- [x] Audit logging and statistics

### Cost Management
- [x] Cost analysis per VM and namespace
- [x] Budget management with alerts
- [x] Cost optimization recommendations
- [x] Waste detection and reporting
- [x] Cost forecasting

### Automation & Orchestration
- [x] Automation rules with triggers (schedule, event, metric)
- [x] Multi-step workflows with templates
- [x] Scheduled task management
- [x] Workflow execution tracking

### Observability
- [x] Log querying with filtering
- [x] Metrics collection and aggregation
- [x] Alert rule management
- [x] Insight generation and recommendations
- [x] Trend analysis

### Multi-Tenancy & RBAC
- [x] Tenant management with namespaces
- [x] User and group management
- [x] Role-based access control
- [x] Resource quotas with presets

### Developer Experience
- [x] Shell completions (bash, zsh, fish, powershell, elvish)
- [x] Configuration template save/load/list
- [x] YAML config diff tool
- [x] Project initialization scaffolding
- [x] Environment info and diagnostics

### API & Interface
- [x] REST API server with OpenAPI spec
- [x] API key + JWT login (TOTP / PAM / opt-in OIDC with PKCE+JWKS — `docs/OIDC.md`)
- [x] Fabric-compatible `/api/vms` create, power, port-forwards, cloud-init, clone, snapshots, hotplug, disk resize, migration
- [x] `/api/vms` create accepts the full CLI `VMConfig` surface (firmware, CPU model, hugepages, TPM/RNG, HyperV, multiple disks/NICs), not just the single-disk/NIC wizard defaults
- [x] Web SPA (`web/`) — Dashboard, VMs, Create VM (Linux + Kryton Windows), Console (serial+VNC+SSH), Snapshots, Migrations, Storage (Rook-Ceph)
- [x] Authenticated `/ws/console/:name` and `/ws/vnc/:name` KubeVirt proxies
- [x] NodePort expose helpers (`src/kube/expose.rs`) for SSH/VNC/RDP
- [x] In-cluster HTTPS NodePort 30152 (`deploy/k8s.yaml`)
- [x] Webhook management with event filtering
- [x] Interactive TUI with ratatui
- [x] TUI VM creation and snapshot creation

### Additional Modules
- [x] FinOps (allocation, budgets, optimization, waste, reports)
- [x] Advanced networking (IPAM, BGP, DNS, QoS, topology)
- [x] Service mesh integration
- [x] Disaster recovery (failover, HA, replication)
- [x] Edge computing (nodes, sync, telemetry)
- [x] Secrets management (encryption, rotation)
- [x] Multi-cloud (providers, federation, connectivity, portability)
- [x] Capacity planning
- [x] AI/ML (GPU management, inference)
- [x] GitOps integration

## Quick Test Commands

```bash
# Build
cargo build --features web

# Run all tests
cargo test

# List templates
cargo run -- templates

# Show template details
cargo run -- template ubuntu

# Generate a VM manifest
cargo run -- generate my-vm --template fedora --cpus 4 --memory 8Gi

# Validate a configuration
cargo run -- validate examples/basic-vm.yaml

# Platform status (Cilium-style logo; needs a kubeconfig)
cargo run -- status
cargo run -- status my-vm

# Create from template (dry-run)
cargo run -- create test-vm --template ubuntu --dry-run

# List profiles
cargo run -- profiles

# List blueprints
cargo run -- blueprints

# Launch TUI
cargo run -- tui --interactive

# Serve API + SPA (local; needs a kube context, serves web/dist after `npm run build`)
cargo run --bin zorvia -- api api-serve --host 127.0.0.1 --port 5151

# Lab deploy
./scripts/deploy-remote.sh <host> sus --quick
# (same as ./deploy/remote-deploy.sh <host> sus --quick)
```

See [docs/WEB_CONSOLE.md](docs/WEB_CONSOLE.md) for the HTTP/WS surface.

## Design Decisions

1. **Handler-based architecture**: All command logic in `src/handlers/` modules, `lib.rs` is a thin router
2. **Separation of concerns**: Config types separate from Kubernetes CRDs
3. **Builder pattern**: Ergonomic programmatic VM creation
4. **Template system**: Uses `once_cell` for efficient template loading
5. **Validation first**: All configs validated before operations
6. **Async-first**: Built on Tokio for Kubernetes operations
7. **Proper error handling**: No unsafe `.unwrap()` in production code, `anyhow` for error propagation
