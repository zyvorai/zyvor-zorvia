# Adopting an existing KubeVirt cluster

**Status: OpenShift Virtualization install path is charted but not lab-proved on a
live OCP cluster.** Zorvia is verified on plain Kubernetes with KubeVirt. The
OpenShift templates below are ready to apply; treat them as a checklist until an
OCP run is recorded in [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md).

Zorvia reads and writes standard KubeVirt objects, so an existing cluster's VMs
appear without conversion. Go in stages.

## 1. Read-only first

Create a kubeconfig or ServiceAccount that can only `get`/`list`/`watch` these
resources:

- `virtualmachines`, `virtualmachineinstances` (`kubevirt.io`)
- `datavolumes` (`cdi.kubevirt.io`)
- `persistentvolumeclaims`, `nodes`, `namespaces`

Install with the lab values, then sign in and confirm the VM list matches
`kubectl get vm -A` (or `oc get vm -A`):

```bash
helm upgrade --install zorvia charts/zorvia -n zorvia-system --create-namespace \
  -f charts/zorvia/values-lab.yaml
```

`GET /api/v1/adopt/report` (or `zorvia adopt`) lists GA/Beta capabilities and
Experimental gaps that matter when leaving another platform (`vm-import-h2kvm`,
`fleet-multicluster`, `cross-cluster-dr`, `gpu-sriov-numa`, …).

## 2. Check on OpenShift

Use the OpenShift values overlay (Route + optional SCC):

```bash
helm upgrade --install zorvia charts/zorvia -n zorvia-system --create-namespace \
  -f charts/zorvia/values-production.yaml \
  -f charts/zorvia/values-openshift.yaml \
  --set openshift.route.host=zorvia.apps.example.com \
  --set image.repository=ghcr.io/zyvorai/zorvia \
  --set image.tag=<version> \
  --set image.pullPolicy=IfNotPresent
```

| Check | What to verify |
|-------|----------------|
| **SCC** | `openshift.scc.create=true` installs a UID-10001 SCC bound to the release SA, or set `openshift.scc.bindExisting=anyuid` / `nonroot`. Confirm the pod is not rejected (`oc describe pod`). |
| **Route** | `openshift.route.enabled` creates a **passthrough** Route (Zorvia terminates TLS). Wait for `Admitted=True`. |
| **Storage / CDI** | StorageClasses used by VMs exist; CDI installed. |
| **KubeVirt** | OpenShift Virtualization operator healthy; VMs list in the console. |

Without a live OCP cluster this remains **unverified** — do not claim production
readiness on OpenShift until SUPPORT_MATRIX records a pass.

## 3. Then day-2

Once the read-only view is right, grant write roles per the RBAC in
[WEB_CONSOLE.md](WEB_CONSOLE.md) and try snapshots and live migration on a
non-critical VM first.

## Limits

- Live migration needs shared storage (same as [FEATURE_MATURITY.md](FEATURE_MATURITY.md)).
- Cross-cluster move of VMs is Experimental (`cross-cluster-dr` same-cluster
  rehearsal only). For multi-site or commercial migration help,
  [talk to sales](https://zyvor.dev/contact?intent=sales&utm_source=github&utm_medium=zorvia&utm_campaign=docs_adopt).

Back to [Leaving OpenShift](leave-openshift.md).
