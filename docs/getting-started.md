# Install and quick start

Build Zorvia, deploy it with Helm, create a first VM and reach the web console.

[Back to the README](../README.md) · [Docs index](README.md)

## Install

One binary, one Helm chart — bring your own KubeVirt cluster.

```bash
git clone https://github.com/zyvorai/zorvia.git
cd zorvia
cargo install --path . --features web   # `web` is the default feature
# or: cargo build --release --features web → target/release/zorvia
```

Requires Rust **1.89+**, a kubeconfig, and a cluster with **KubeVirt** (CDI optional for golden images / CDI clone).

**Helm (cluster install):**

```bash
helm upgrade --install zorvia charts/zorvia -n zorvia-system --create-namespace \
  -f charts/zorvia/values-lab.yaml
```

Production values: `charts/zorvia/values-production.yaml`. Lab remote deploy: [docs/LAB.md](LAB.md).

---

## Quick start

Profile → create → start → reach the guest. Five commands, one real VM.

```bash
# Size from a profile, then create
zorvia profile show database
zorvia vm create prod-db \
  --template ubuntu-22.04 \
  --cpus 6 --memory 16Gi --disk-size 200Gi
zorvia vm start prod-db
zorvia guest wait-ready prod-db --timeout 120
zorvia guest guest-insight prod-db
zorvia vm status                 # platform components + features
zorvia vm status prod-db --watch
```

Multi-VM stack:

```bash
zorvia blueprint blueprints
zorvia blueprint deploy lamp --prefix myapp --start
zorvia vm list
```

Web console (HTTPS NodePort **30152**, self-signed — use `curl -sk`):

```bash
./scripts/deploy-remote.sh <host> sus --quick
# or: make deploy-remote H=<host> U=sus ARGS=--quick
open https://<HOST>:30152/app
```

| Port | Role |
|------|------|
| **30152** | Cluster front door (UI + API) |
| **5151** | In-pod listen / `zorvia api api-serve --tls --port 5151` |
| **8080** | Config-file default when `--port` is omitted |

```bash
# Health + login + list VMs
curl -sk https://HOST:30152/api/v1/health | jq .
TOKEN=$(curl -sk -X POST https://HOST:30152/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"…"}' | jq -r .token)
curl -sk -H "Authorization: Bearer $TOKEN" https://HOST:30152/api/vms | jq .
curl -sk -H "Authorization: Bearer $TOKEN" \
  'https://HOST:30152/api/audit/export?format=jsonl' | head
```

Lab admin password comes from the `zorvia-auth` Secret (printed once on first deploy) — see [docs/LAB.md](LAB.md). Change credentials before anything shared.
