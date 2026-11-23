# Zorvia Helm chart

Production-oriented packaging for the Zorvia API/console.

## Lab install

```bash
./scripts/create-auth-secret.sh
helm upgrade --install zorvia ./charts/zorvia \
  -n zorvia-system --create-namespace \
  -f charts/zorvia/values-lab.yaml \
  --set auth.existingSecret=zorvia-auth \
  --set api.exposeHost=<node-ip>
```

## Production install

```bash
./scripts/create-auth-secret.sh   # unique credentials; do NOT set ZORVIA_LAB_MODE
helm upgrade --install zorvia ./charts/zorvia \
  -n zorvia-system --create-namespace \
  -f charts/zorvia/values-production.yaml \
  --set auth.existingSecret=zorvia-auth \
  --set image.repository=ghcr.io/zyvorai/zorvia \
  --set image.tag=0.3.3 \
  --set 'httpRoute.hostnames[0]=zorvia.example.com'
```

### Setura prerequisite

Production values expose Zorvia through [Setura](https://github.com/zyvorai/zyvor-setura-gateway) (a Gateway API
controller with a built-in proxy) using the Gateway API (`HTTPRoute` + `Gateway`):

```bash
# Gateway API CRDs (Setura does not ship them; skip if the cluster already has them)
kubectl apply -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.4.0/standard-install.yaml

# Controller, ServiceAccount/RBAC and the `setura` GatewayClass (controller zyvor.dev/setura-gateway)
setura gateway install --image ghcr.io/zyvorai/zyvor-setura-gateway:0.7.0 \
  --namespace setura-system | kubectl apply -f -
```

Without the `setura` GatewayClass the Gateway never becomes Accepted. For lab hosts,
`deploy/remote-deploy.sh <host> <user> --bootstrap` installs k3s (no flannel), Cilium, KubeVirt, CDI,
Setura and this GatewayClass before deploying. The Gateway listens on 80 and 443, which is what the
`setura gateway install` Service exposes.

The chart creates a `Gateway` (class `setura`) with an HTTP listener that redirects to HTTPS and an
HTTPS listener using the `zorvia-tls` Secret (cert-manager `gateway-shim` via the
`cert-manager.io/cluster-issuer` annotation). To attach to an existing Gateway instead, set
`httpRoute.gateway.create=false` and `httpRoute.parentRefs`. When `networkPolicy.enabled=true`, set
`networkPolicy.ingressFrom` to allow the `setura-system` namespace.

## Features

| Value | Lab | Production |
|-------|-----|------------|
| replicas | 1 | 2 |
| TLS | in-pod self-signed | Setura + cert-manager |
| Service | NodePort 30152 | ClusterIP |
| PDB / NetworkPolicy / anti-affinity | off | on |
| Leader election (Lease) | on | on |
| Persistent audit DB | `/data/audit.db` | same |
| Rook bootstrap SA | opt-in (`rbac.rookBootstrap`) | opt-in |

### Optional env (set via `extraEnv` / values)

| Variable | Purpose | Docs |
|----------|---------|------|
| `ZORVIA_OIDC_ENABLED=1` + issuer/client/secret/redirect | Opt-in OIDC SSO | [docs/OIDC.md](../../docs/OIDC.md) |
| `ZORVIA_EXPERIMENTAL` + `ZORVIA_FEATURE_*` | Phase 5 plan APIs (non-prod) | [docs/PHASE5_ENTERPRISE.md](../../docs/PHASE5_ENTERPRISE.md) |

Raw manifests in `deploy/k8s.yaml` remain for the existing remote-deploy path.
