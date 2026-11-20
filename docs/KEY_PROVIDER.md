# Key provider: encrypting secrets Zorvia has to read back

TOTP secrets must be stored in a form the server can use to check a code, so they cannot be hashed.
Without a key provider they sit in `auth.db` as plain base32, readable by anyone who obtains a copy of the
database or a backup of the volume. With a provider they are stored encrypted, so a database copy alone is not
enough.

Nothing changes unless you configure one: with no provider, secrets stay as before.

## Local key

```bash
# 64 hex characters (32 bytes), e.g. openssl rand -hex 32; keep it in a Kubernetes Secret
ZORVIA_KEY_FILE=/keys/zorvia.key      # or ZORVIA_KEY_HEX=<hex>
ZORVIA_KEY_ID=k1                      # label stored with each value (default k1)
ZORVIA_KEY_PREVIOUS_FILES=k0=/keys/old.key   # older keys, decrypt-only (comma separated id=path)
```

Values are AES-256-GCM sealed as `enc:v1:<key id>:<payload>` with a random nonce, and **bound to the user
they belong to**: a ciphertext copied onto another user's row does not decrypt.

**Rotation.** Put the new key in `ZORVIA_KEY_FILE` with a new `ZORVIA_KEY_ID`, list the old one in
`ZORVIA_KEY_PREVIOUS_FILES`, restart: at startup every value sealed under another id is re-sealed with the
current key. Once nothing refers to the old id you may drop it.

## HashiCorp Vault (Transit)

```bash
ZORVIA_VAULT_ADDR=https://vault.example.com:8200   # https required (plain http only for localhost)
ZORVIA_VAULT_TRANSIT_KEY=zorvia
ZORVIA_VAULT_TOKEN_FILE=/vault/token               # or ZORVIA_VAULT_TOKEN
ZORVIA_VAULT_MOUNT=transit                         # default
```

The key never leaves Vault; Zorvia calls `transit/encrypt` and `transit/decrypt` and stores
`enc:v1:vault:<vault ciphertext>`. The token needs only those two operations on that key. Vault key rotation is
handled by Vault (older ciphertexts still decrypt). Configure either a local key or Vault, not both. Every
sealing or opening makes an HTTPS call (10 s timeout), so an unreachable Vault shows up as failed 2FA checks.

## Behaviour you should know

- **Existing plaintext is sealed at startup** (idempotent; logged as `sealed N TOTP secret(s)`).
- **No silent fallback.** With a provider configured, a failed seal is an error, never a plaintext write.
- **A secret that cannot be decrypted fails closed**: the 2FA check is refused (the log says
  `cannot decrypt the TOTP secret ... an admin can reset their 2FA`). It happens with a wrong, missing or rotated-out
  key, or with encrypted values and no provider configured (a startup error is logged with the count).
- **Losing the key loses the secrets**, not the accounts. Recovery is the admin reset:
  `DELETE /api/v1/users/{id}/totp` (users.admin) removes the user's second factor and revokes their sessions; they sign
  in with their password and enrol again. The same endpoint covers a lost authenticator.
- Back the key up separately from the database; a backup of both together defeats the purpose.

## Not covered

Other stored secrets (webhook signing secrets, the bootstrap file) are not sealed by this; API tokens and passwords are
already stored as hashes. The Vault client is tested against a mock server, not a real Vault; check it with yours.
