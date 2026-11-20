# Rescue mode — offline guest-disk operations

**Rescue mode** runs a handful of offline configuration changes against a
**stopped** VM's own disk — set its hostname, inject an SSH public key, or
enable the SSH service — without booting the guest or needing an in-guest
agent. It works by creating a privileged Kubernetes Job that mounts the
VM's PVC and calls [GuestKit](https://github.com/zyvorai/guestkit) directly
against the disk image, the same way an operator would run `virt-customize`
or `guestfish` by hand. It is **admin-only** end to end, the same as the
Pods page (see [PODS.md](PODS.md) for the parallel pattern this mirrors).

- [Where to find it](#where-to-find-it)
- [What it does today](#what-it-does-today)
- [What's deferred](#whats-deferred)
- [HTTP API](#http-api)
- [Security model](#security-model)
- [Kubernetes RBAC](#kubernetes-rbac)
- [The `guestkit` dependency](#the-guestkit-dependency)
- [Audit trail](#audit-trail)
- [Testing](#testing)
- [Implementation map](#implementation-map)

## Where to find it

The **Rescue** tab on a VM's detail page (`/app/vms/<name>`, next to Console/
Snapshots/etc.). Every action is disabled unless the VM is **stopped** — a
running VM's PVC is already attached to `virt-launcher`, so the rescue Job
can't get exclusive access to it — and unless the caller has write access.

## What it does today

| Operation | What happens | GuestKit call |
|-----------|--------------|----------------|
| **Inject SSH Key** | Overwrites `~<user>/.ssh/authorized_keys` with the given key (not appended — matches upstream's single-key field) | `Guestfs::set_ssh_authorized_keys` |
| **Set Hostname** | Writes `/etc/hostname` and updates the `127.0.1.1` line in `/etc/hosts` | `Guestfs::set_hostname` |
| **Enable SSH Service** | Symlinks `ssh.service` (falling back to `sshd.service`) into `multi-user.target.wants/` — the same trick GuestKit's own CLI uses (`cli/plan/firstboot_stage.rs`), no chroot needed | `mkdir_p` + `ln_sf` |

Each request creates a short-lived Job, returns `202 {job_name, state:
"queued"}` immediately, and the caller polls `GET .../rescue/{job_name}`
until it reaches `succeeded`/`failed`. The Job itself never processes more
than one operation and self-deletes after an hour
(`ttl_seconds_after_finished`).

## What's deferred

These are still on the tab, marked "Not yet available" rather than calling
a route that doesn't exist:

- **Reset Password** — GuestKit has no direct Linux password mutator (only
  Windows, via a `registry-write` libhivex FFI feature not enabled here).
  Would need a chroot + `chpasswd`/`usermod -p` approach, which is real
  engineering, not wiring.
- **Install Packages** — needs guest network egress from inside an already-
  privileged Job, plus a guest package manager to shell into. Deferred
  pending a real design for that, not attempted here.
- **Inspect Disk** — needs its own research into GuestKit's inspection API
  (`inspect_os`/`inspect_get_mountpoints` are used internally by the Job to
  find and mount the guest filesystem, but a rich read-only "disk info"
  view is a separate piece of work not yet investigated).

## HTTP API

All routes need a bearer token with `cluster.admin` — including the `GET`
status route, since it reveals whether/what a rescue operation did.

### `POST /api/vms/{name}/rescue`

```json
{ "operation": "set-hostname", "hostname": "new-name" }
{ "operation": "inject-ssh-key", "user": "root", "key": "ssh-ed25519 AAAA..." }
{ "operation": "enable-ssh" }
```

```json
{ "job_name": "zorvia-rescue-web-01-...", "state": "queued" }
```

`409 VM_NOT_STOPPED` if the VM isn't stopped; `409 NO_RESCUABLE_DISK` if the
VM has no PVC- or DataVolume-backed disk (an `emptyDisk`/`containerDisk`
VM has nothing to mount).

### `GET /api/vms/{name}/rescue/{job_name}`

```json
{ "state": "succeeded", "result": { "success": true, "operation": "set-hostname", "message": "hostname set to 'new-name'" } }
```

`state` is `pending`/`running`/`succeeded`/`failed`. `result` is only
present once the Job's pod has produced its single JSON result line
(read via the same `Api<Pod>::logs()` kube-rs call the Pods page already
uses — no separate result-passing infrastructure).

### `DELETE /api/vms/{name}/rescue/{job_name}`

Best-effort early cleanup (the Job also self-expires).

### Errors

| Status | Code | When |
|--------|------|------|
| 400 | `INVALID` | VM/job name is not a valid Kubernetes name, or the request body failed validation (e.g. key doesn't look like an SSH public key) |
| 403 | — | Authenticated but not `cluster.admin` |
| 404 | `NOT_FOUND` | VM or Job does not exist |
| 409 | `VM_NOT_STOPPED` | VM is not stopped |
| 409 | `NO_RESCUABLE_DISK` | VM has no PVC/DataVolume-backed disk |

## Security model

Rescue mode creates a privileged Job that mounts an arbitrary PVC and runs
a fixed binary against it — exec-equivalent sensitivity, gated the same way:

1. **REST RBAC** — `required_permission` in `src/api/auth/permissions.rs`
   maps every `/vms/*/rescue*` path (any method) to `ApiPermission::ClusterAdmin`,
   ahead of the "reads are free" fallback. Unit test:
   `rescue_mode_requires_cluster_admin_even_on_get`.
2. **Fixed argv, no shell strings** — the Job's container always runs the
   same `rescue-agent` binary; operation parameters travel as environment
   variables the server sets, never a client-supplied command string.
3. **Input validation** — VM/job names are DNS-1123 checked before
   reaching a Kubernetes API path; the SSH username is restricted to
   `[A-Za-z0-9_-]`, the key must look like an SSH public key.
4. **VM-stopped guard, enforced server-side** — the frontend already
   disables every action unless the VM is stopped; the handler checks
   again independently (defense in depth, not just UX).
5. **Privileged, but not maximally so** — the Job's container requests
   `SYS_ADMIN`+`SYS_RESOURCE`, not full `privileged: true`, since that's
   what GuestKit's own disk-mount path (`losetup`/`qemu-nbd`/`mount`/
   `chroot`) needs; no `/dev/kvm`, no `NET_ADMIN`.
6. **Audit** — every request is recorded, success and failure (below).

## Kubernetes RBAC

The `zorvia` ServiceAccount needs one additional rule beyond what it
already has:

```yaml
- apiGroups: ["batch"]
  resources: ["jobs"]
  verbs: ["get", "list", "watch", "create", "delete"]
```

No new PVC permission is needed — the existing `persistentvolumeclaims`
get/list/create/update/patch rule already covers looking up (and mounting,
via the Job/Pod spec itself) an existing PVC by name; mounting a PVC into a
pod is authorized by the pod-create permission, not a separate rule.

This rule ships in `deploy/k8s.yaml`, `deploy/k3s-zorvia-web.yaml`, and the
Helm chart (`charts/zorvia/templates/rbac.yaml`, both cluster-scoped and
namespaced modes). **Existing installs**: re-apply the manifest / `helm
upgrade`, or patch the live role:

```bash
kubectl patch clusterrole zorvia --type=json -p='[
  {"op":"add","path":"/rules/-","value":{"apiGroups":["batch"],"resources":["jobs"],"verbs":["get","list","watch","create","delete"]}}
]'

SA=system:serviceaccount:zorvia-system:zorvia
kubectl auth can-i create jobs.batch --as=$SA -A   # yes
```

Without this rule, `POST /vms/{name}/rescue` returns `403 FORBIDDEN`;
everything else keeps working.

## The `guestkit` dependency

The `rescue-agent` binary (the Job's entrypoint, `src/bin/rescue_agent.rs`)
depends on [GuestKit](https://github.com/zyvorai/guestkit) — a real, mature
sibling project, same author. It is **not** on the same dependency graph as
the main `zorvia`/`zorvia-api` server: `guestkit` is an `optional`
dependency gated behind the `rescue-agent` Cargo feature, and `cargo build`
(default features) never fetches or compiles it.

**Pinned to a `git` revision, not a crates.io version**, deliberately: the
`guestkit` crate published on crates.io (0.3.x) is stale, comes from a
different (personal) repository, and is licensed **LGPL-3.0-or-later** —
not the **Zyvor Production License v1.0** code in the current `zyvorai/guestkit` source this
is pinned to. Repin to a real crates.io release once one exists under the
current license and org. See `Cargo.toml`'s comment on the `guestkit`
dependency line for the exact pinned commit.

The `rescue-agent` container image (`deploy/rescue-agent.Dockerfile`) is
built and pushed by a separate, best-effort CI job
(`rescue-agent-docker` in `.github/workflows/release.yml`) — it does not
block the main zorvia release if it fails.

**Verified before relying on this for a real VM**: what exact file within
the mounted PVC holds the raw/qcow2 disk image depends on the CSI driver
and whether KubeVirt provisioned it via a DataVolume or a raw PVC —
`src/kube/rescue.rs`'s `build_rescue_job` currently assumes `disk.img` at
the PVC's mount root; confirm this against your own cluster's storage
class before trusting a rescue operation on a production VM.

## Audit trail

| Event | `action` | Severity | `resource_name` | `details` |
|-------|----------|----------|------------------|-----------|
| Rescue Job requested | `exec` | **High** | `<vm name>` | `{"operation":"set-hostname","job_name":"...","pvc":"..."}` (`success=false` if Job creation itself failed) |

`resource_type` is `vm`, `user` is the signed-in username (`api-token` for
token principals). The Job's own outcome (succeeded/failed) is not a
separate audit entry — poll `GET .../rescue/{job_name}` for that, or check
the Job/Pod directly with `kubectl`.

## Testing

- **Rust**: `cargo build --features rescue-agent --bin rescue-agent` (only
  this pulls in the `guestkit` git dependency and compiles the agent
  binary); `cargo test --features web --lib -- rescue_handlers permissions::tests`.
- **Live verification requires a real cluster with a PVC-backed test VM**
  (create one via the existing "Existing PVC" disk-source option in
  CreateVM.tsx, or `POST` a VM with `"image": "pvc:<name>"`) — this cannot
  be exercised against the default `emptyDisk` VMs. Stop the VM, trigger
  each operation through the real API, then start it and confirm the
  change landed (or check via a snapshot).

## Implementation map

| Layer | File |
|-------|------|
| Job entrypoint (runs inside the privileged Job pod) | `src/bin/rescue_agent.rs` |
| PVC resolution + Job spec builder | `src/kube/rescue.rs` |
| REST handlers | `src/api/http_server/web/rescue_handlers.rs` |
| Route registration | `src/api/http_server.rs` (`/vms/{name}/rescue[/{job_name}]`) |
| Permission gate | `src/api/auth/permissions.rs` |
| Container image | `deploy/rescue-agent.Dockerfile` |
| CI build/push | `.github/workflows/release.yml` (`rescue-agent-docker` job) |
| API client | `web/src/api/guestRescue.ts` |
| UI | `web/src/pages/vm-details/RescueTab.tsx` |
