# Upgrade notes

## Unreleased (after v0.7.4)

### Required ops
1. **Minimum Kubernetes API server is now 1.33** (was 1.32). `k8s-openapi` moved from 0.28 to 0.29, which has no `v1_32` feature; `kube` 5.0 already required 0.29. The lab runs v1.36.
2. **Rescue reset-password needs new RBAC.** The Zorvia ServiceAccount gains `secrets` `create`, `delete` and `patch` in its namespace (Helm: `charts/zorvia/templates/rbac.yaml`; manifests: `deploy/k8s.yaml`, `deploy/k3s-zorvia-web.yaml`). Existing installs must re-apply the RBAC. Only the new experimental rescue operations use it.
3. **Volume encryption** also stores wrapped data keys in Secrets (`zorvia-dek-<vm>-<volume>`), so it needs the same Secret permissions. See [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md).

4. **Certificate lifecycle** (optional) needs `get`/`list` on `secrets` (cluster-wide or per namespace), `list` on `namespaces`, and for cert-manager `get`/`list` on `certificates` plus `update` on `certificates/status`. The chart does not grant these; see [CERTIFICATE_LIFECYCLE.md](CERTIFICATE_LIFECYCLE.md).

### Behaviour to know
- All seven new features are off unless `ZORVIA_EXPERIMENTAL=1` plus the feature's own switch (see [FEATURE_MATURITY.md](FEATURE_MATURITY.md)); nothing changes for an install that sets neither.
- **SAML:** `ZORVIA_SAML_ENABLED=1` exposes metadata and a login redirect, but the ACS endpoint answers 501 and creates no session. Keep using OIDC.
- **Autoscaler loop:** it only recommends, even with `ZORVIA_AUTOSCALER_APPLY=1`, until a real metrics source replaces the simulated one.
- `deploy/rescue-agent.Dockerfile` builds with `--no-default-features --features rescue-agent`, which fails; build with default features plus `rescue-agent` until it is fixed.

## v0.3.2 → v0.3.3+ (Production Foundation)

### Breaking / required ops

1. **Auth Secret is no longer in manifests.** Create it before upgrade:
   ```bash
   ./scripts/create-auth-secret.sh
   ```
2. **Lab defaults refused outside lab mode.** Production must omit `ZORVIA_LAB_MODE` and use unique JWT/admin passwords. The shipped `deploy/k8s.yaml` and `deploy/k3s-zorvia-web.yaml` no longer set it; if you relied on it (shared `ZORVIA_API_KEY`, known default password), add it back yourself on a throwaway lab only.
3. **OIDC is opt-in.** Set `ZORVIA_OIDC_ENABLED=1` plus issuer/client/secret/redirect to enable
   PKCE + token exchange + JWKS. Without the enable flag, `ZORVIA_OIDC_*` is ignored.
4. **Shared `ZORVIA_API_KEY` ignored** unless `ZORVIA_LAB_MODE=1`. Prefer `POST /api/v1/api-tokens`.
5. **Viewer role** can no longer call mutating APIs (server-side RBAC).
6. **Rook bootstrap** privileges removed from the core ServiceAccount. Apply `deploy/rook-bootstrap-rbac.yaml` or Helm `rbac.rookBootstrap=true` only if needed.
7. **TOTP disable** now requires password + TOTP code; sessions are revoked via `token_version`.
8. **Console, VNC and SSH sockets are permissioned.** `/ws/console` and `/ws/vnc` need `vm.power`, `/ws/ssh` needs `cluster.admin`. Viewers lose access.
9. **Audit logs, webhooks, VM logs are no longer readable by Viewers** (`cluster.admin`; VM logs `vm.power`). Deleting backups and snapshots needs `vm.delete`.
10. **`ZORVIA_JWT_SECRET` must be 32+ bytes.** A shorter secret is ignored (ephemeral secret, sessions reset on restart).
11. **Failed-login lockout** (8 attempts / 15 min per username) and TOTP re-enrolment now requires password + current code.
12. **OIDC** sign-in needs cookies enabled on the console origin, and the token now arrives as `/sign-in#oidc_token=…`.
13. **Helm: `replicaCount > 1` requires a ReadWriteMany volume**, and `values-production.yaml` now sets one replica with `strategy: Recreate` and no PodDisruptionBudget. See [HA.md](HA.md).

### Compatible

- Existing SQLite `auth.db` migrates in place (`token_version`, `api_tokens` tables).
- KubeVirt VM CRs unchanged.
- Helm chart (`charts/zorvia`) is preferred for new installs; `deploy/k8s.yaml` remains for lab remote-deploy.

### Recommended verification

```bash
curl -sk https://<host>:30152/api/v1/health
curl -sk https://<host>:30152/api/v1/features | jq .
curl -sk https://<host>:30152/api/v1/auth/providers   # [] unless OIDC enabled
# Viewer must get 403 on POST /api/vms
# Enterprise plans return 403 without ZORVIA_EXPERIMENTAL + ZORVIA_FEATURE_*
```

### New optional surfaces (0.3.3+)

| Surface | How to enable | Docs |
|---------|---------------|------|
| OIDC SSO | `ZORVIA_OIDC_ENABLED=1` + issuer/client/secret/redirect | [OIDC.md](OIDC.md) |
| Enterprise plan APIs | `ZORVIA_EXPERIMENTAL=1` + `ZORVIA_FEATURE_*` | [PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) |
| Feature registry | always on | [FEATURE_MATURITY.md](FEATURE_MATURITY.md) |
| Pods page (logs, exec, events, YAML, restart/delete) | admin role + `pods` delete / `pods/log` / `pods/exec` RBAC | [PODS.md](PODS.md) |
| Rescue mode (hostname/SSH-key/enable-SSH) | admin role + `batch/jobs` RBAC | [RESCUE.md](RESCUE.md) |

### Rescue mode

The admin-only **Rescue** tab (on a VM's detail page) needs one new rule on
the `zorvia` role — `batch` `jobs` (get, list, watch, create, delete), used
to run a short-lived privileged Job per rescue request. It ships in
`deploy/k8s.yaml`, `deploy/k3s-zorvia-web.yaml` and the Helm chart; `helm
upgrade` / `kubectl apply` picks it up. For an in-place patch of a live
cluster see [RESCUE.md → Kubernetes RBAC](RESCUE.md#kubernetes-rbac).
Without it, `POST /vms/:name/rescue` returns `403 FORBIDDEN`; everything
else keeps working. This also needs the `rescue-agent` container image
(`ghcr.io/zyvorai/zorvia-rescue-agent`) to be reachable from your cluster —
see [RESCUE.md → The guestkit dependency](RESCUE.md#the-guestkit-dependency)
for how it's built.

### Pods page (logs & exec)

The admin-only **Pods** page needs three new rules on the `zorvia` role —
`pods` (delete, for Restart/Delete), `pods/log` (get) and `pods/exec` (create, get). They ship in `deploy/k8s.yaml`,
`deploy/k3s-zorvia-web.yaml` and the Helm chart; `helm upgrade` / `kubectl apply`
picks them up. For an in-place patch of a live cluster see
[PODS.md → Kubernetes RBAC](PODS.md#kubernetes-rbac). Without them the page still
lists pods, shows events and YAML, but log streams, shells and restart/delete fail
with `forbidden`.

`pods/exec` and `pods` delete are powerful grants (shell into / remove any pod the role
can see). Zorvia gates them behind `cluster.admin` and audits every session and deletion;
omit either rule to switch that capability off — the rest of the page keeps working.

You don't have to guess whether a cluster is patched: the Pods page checks the
ServiceAccount's rights (`GET /api/v1/pods/capabilities`) and, when a rule is missing,
greys out the affected buttons and shows a yellow notice naming the exact rule.

### Matrix

| Component | From | To |
|-----------|------|-----|
| zorvia | 0.3.2 | 0.3.3+ |
| Kubernetes | 1.28+ | 1.28+ |
| KubeVirt | 1.2+ | 1.2+ |

### Dependency bumps (0.3.3+)

Mid-risk crates refreshed earlier (dialoguer 0.12, indicatif 0.18, ratatui 0.30, rustyline 18, dirs 6, bcrypt 0.17, rusqlite 0.37). **Follow-up:** kube **4.2** / k8s-openapi **0.28** (API feature `v1_32`) / schemars **1** / toml **1** / secrecy **0.10** / thiserror **2** / tokio-tungstenite **0.30** / axum **0.8** / axum-server **0.8**; website TypeScript **7**; deploy images `ubuntu:26.04` and `alpine/openssl:3.5.8`. MSRV is now **Rust 1.89+**.
