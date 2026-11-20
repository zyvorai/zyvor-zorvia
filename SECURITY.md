# Security Policy

## Supported Versions

We release patches for security vulnerabilities for the following versions:

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| 0.2.x   | :x:                |
| 0.1.x   | :x:                |

**0.3.3** hardens authentication and authorization (server-side RBAC, scoped API
tokens, JWT revocation, optional secure OIDC (`ZORVIA_OIDC_ENABLED=1`), lab
credential guards, webhook SSRF hardening). Upgrade from 0.3.2 as soon as
practical.

## Reporting a Vulnerability

We take the security of Zorvia seriously. If you have discovered a security vulnerability, please follow these steps:

### Private Disclosure

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them via one of the following methods:

1. **GitHub Security Advisories** (Preferred)
   - Go to the [Security tab](https://github.com/zyvorai/zorvia/security/advisories)
   - Click "Report a vulnerability"
   - Fill in the details

2. **Email**
   - Send an email to: info@zyvor.dev
   - Include as much information as possible (see below)

### What to Include

Please include the following information:

- **Type of vulnerability** (e.g. authorization bypass, SSRF, credential exposure)
- **Full paths of source file(s)** related to the vulnerability
- **Location of the affected source code** (tag/branch/commit or direct URL)
- **Step-by-step instructions to reproduce** the issue
- **Proof-of-concept or exploit code** (if possible)
- **Impact of the issue**, including how an attacker might exploit it

### Response Timeline

- We aim to acknowledge reports within **72 hours**
- We aim to provide an initial assessment within **7 days**
- Critical issues will be prioritized for immediate patching

### Disclosure Policy

- We request that you give us reasonable time to address the issue before public disclosure
- We will credit researchers who responsibly disclose vulnerabilities (unless anonymity is requested)
- Once a fix is available, we will publish a security advisory with details

## Security Best Practices

When deploying Zorvia:

1. **Do not use lab defaults in production** — `ZORVIA_LAB_MODE` is no longer set by the
   shipped manifests (leave it off), set unique
   `ZORVIA_JWT_SECRET` and `ZORVIA_ADMIN_PASSWORD`, and create auth Secrets via
   `./scripts/create-auth-secret.sh` (never commit Secret manifests).
2. Prefer **scoped API tokens** (`POST /api/v1/api-tokens`) over a shared
   `ZORVIA_API_KEY` (lab mode only).
3. Use TLS in production (Ingress/cert-manager preferred over NodePort self-signed).
4. Keep dependencies updated (`cargo update`, Dependabot/Renovate).
5. Run Zorvia with least-privilege Kubernetes RBAC — apply
   `deploy/rook-bootstrap-rbac.yaml` only when intentionally bootstrapping Rook.
6. Restrict webhook destinations with `ZORVIA_WEBHOOK_ALLOWLIST` when possible.
7. **OIDC is opt-in** — set `ZORVIA_OIDC_ENABLED=1` with issuer/client/secret/redirect
   for PKCE + token exchange + JWKS verification. Without the enable flag, OIDC
   env vars are ignored.

## Known Security Considerations

- **Roles and sockets.** `/ws/console` and `/ws/vnc` require `vm.power`, `/ws/ssh`
  and `/ws/pods/*` require `cluster.admin`. A Viewer cannot open any of them. VM
  names are validated, cross-origin WebSocket handshakes are refused, and every
  console/VNC/SSH session is written to the audit trail.
- **Sensitive reads are not for Viewers.** Audit logs and stats, webhooks, the Atlas
  audit export and VM/launcher logs need `cluster.admin` (VM logs: `vm.power`).
  Deleting a backup or snapshot needs `vm.delete`.
- **Namespace restriction.** An admin can confine a user to a namespace allow-list
  (`PUT /api/v1/users/{id}/namespaces`); see [docs/TENANCY.md](docs/TENANCY.md).
- **MFA.** While 2FA is on, re-enrolling (`POST /v1/auth/totp/setup`) needs the
  password and a current code, and revokes sessions.
- **Sessions.** `POST /v1/auth/logout` revokes every session of the caller (PAM
  sessions via a per-user not-before time); `POST /v1/auth/password` changes your own
  password and revokes sessions.
- **Brute force.** 8 failed sign-ins for a username lock it for 15 minutes (in-memory,
  per process, plus a shared counter in PostgreSQL when it is configured, so several replicas lock together; see [docs/POSTGRES.md](docs/POSTGRES.md)). A wrong TOTP code counts as a failure. This also means someone who
  knows a username can lock that account out for 15 minutes.
- **JWT secret.** `ZORVIA_JWT_SECRET` must be at least 32 bytes outside lab mode; a
  shorter value is ignored and an ephemeral secret is used (tokens reset on restart).
- **OIDC.** The flow is bound to the starting browser by an HttpOnly `SameSite=Lax`
  state cookie (login CSRF), the session token is returned in the URL fragment, and an
  ID token without a `kid` is only accepted when the IdP publishes a single RSA key.

- A TOTP code is accepted once (the accepted time step is stored per user); a disabled
  account is only revealed to someone who knows its password, and unknown users cost the
  same bcrypt time as real ones; an API token whose `expires_at` cannot be read is treated
  as expired, and creation rejects a malformed value.
- A generated bootstrap admin password (only when `ZORVIA_ADMIN_PASSWORD` is unset on a
  fresh database) is written to `bootstrap-admin-password` (mode 0600) next to the auth
  database instead of the log; read it once and delete the file.
- OIDC ID tokens: the JWKS is cached for 5 minutes (an unknown `kid` forces one refresh),
  `azp` is required when there are several audiences and must be this client, and `nbf`/`iat`
  are checked with 2 minutes of skew.
- OIDC group-to-role mapping (`ZORVIA_OIDC_GROUP_ROLES`): the IdP's groups set role and namespaces at every sign-in,
  unmatched users are denied by default; see [docs/OIDC.md](docs/OIDC.md).
- API tokens can be confined to namespaces at creation (`namespaces`); see
  [docs/TENANCY.md](docs/TENANCY.md).
- TOTP secrets can be encrypted at rest with a key provider (a local AES-256-GCM key or HashiCorp Vault Transit),
  bound to their owner, with an admin 2FA reset as the recovery path; see [docs/KEY_PROVIDER.md](docs/KEY_PROVIDER.md).
  Off unless configured: with no provider they are stored as before.
- PostgreSQL (`ZORVIA_DATABASE_URL`) holds password hashes, token hashes and sealed TOTP secrets: keep the URL in a Secret
  (`database.existingSecret`), use `sslmode=verify-full` with `ZORVIA_DATABASE_CA_FILE` outside a trusted network (`require`
  encrypts but does not verify the server), and give the database user rights on Zorvia's tables only; see [docs/POSTGRES.md](docs/POSTGRES.md).
- Guest agent views (`/api/vms/{name}/guest/...`) read accounts, certificates and installed software from inside a guest and
  need `vm.power`; only a fixed read-only method set of the agent is reachable (no guest exec or file access). The Zyvor
  agent is given the single ambient capability `CAP_SYS_ADMIN` so it can freeze filesystems for snapshots; see [docs/GUEST_AGENT.md](docs/GUEST_AGENT.md).
- Not yet addressed: PAM sessions are not namespace-restricted (PAM login is opt-in and a stub on current builds).
- JWT access tokens default to a **60-minute** TTL (`ZORVIA_JWT_TTL_MINUTES`);
  local-user disable/role/password/TOTP changes bump `token_version` and revoke
  outstanding sessions.
- TOTP disable requires password + current TOTP code and revokes sessions.
- The shared env API key is ignored unless `ZORVIA_LAB_MODE=1`.
- Webhook delivery resolves DNS, pins the address, and re-validates redirects;
  private/link-local destinations are rejected.

## Contact

For security-related inquiries: info@zyvor.dev
