# Upgrade notes

## v0.7.4 -> v0.8.0

Everything new in 0.8.0 is opt-in and Experimental except scheduled snapshots. Nothing changes for an install that sets neither
`ZORVIA_EXPERIMENTAL=1` nor a feature switch. The Experimental features were tested with unit tests, mock servers and fake
stores only; none has been run against a live cluster, a real IdP or a real cloud KMS (see [FEATURE_MATURITY.md](FEATURE_MATURITY.md)).

### Required ops
1. **Minimum Kubernetes API server is now 1.33** (was 1.32). `k8s-openapi` moved from 0.28 to 0.29, which has no `v1_32` feature; `kube` 5.0 already required 0.29. The lab runs v1.36.
2. **Helm chart version 0.8.0** (`appVersion` 0.8.0; `values-production.yaml` pins image tag `0.8.0`). Run `helm upgrade` so the ServiceAccount picks up the new RBAC below. For `deploy/k8s.yaml` and `deploy/k3s-zorvia-web.yaml`, re-apply the manifests.
3. **New RBAC: `secrets` `create`, `delete`, `patch`** in the Zorvia namespace (Helm: `charts/zorvia/templates/rbac.yaml`; manifests as above). Used only by the Experimental rescue `reset-password` (a per-Job Secret) and by volume encryption, which stores wrapped data keys in Secrets (`zorvia-dek-<vm>-<volume>`). See [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md).
4. **Certificate lifecycle** (optional) needs `get`/`list` on `secrets` (cluster-wide or per namespace), `list` on `namespaces`, and for cert-manager `get`/`list` on `certificates` plus `update` on `certificates/status`. The chart does not grant these; see [CERTIFICATE_LIFECYCLE.md](CERTIFICATE_LIFECYCLE.md).
5. **Autoscaler metrics** (optional): to read load from the Kubernetes metrics API the ServiceAccount needs `list` on `pods.metrics.k8s.io` and a metrics-server; the chart does not grant this. See [AUTOSCALER_LOOP.md](AUTOSCALER_LOOP.md).
6. **Resource pools** need `list` and `patch` on `virtualmachines` in all namespaces (the same as `zorvia inventory show -A` and the tag/folder commands); see [RESOURCE_POOLS.md](RESOURCE_POOLS.md).

### New opt-in features and switches
All need `ZORVIA_EXPERIMENTAL=1` on the API server, plus:

| Feature | Extra switch / config | Docs |
|---------|-----------------------|------|
| Disk image upload | `ZORVIA_CDI_UPLOAD_PROXY_URL` (optional `_CA_FILE`, `ZORVIA_IMAGE_UPLOAD_MAX_BYTES`) | [IMAGE_UPLOAD.md](IMAGE_UPLOAD.md) |
| Autoscaler loop and decisions | `ZORVIA_AUTOSCALER_LOOP=1`; `ZORVIA_AUTOSCALER_APPLY=1` to act (default recommends only); optional `ZORVIA_AUTOSCALER_PROMETHEUS_URL`, `_PROMETHEUS_TOKEN_FILE` | [AUTOSCALER_LOOP.md](AUTOSCALER_LOOP.md) |
| SAML SSO | `ZORVIA_SAML_ENABLED=1` plus SP/IdP settings; login only in a build with the `saml` cargo feature | [SAML.md](SAML.md) |
| Rescue reset-password / install-packages / inspect | none beyond the gate; `cluster.admin` | [RESCUE.md](RESCUE.md) |
| Golden-image scan/sign gates and gates v2 | `ZORVIA_FEATURE_GOLDEN_PIPELINE=1` and the `ZORVIA_GOLDEN_*` settings; unconfigured steps stay deferred | [GOLDEN_IMAGES.md](GOLDEN_IMAGES.md) |
| Volume encryption (local, Vault, AWS KMS, GCP KMS, guest LUKS) | `ZORVIA_AWS_KMS_KEY_ID` or `ZORVIA_GCP_KMS_KEY` to pick a cloud KMS; `ZORVIA_GUEST_LUKS_URL` for guest LUKS | [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md) |
| DR replication pairs (failover, failback) | `ZORVIA_FEATURE_DR_REPLICATION=1` and an off-cluster backup target | [DR_REPLICATION.md](DR_REPLICATION.md) |
| Resource pools | none beyond the gate; `cluster.admin` | [RESOURCE_POOLS.md](RESOURCE_POOLS.md) |
| Certificate lifecycle | optional `ZORVIA_CERT_ALERT_DAYS`, `ZORVIA_CERT_CHECK_INTERVAL_SECS` | [CERTIFICATE_LIFECYCLE.md](CERTIFICATE_LIFECYCLE.md) |

Scheduled per-VM snapshots (`/snapshot-schedules`, see [SNAPSHOTS.md](SNAPSHOTS.md)) are not behind the experimental gate.

### New CLI commands
`zorvia dr pair list|create|status|failover|failback-plan|resume|delete`, `zorvia volume encryption status|plan|apply|guest-token|rotate`,
`zorvia rescue reset-password|install-packages|inspect|status` (with `--wait`/`--timeout`), `zorvia certs list|status|alerts|cert-manager list|renew`,
`zorvia autoscaler status|decisions`, `zorvia inventory pool list|create|show|report|hierarchy|assign|unassign|delete`, and `zorvia guest image-upload`.
All but `image-upload` take `--url`/`--token` (`ZORVIA_URL`/`ZORVIA_TOKEN`). See [CLI_REMOTE.md](CLI_REMOTE.md). Shell completions pick them up automatically.

### New routes
All under `/api`; each answers 403 `EXPERIMENTAL_DISABLED` when the gate is off (the redeem route and guest-LUKS token are documented in [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md)).

- `/certificates/status|inventory|alerts|cert-manager` and `POST /certificates/cert-manager/renew` (`cluster.admin`).
- `/volume-encryption/status|plan|apply|rotate`, `POST /volume-encryption/guest-luks/token`, and `POST /volume-encryption/guest-luks/redeem/{namespace}/{vm}/{volume}` (unauthenticated, bearer one-time token; needs a security review before it can leave Experimental).
- `/v1/autoscaler/status` and `/v1/autoscaler/decisions` (`cluster.admin`).
- `/v1/enterprise/dr/pairs[/{name}[/status|failover|failover/plan|failback|failback/plan|failback/cancel|resume]]` and `/v1/enterprise/dr/alerts`.
- `/v1/inventory/pools[/{name}[/report|assign]]` and `/v1/inventory/pools/hierarchy` (`cluster.admin`).
- `POST /images/uploads/{name}`, `GET /images/uploads/{name}`, `GET /images/upload-config`, `POST /images/validate`.
- `/snapshot-schedules[/{name}[/enable|disable]]`.
- `GET /v1/auth/saml/metadata`, `/v1/auth/saml/login`, `POST /v1/auth/saml/acs` (404 unless `ZORVIA_SAML_ENABLED=1`).

New console pages: `/app/dr-pairs`, `/app/volume-encryption`, `/app/certificates`, `/app/autoscaler` (all admin-only), and an upload panel on `/app/disk-images`. Resource pools have no web page.

### Behaviour to know
- **SAML:** the published image is built without the `saml` feature, so the ACS endpoint answers 501 and creates no session. Keep using OIDC unless you build with `--features saml` (needs libxmlsec1) and accept that no real IdP has been tested.
- **Autoscaler loop:** it only recommends unless `ZORVIA_AUTOSCALER_APPLY=1`. Load now comes from the Kubernetes metrics API (optionally Prometheus); missing or stale data produces no decision. The executor has not been verified on a live cluster. Decisions are kept in memory only (100 per target), per process, and lost on restart.
- **Volume encryption:** the CSI driver does not consume the Zorvia key. The cloud KMS wrappers (AWS, GCP) replace the old stub and were tested against mock servers only; there is no Azure Key Vault. Guest LUKS tokens are in memory, single replica, and are not delivered automatically at each VM start.
- **Resource pool limits are advisory.** VM create returns `pool_warnings` and never blocks.
- **Cert alerts** go to the notification list and the log only; there is no email or webhook delivery.
- **Failback** never overwrites: a stopped original VM must be removed by the operator before it restores.
- `deploy/rescue-agent.Dockerfile` builds with `--no-default-features --features rescue-agent`, which now builds; the Dockerfile itself has not been built (no Docker on the dev host).

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
