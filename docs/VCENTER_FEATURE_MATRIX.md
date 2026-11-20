# Zorvia vCenter-style Operations Feature Matrix

Zorvia maps familiar vCenter operator workflows onto Kubernetes and KubeVirt primitives rather than cloning VMware APIs.

| Familiar vCenter capability | Zorvia capability | Native representation |
|---|---|---|
| Datacenter / Cluster / Folder inventory | Inventory Navigator | `inventory.zorvia.io/*` labels + live VMI placement |
| Tags | VM Tags | `tag.zorvia.io/<key>` labels |
| Custom attributes | VM Attributes | `attr.zorvia.io/<key>` annotations |
| Recent Tasks / Events / Alarms | Activity Center | Kubernetes Events, normalized and classified |
| Host Maintenance Mode | Maintenance Orchestrator | Node cordon + KubeVirt migration/controlled stop |
| DRS recommendations | Placement Advisor | K8s capacity + labels + KubeVirt VMI requests; recommendation-first |
| vMotion | Live migration | KubeVirt `VirtualMachineInstanceMigration` |
| Snapshots | Snapshots | KubeVirt snapshots + Zorvia CLI/UI |
| Templates / Content Library | Templates + CDI golden images | KubeVirt/CDI · [GOLDEN_IMAGES.md](GOLDEN_IMAGES.md) |
| Health / monitoring | Health + metrics | Kubernetes/KubeVirt telemetry · guest-insight |
| RBAC | Multitenancy / RBAC helpers | Kubernetes + Zorvia RBAC surfaces |

## CLI

```bash
zorvia inventory show
zorvia inventory show --datacenter dc1 --cluster rack-a
zorvia inventory tag-set myvm env prod
zorvia inventory attribute-set myvm owner platform
zorvia inventory inventory-datacenter-set myvm dc1
zorvia inventory activity --limit 50
zorvia maintenance maintenance-plan worker-3
zorvia maintenance maintenance-enter worker-3 --dry-run
zorvia maintenance maintenance-exit worker-3
zorvia placement placement-advisor --cpu 2 --memory-gib 4
zorvia placement placement-rebalance --threshold 0.15
```

## Metadata conventions

- `inventory.zorvia.io/datacenter`
- `inventory.zorvia.io/cluster`
- `inventory.zorvia.io/folder`
- `tag.zorvia.io/<tag>`
- `attr.zorvia.io/<attribute>`
- `maintenance.zorvia.io/policy`: `migrate`, `stop`, `skip`, `block`
- `maintenance.zorvia.io/priority`: `0..255`

The maintenance `--force` switch is intentionally explicit. It can stop a workload whose policy otherwise blocks a complete evacuation; gate it with RBAC and change approval in production.

## Related

- [GOLDEN_IMAGES.md](GOLDEN_IMAGES.md) · [SNAPSHOTS.md](SNAPSHOTS.md) · [GUEST_INSIGHT.md](GUEST_INSIGHT.md) · root [README.md](../README.md)
