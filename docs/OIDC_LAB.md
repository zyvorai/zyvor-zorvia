# OIDC lab bake-off

Short checklist to validate [OIDC.md](OIDC.md) against a real IdP in lab
(`ZORVIA_LAB_MODE=1` allowed). Production must use unique secrets and omit lab mode.

## Prerequisites

- Zorvia API reachable (lab NodePort `https://HOST:30152` or Ingress)
- IdP with a confidential client (Authorization Code + PKCE)
- Redirect URI registered exactly:
  `https://HOST:30152/api/v1/auth/oidc/callback`

## Dex (minimal)

```yaml
# excerpt — staticClients
staticClients:
  - id: zorvia
    secret: lab-oidc-secret
    name: Zorvia
    redirectURIs:
      - https://175.110.122.71:30152/api/v1/auth/oidc/callback
```

API env:

```bash
ZORVIA_OIDC_ENABLED=1
ZORVIA_OIDC_ISSUER=https://dex.lab.example/dex
ZORVIA_OIDC_CLIENT_ID=zorvia
ZORVIA_OIDC_CLIENT_SECRET=lab-oidc-secret
ZORVIA_OIDC_REDIRECT_URI=https://175.110.122.71:30152/api/v1/auth/oidc/callback
```

## Keycloak

1. Create realm client `zorvia`, Standard flow ON, Direct access OFF
2. Valid redirect URIs: the callback URL above
3. Client authentication ON → copy client secret
4. Issuer = `https://keycloak…/realms/<realm>` (no trailing slash mismatch)

## Bake-off steps

```bash
# 1. Disabled → empty providers
curl -sk https://HOST:30152/api/v1/auth/providers
# []

# 2. Enable OIDC env, restart API, providers non-empty
curl -sk https://HOST:30152/api/v1/auth/providers | jq .
# [{"id":"default","name":"OIDC",...}]

# 3. Browser: open /sign-in → OIDC button → IdP login
# 4. Land on /sign-in#oidc_token=… → SPA stores JWT
# 5. curl -sk -H "Authorization: Bearer $TOKEN" https://HOST:30152/api/v1/auth/me

# 6. Disable user / bump role via admin → old JWT fails (token_version)
```

Unit coverage (no IdP): `cargo test --features web --lib oidc` and
`tests/security_p0.rs` (`oidc_from_env_off_without_enabled_flag`).

## Pass criteria

| Check | Expected |
|-------|----------|
| Flag off | providers `[]`, OIDC routes unused |
| Flag on | discovery + PKCE authorize URL |
| Bad state/nonce | callback 4xx, no JWT |
| Happy path | local `oidc:<sub>` user + Zorvia JWT |
| Revocation | disable user → 401 on next API call |
