# Lab deploy & smoke

Lab front door is Kubernetes in-pod HTTPS on NodePort **30152** (self-signed — use `curl -sk`).

```text
UI:       https://<HOST>:30152/
Sign-in:  https://<HOST>:30152/sign-in
Health:   https://<HOST>:30152/api/v1/health
Features: https://<HOST>:30152/api/v1/features
```

## Deploy

```bash
# From a machine with SSH to the lab host (wrapper → deploy/remote-deploy.sh)
./scripts/deploy-remote.sh <host> sus --quick
# or
make deploy-remote H=<host> U=sus ARGS=--quick
```

On first apply, remote deploy mints a random admin password into Secret `zorvia-auth` (printed once in deploy logs). Retrieve later:

```bash
kubectl -n zorvia-system get secret zorvia-auth \
  -o jsonpath='{.data.admin-password}' | base64 -d; echo
```

Lab mode (`ZORVIA_LAB_MODE=1`) is **off** in the shipped manifests: it allows known default passwords and a shared `ZORVIA_API_KEY` with full admin. The deploy script creates random credentials, so you do not need it. The API refuses fixed lab passwords outside lab mode — use `./scripts/create-auth-secret.sh` before applying manifests.

Auth + audit SQLite (or PostgreSQL, [POSTGRES.md](POSTGRES.md)) live on PVCs (`zorvia-auth-data`, audit DB path via `ZORVIA_AUDIT_DB`) so accounts and the audit trail survive restarts.

**Helm alternative:**

```bash
helm upgrade --install zorvia charts/zorvia -n zorvia-system --create-namespace \
  -f charts/zorvia/values-lab.yaml \
  --set auth.existingSecret=zorvia-auth
```

## Smoke

```bash
export ZORVIA_LAB_HOST=<host>   # default 175.110.122.71
export ZORVIA_E2E_PASSWORD='…'  # from zorvia-auth Secret
./scripts/lab-smoke.sh
```

Checks health, feature maturity (`audit-trail`), login, and `GET /api/audit/export`.

Manual:

```bash
curl -sk https://HOST:30152/api/v1/health | jq .
TOKEN=$(curl -sk -X POST https://HOST:30152/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"admin\",\"password\":\"$ZORVIA_E2E_PASSWORD\"}" | jq -r .token)
curl -sk -H "Authorization: Bearer $TOKEN" https://HOST:30152/api/vms | jq '.[0:3]'
curl -sk -H "Authorization: Bearer $TOKEN" \
  'https://HOST:30152/api/audit/export?format=jsonl&limit=5'
```

## Ports

| Port | Role |
|------|------|
| **30152** | Cluster front door (UI + API NodePort) |
| **5151** | In-pod listen (`zorvia api api-serve --tls --port 5151`) |
| **8080** | Config-file default when `--port` is omitted |

Set `ZORVIA_EXPOSE_HOST` so the UI shows the correct host for NodePort SSH/VNC/RDP.

See also [WEB_CONSOLE.md](WEB_CONSOLE.md), [OIDC_LAB.md](OIDC_LAB.md), [FEATURE_MATURITY.md](FEATURE_MATURITY.md).
