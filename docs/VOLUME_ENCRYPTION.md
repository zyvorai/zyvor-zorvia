# Volume envelope encryption (Experimental)

Status: **Experimental**, behind `ZORVIA_EXPERIMENTAL=1`. Unit-tested only. Not run against a live cluster, a CSI driver, a real Vault or a cloud KMS.

## Model

1. Zorvia generates a random 256-bit data key (DEK) per volume.
2. The configured key provider ([KEY_PROVIDER.md](KEY_PROVIDER.md): local AES-256-GCM key, or Vault Transit) wraps it. The wrap authenticates `namespace/vm/volume` and the purpose `dek`, so a wrapped key copied to another volume's Secret, or a TOTP value used as a key, does not unwrap.
3. Only the wrapped form is stored: Secret `zorvia-dek-<vm>-<volume>`, key `wrapped-dek`, annotations for version and provider.
4. Before anything is stored the wrap is unwrapped again and compared; a wrap that does not round-trip is discarded.
5. **Rotation** unwraps with whichever key sealed the value (the local provider keeps old keys via `ZORVIA_KEY_PREVIOUS_FILES`) and re-wraps under the current one, then verifies and replaces the Secret using its `resourceVersion`. The DEK is unchanged, so no data is re-encrypted. If the value already uses the current key, nothing changes.

Plaintext handling: the DEK type has no `Debug`/`Clone`/`Serialize`, is zeroed on drop, and is never logged, returned by the API, put in audit details or written to disk or a Secret. Caveat: zeroing is best effort (`fill(0)`); copies made inside the AES/HTTP libraries and the Vault response buffer are not zeroed.

## API (cluster.admin only)

| Route | Purpose |
|-------|---------|
| `GET /volume-encryption/status` | provider name, readiness |
| `POST /volume-encryption/plan` | preflight: key provider present, StorageClass encrypts, no existing key Secret |
| `POST /volume-encryption/apply` | wrap key, create the Secret, create the DataVolume; audited (`volume-encryption`, High) |
| `POST /volume-encryption/rotate` | re-wrap; audited |

Request: `{"namespace","vm","volume","size","storage_class","mode":"storage_class","acknowledge_unverified_encryption":false}`. Responses contain Secret and DataVolume names, never key material. Apply deletes the key Secret again if the DataVolume cannot be created.

## Encrypted volume modes

* `storage_class` (implemented): the DataVolume uses a StorageClass whose parameters encrypt. The only provisioner Zorvia recognises is Ceph-CSI RBD (`parameters.encrypted: "true"` and `encryptionKMSID`); for any other provisioner the plan is blocked unless `acknowledge_unverified_encryption` is set.
* `guest_luks` (documented only, apply refuses it): LUKS inside the guest needs the key delivered to the guest at boot. That would need either a plaintext Secret (not allowed here) or a guest agent that asks Zorvia to unwrap, which is not built. Do not rely on it.

### Important limit

With `storage_class`, the CSI driver encrypts with **its own KMS configuration**. The Zorvia-wrapped DEK is recorded for key management, audit and rotation, but the CSI driver does not use it. Making the driver use a Zorvia-managed key (for example a Ceph-CSI `secrets-metadata` passphrase Secret) would require storing a usable key in a Secret, which this feature deliberately does not do. Treat the current state as key custody plus an encrypted-volume provisioning flow, not as end-to-end KMS-backed encryption.

## Providers

* Local and Vault Transit: implemented (shared with TOTP sealing). Vault is tested against a fake HTTP Vault only.
* AWS KMS and Google Cloud KMS: implemented (Experimental), see below. **Nothing has been run against a real AWS or Google KMS**; the clients are tested against local mock servers only.
* Azure Key Vault: not implemented. Its wrap/unwrap API has no authenticated-context parameter, so the binding to `namespace/vm/volume` would have to be faked inside the plaintext; it is left out rather than offered in a weaker form.

## Cloud KMS providers (Experimental)

Select one by environment (exactly one provider may be configured: local key, Vault, AWS or GCP). The provider then wraps data keys (`dek`) and, like the other providers, TOTP secrets, so every TOTP seal/open becomes a KMS call; use a cloud KMS for volume keys only if that latency and availability dependency is not acceptable for logins.

| Variable | Meaning |
|----------|---------|
| `ZORVIA_AWS_KMS_KEY_ID` | selects AWS: key id, key ARN or alias |
| `ZORVIA_AWS_REGION` (or `AWS_REGION`, `AWS_DEFAULT_REGION`) | region |
| `ZORVIA_AWS_KMS_ENDPOINT` | optional, e.g. a VPC endpoint; https only (http for loopback) |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | credentials from the environment (the Kubernetes Secret mounted as env) |
| `ZORVIA_AWS_CREDENTIALS_FILE`, `ZORVIA_AWS_PROFILE` | alternative: a mounted shared-credentials style file (`[default]` with `aws_access_key_id` etc.), re-read on every request so rotation works |
| `ZORVIA_GCP_KMS_KEY` | selects GCP: `projects/P/locations/L/keyRings/R/cryptoKeys/K` |
| `ZORVIA_GCP_ACCESS_TOKEN_FILE` | mounted file with an OAuth2 access token, re-read on every request (for example kept fresh by a sidecar) |
| `ZORVIA_GCP_USE_METADATA=1` | workload identity: token from the GCE/GKE metadata server, cached until near expiry (plain http to the link-local metadata server is the one non-https call) |
| `ZORVIA_GCP_ACCESS_TOKEN` | fixed token; short-lived, prefer the two above |
| `ZORVIA_GCP_KMS_ENDPOINT` | optional override; https only (http for loopback) |

Set exactly one GCP token source. Credentials are never read from API request bodies and never logged; error messages contain only the service, operation, HTTP status and the service's error code.

**Binding.** AWS: `EncryptionContext` = `{"zorvia:purpose":"dek","zorvia:owner":"<namespace>/<vm>/<volume>"}`. GCP: `additionalAuthenticatedData` = `zorvia:dek:<namespace>/<vm>/<volume>`. A wrapped key opened with another volume's context is refused by the KMS. Note that AWS records the encryption context in CloudTrail in clear, so namespace, VM and volume names are visible there. Sealed values are `enc:v1:aws:<CiphertextBlob>` or `enc:v1:gcp:<ciphertext>`. Key rotation happens inside the KMS (old ciphertexts keep decrypting), so `needs_rewrap` is true only for values from another provider.

**Transport.** https only (plain http for loopback), 10 s request and 5 s connect timeouts, no redirects, 64 KiB response cap, up to 4 attempts with exponential backoff and jitter on HTTP 429, 5xx, transport errors and AWS throttling error codes; other 4xx are not retried. AWS requests are signed with SigV4 implemented in-tree (HMAC-SHA256 over the `sha2` crate); it is checked against the published AWS SigV4 test-suite vectors (get-vanilla, post-vanilla), the AWS documentation signing-key example and RFC 4231 HMAC vectors.

**Why no SDK.** The official AWS SDK (`aws-sdk-kms` with `aws-config`) and Google's Cloud KMS client each pull in dozens of crates (smithy runtime, credential chains, hyper/tonic stacks, a second TLS stack in places) for two JSON POSTs. The REST route adds **no new crate**: `reqwest`, `sha2`, `base64` and `chrono` were already dependencies, so `Cargo.lock` is unchanged and `deny.toml` is unaffected.

**Not done.** AWS IRSA / web-identity, instance-profile and SSO credential chains (use env or a credentials file kept fresh by your own tooling); GCP service-account JSON key signing (needs RSA; use workload identity or a token file); Azure Key Vault; multi-region keys; KMS request-level grants; zeroing of the HTTP response buffers (the data key transits the process heap like any other string in the Vault path).

## Storage backends considered

| Backend | Considered | Status |
|---------|-----------|--------|
| Ceph-CSI RBD | `encrypted` / `encryptionKMSID` parameters recognised | parameters checked in unit tests only; no real cluster |
| Rook-Ceph (same driver) | as above | not verified |
| Other CSI drivers (hostpath, local-path, LVM, vendor arrays) | not recognised | blocked unless acknowledged; whether they encrypt is the operator's responsibility |
| Guest LUKS (cryptsetup) | design notes above | not implemented |

## Not verified

Live DataVolume/Secret creation, RBAC needed for Secret create/replace by the Zorvia service account (add `secrets` create/get/update in the target namespaces), a real Vault, restore/backup of encrypted volumes, and live migration of encrypted volumes.

## Web console

`/app/volume-encryption` (admin) shows provider status and offers plan, apply (behind a confirmation) and rotate. Apply is enabled only after a ready plan for the same inputs. A 403 `EXPERIMENTAL_DISABLED` shows an experimental, disabled notice.
