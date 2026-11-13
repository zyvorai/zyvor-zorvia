# Live fleet inventory (experimental)

`GET /api/v1/enterprise/fleet` and **Ops → Fleet** (`/app/fleet`) collect live
Kubernetes nodes and KubeVirt VirtualMachines. Reads require `cluster.admin`,
including scoped API tokens. The console is restricted to administrators.
This is an inventory feature; there are no remote power, migration, enrollment,
or failover APIs.

## Enable and enroll

Set both `ZORVIA_EXPERIMENTAL=1` and `ZORVIA_FEATURE_FLEET=1`. With no registry,
the fleet contains `local`, using Zorvia's existing client and configured VM
namespace. Nodes are cluster-wide. The local client's existing RBAC is unchanged;
a namespace-only deployment may show nodes as unknown.

For remote clusters, mount an administrator-managed JSON registry and set
`ZORVIA_FLEET_CONFIG=/etc/zorvia/fleet/registry.json`:

```json
[
  {
    "name": "east",
    "kubeconfig": "/etc/zorvia/fleet/east.yaml",
    "context": "east-readonly",
    "namespaces": ["workloads", "staging"],
    "environment": "production",
    "region": "east"
  }
]
```

Names must be unique DNS labels (`local` is reserved). Each enrollment requires
an absolute kubeconfig path, explicit context, and 1–32 unique namespaces.
At most 16 remote clusters are allowed. Each registry/kubeconfig is limited to
1 MiB. Registry errors prevent startup when fleet is enabled; a broken remote
kubeconfig affects only that cluster. Restart after registry changes.
Enroll each physical cluster once: identity is administrator assigned; two
contexts pointing at the same API server would double-count that inventory.

Mount kubeconfigs as **read-only Kubernetes Secrets**, separately from the
registry ConfigMap. They need HTTPS, verified TLS, and embedded credentials
(`token` or `client-certificate-data`/`client-key-data`) and, for private CAs,
`certificate-authority-data`. Exec credential plugins, auth providers, credential
file references, CA file references, insecure TLS, proxy overrides, and URLs
containing credentials/query/fragment are rejected. All entries in a kubeconfig
are checked; prefer a dedicated single-context kubeconfig. Never commit real
credentials or return kubeconfigs through HTTP. Remote kubeconfigs are reread
on every uncached refresh, allowing mounted Secret rotation. Static embedded
tokens must be rotated by the operator; this feature does not mint tokens.

The registry is trusted server configuration and deliberately supports private
Kubernetes API endpoints. It cannot be changed by browser requests. Restrict
network egress and file permissions according to your deployment.

## Minimal remote permissions

Use a dedicated identity. Bind this ClusterRole only to that identity:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: zorvia-fleet-nodes-reader
rules:
  - apiGroups: [""]
    resources: ["nodes"]
    verbs: ["list"]
```

For **each enrolled namespace**, bind this Role to the same identity:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: zorvia-fleet-vms-reader
  namespace: workloads
rules:
  - apiGroups: ["kubevirt.io"]
    resources: ["virtualmachines"]
    verbs: ["list"]
```

Create the corresponding ClusterRoleBinding and namespace RoleBindings with
your own ServiceAccount/user subject. No Secret, pod, VM mutation, or bootstrap
permissions are needed. The collector issues only LIST requests (HTTP GET).

## Results and limits

- All enrolled targets are probed concurrently (at most 17); each complete
  connection/inventory attempt has a 10-second deadline. No retries happen
  inside a probe. Concurrent HTTP requests share a refresh and a 15-second cache.
  Each server replica has its own cache. The console polls every 30 seconds
  while visible and labels responses older than 60 seconds as stale.
- Node and VM lists are paginated in pages of 500, with at most 5,000 nodes and
  5,000 VMs per cluster and 32 pages per list. A failed page, repeated token,
  expired list token, or exceeded limit discards that component's partial results.
  VM namespaces are explicit; one namespace failure makes that cluster's entire
  VM inventory unknown, rather than reporting a misleading subset as complete.
- Unknown counts/readiness are `null`; a successfully observed empty VM list is
  `0`. Components can succeed independently. A deadline makes the entire
  cluster unknown. Kubernetes/transport/configuration error details are redacted.
  API unavailability does not prove the cluster is down.
- `totals.observed_vms` and `observed_nodes` sum successful components only.
  `totals.partial` and `complete_clusters` describe inventory completeness.
  Treat partial totals as observed counts, never authoritative fleet totals.
- `healthy` means both inventory queries succeeded, at least one node exists,
  and all observed nodes have `Ready=True`; `degraded` means complete inventory
  with zero nodes or at least one node not Ready. `unknown` means an incomplete
  probe. This is not guest/application health or resource usage. VM readiness
  comes directly from `status.ready`; stopped VMs may legitimately be not Ready.
- The response exposes VM name, enrolled namespace, printable status and readiness,
  not VM specs, credentials, endpoint URLs, or raw node details. Searches retain
  cluster identity, and the console does not route remote VM names into local
  lifecycle actions. Results are observations, not a transactional snapshot
  spanning clusters or Kubernetes resource kinds.

## Lab dual-enrollment (same API, two identities)

On a single k3s/KubeVirt host you can exercise the remote probe path without a
second physical cluster: enroll the **same** API server twice under a different
name with a dedicated read-only ServiceAccount kubeconfig. This double-counts
inventory on purpose for lab proof only — do not use in production (see the
"enroll each physical cluster once" rule above).

```bash
# From a clone on the lab host (needs kubectl + cluster-admin):
./scripts/lab-fleet-dual-enroll.sh
# Then restart / redeploy Zorvia with:
#   ZORVIA_EXPERIMENTAL=1
#   ZORVIA_FEATURE_FLEET=1
#   ZORVIA_FLEET_CONFIG=/etc/zorvia/fleet/registry.json
# and mount the generated Secret + ConfigMap (script prints the kubectl apply).
curl -sk -H "Authorization: Bearer $TOKEN" \
  https://127.0.0.1:30152/api/v1/enterprise/fleet | jq .
```

Chaos check: delete the RoleBinding for the lab-alias SA and refresh — that
cluster should become `unknown` with redacted issues, not a 500.

## Validation before production

Unit/API tests exercise real Kubernetes client requests against mock servers,
pagination, permissions, partial failures, redaction, cache coalescing, and
configuration guards. Lab dual-enrollment (script above) exercises the remote
kubeconfig path on one host. Certificate/token rotation across true multi-API
clusters and outages still require a live multi-cluster trial. The feature
remains **Experimental**.
