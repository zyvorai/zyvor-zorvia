# OIDC / enterprise SSO (Beta)

Zorvia supports OpenID Connect as an **opt-in** login path. It is off unless
you set `ZORVIA_OIDC_ENABLED=1`. Without that flag, `ZORVIA_OIDC_*` variables
are ignored and `GET /api/v1/auth/providers` returns `[]`.

## Security model

The callback **never** mints a session from an authorization `code` alone.
The flow is:

1. `GET /api/v1/auth/oidc/:id` — OpenID discovery, create PKCE verifier + nonce + state
2. Browser redirects to the IdP authorize URL (`code_challenge` S256)
3. `GET /api/v1/auth/oidc/callback` — one-time state lookup, **token exchange**
   with `code_verifier`, fetch JWKS, verify `id_token` (`iss`, `aud`, `nonce`, `exp`)
4. JIT provision a local SQLite user `oidc:<sub>` (unusable password; IdP-only login)
   The callback also requires the HttpOnly `zorvia_oidc_state` cookie set in step 1
   to equal the `state` (login CSRF protection), so sign-in must finish in the browser
   that started it. An ID token with no `kid` is accepted only when the IdP's JWKS has
   a single RSA key.
5. Mint a normal Zorvia JWT via `JwtConfig::generate` (honors `token_version` / disable)

Admin RBAC and session revocation apply to OIDC users the same as password users.

## Group-to-role mapping

By default every OIDC user is created as a plain `User` and an admin changes roles by hand. Set
`ZORVIA_OIDC_GROUP_ROLES` and the identity provider decides instead, **at every sign-in**:

```bash
ZORVIA_OIDC_GROUP_ROLES='[
  {"group":"zorvia-admins","role":"admin"},
  {"group":"team-a-devs","role":"user","namespaces":["team-a"]},
  {"group":"auditors","role":"viewer"}
]'
ZORVIA_OIDC_GROUPS_CLAIM=groups      # name of the ID-token claim (array of strings, or one string)
ZORVIA_OIDC_DEFAULT_ROLE=deny        # viewer | user | admin | deny (default deny) when no group matches
```

- The **highest** matching role wins (`admin` > `user` > `viewer`).
- `namespaces` confines a non-admin user to those namespaces (see [TENANCY.md](TENANCY.md)). The allow-list is the
  union over the matching mappings **at the winning role**; if one of those has no list the user is unrestricted. A
  lower-role group (say a global read-only `auditors`) never lifts the restriction of a restricted `user` group.
  Admins are never restricted.
- Group matching is exact and case-sensitive. The groups claim must be in the ID token (configure the IdP to emit it
  and request a scope that includes it, e.g. `ZORVIA_OIDC_SCOPES="openid profile email groups"`).
- A change of role or namespaces at sign-in bumps the user's token version, so older sessions stop working: a demotion
  or a narrowed list cannot be outlived by an old token.
- A user who matches no group and has no default role is refused (403) and their existing sessions are revoked.
  Removing someone from the IdP group therefore removes their access at the latest at their next sign-in or when
  their token expires (`ZORVIA_JWT_TTL_MINUTES`); there is no back-channel logout.
- A malformed mapping fails startup rather than guessing.

Not exercised against a real IdP yet: the decision logic, the claim parsing and the role/namespace/revocation sync are
unit-tested; the end-to-end sign-in with a group-bearing token still needs a trial with your IdP.

## Configuration

| Variable | Required | Default |
|----------|----------|---------|
| `ZORVIA_OIDC_ENABLED` | yes (`1` / `true`) | off |
| `ZORVIA_OIDC_ISSUER` | yes | — |
| `ZORVIA_OIDC_CLIENT_ID` | yes | — |
| `ZORVIA_OIDC_CLIENT_SECRET` | yes | — |
| `ZORVIA_OIDC_REDIRECT_URI` | recommended | `https://127.0.0.1:30152/api/v1/auth/oidc/callback` |
| `ZORVIA_OIDC_NAME` | no | `OIDC` |
| `ZORVIA_OIDC_SCOPES` | no | `openid profile email` |

Redirect URI must match the IdP app registration exactly (including HTTPS and path).

### Helm / Kubernetes

Set the vars on the API Deployment (or Secret → env). Example:

```yaml
env:
  - name: ZORVIA_OIDC_ENABLED
    value: "1"
  - name: ZORVIA_OIDC_ISSUER
    value: "https://idp.example.com"
  - name: ZORVIA_OIDC_CLIENT_ID
    valueFrom: { secretKeyRef: { name: zorvia-oidc, key: client-id } }
  - name: ZORVIA_OIDC_CLIENT_SECRET
    valueFrom: { secretKeyRef: { name: zorvia-oidc, key: client-secret } }
  - name: ZORVIA_OIDC_REDIRECT_URI
    value: "https://zorvia.example.com/api/v1/auth/oidc/callback"
```

## Operator checks

```bash
# Providers empty when disabled
curl -sk https://HOST:30152/api/v1/auth/providers
# []

# Live maturity registry
curl -sk https://HOST:30152/api/v1/features | jq '.features[] | select(.id=="oidc")'
```

After enable, the sign-in page shows an OIDC button from `/api/v1/auth/providers`.
Successful callback redirects to `/sign-in#oidc_token=…` (SPA stores the JWT).

## Limitations

- Single provider (`id=default`) per process today
- No SAML
- JWT still appears in the redirect query string (prefer short-lived exchange in a later release)
- Role is `user` for JIT accounts (promote via Access Control)

See also [FEATURE_MATURITY.md](FEATURE_MATURITY.md), [OIDC_LAB.md](OIDC_LAB.md), [SECURITY.md](../SECURITY.md), [UPGRADE.md](UPGRADE.md).
