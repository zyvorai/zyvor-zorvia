# Resource pools (Experimental)

A resource pool is a named set of VMs with optional quota-like limits (max VMs, vCPUs,
memory). It sits beside the label-based inventory (`inventory.zorvia.io/datacenter|cluster|folder`,
see [VCENTER_FEATURE_MATRIX.md](VCENTER_FEATURE_MATRIX.md)) and adds a fourth level to the view:
datacenter > cluster > folder > pool. No new CRDs.

Enable with `ZORVIA_EXPERIMENTAL=1` on the API server (otherwise 403 `EXPERIMENTAL_DISABLED`).
All routes need `cluster.admin`, even `GET`, because they list and relabel VMs of every namespace.
The service account needs `list` on `virtualmachines` in all namespaces and `patch` on them
(the same as `zorvia inventory show -A` and the tag/folder commands).

## Where the data lives (and why)

| Data | Where | Why |
|---|---|---|
| Membership | VM label `pool.zorvia.io/<pool>=member` | Same mechanism as datacenter/cluster/folder: visible to `kubectl`, survives without Zorvia, no second source of truth. A VM can be in several pools. Only the value `member` counts. |
| Definition (description, limits) | One JSON document in the shared state store (`resource_pools.json`: a file under the data directory, or PostgreSQL with `ZORVIA_DATABASE_URL`) | Labels hold short strings; limits are numbers plus a description. A ConfigMap would need new `configmaps` write RBAC, which Zorvia does not otherwise have for this. Same store and caveats as DR pairs and warm pools (concurrent edits from two replicas are last-writer-wins, see [POSTGRES.md](POSTGRES.md)). |

A label with no definition is still a pool: it is listed as `defined: false`, unlimited. Deleting
a definition never touches labels unless you ask (`unassign`).

## Limits are advisory

Limits are evaluated by a read-only report and are **never enforced by Kubernetes**.

- **Report**: usage counts every member VM, running or not (a stopped VM can start): VM count,
  running count, vCPUs (cores x sockets x threads), and memory from the VM's memory request.
  A VM without a readable memory request counts as 0 MiB and is shown in `unknown_memory_vms`.
  Per limit: `ok`, `near` (>= 90 %), `exceeded` (used > limit; exactly at the limit is ok).
  Pool status is the worst of its limits, or `unlimited` when it has none.
- **VM create**: when the create request's `labels` include `pool.zorvia.io/<pool>=member` for a
  defined pool, the response carries `pool_warnings` if this VM would push the pool over a limit.
  The create is **not** blocked. This is advisory on purpose: the check reads the VM list and the
  state store without a lock, so two concurrent creates can both pass; VMs created with
  `kubectl` or GitOps bypass Zorvia entirely; and memory/vCPU of members cannot always be read.
  A hard refusal built on that would be wrong in both directions. If you need hard limits, use
  a Kubernetes `ResourceQuota`/admission policy per namespace.

## API (`/api/v1/inventory/pools`)

| Route | Purpose |
|---|---|
| `GET /` | pools with usage, limits and status (no member rows) |
| `POST /` | `{name, description?, max_vms?, max_vcpus?, max_memory_mib?}` (name: lowercase DNS label, limits 1..max) |
| `GET /{name}` and `GET /{name}/report` | usage versus limits with member VMs (404 if neither defined nor labelled) |
| `GET /hierarchy` | datacenter > cluster > folder > pool, VM and vCPU/memory totals per pool node, unpooled count per folder |
| `POST /{name}/assign` | `{vm, namespace?, remove?}` adds or removes the label; joining needs a defined pool |
| `DELETE /{name}[?unassign=true]` | removes the definition; 409 `POOL_NOT_EMPTY` while VMs are members unless `unassign=true`, which first removes the label from each member (the definition is kept if any removal fails) |

Create, delete and assign are audited (`resource-pool`). In the hierarchy a VM in several pools is
counted under each of them. Names `hierarchy` and `report` are reserved.

## CLI

```
zorvia inventory pool list
zorvia inventory pool create web --max-vms 10 --max-vcpus 40 --max-memory-mib 65536 --description "web tier"
zorvia inventory pool assign web vm1 --namespace default
zorvia inventory pool report web
zorvia inventory pool hierarchy
zorvia inventory pool unassign web vm1
zorvia inventory pool delete web [--unassign]
```

Uses `--url`/`ZORVIA_URL`, `--token`/`ZORVIA_TOKEN`, `--output` like the other API-backed
commands ([CLI_REMOTE.md](CLI_REMOTE.md)). Unlike `inventory show` it talks to the API, because
definitions live on the server.

## Not done / limits

- Not tested against a live cluster; tested with fake VM objects and a mock API server only.
- No enforcement (see above), no per-namespace or nested pools, no pool-level permissions
  (everything is `cluster.admin`), no update-limits route (delete and recreate).
- No web console page: the console has no Inventory page to extend, so a pool view would be a new
  page, not a small addition.
- Memory is the VM's memory request, not live usage.
