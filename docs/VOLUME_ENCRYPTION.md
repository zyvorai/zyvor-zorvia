# Volume envelope encryption (Experimental)

Status: **Experimental**, behind `ZORVIA_EXPERIMENTAL=1`. Unit-tested only. Not run against a live cluster, a CSI driver, a real Vault or a cloud KMS.

## Model

1. Zorvia generates a random 256-bit data key (DEK) per volume.
2. The configured key provider ([KEY_PROVIDER.md](KEY_PROVIDER.md): local AES-256-GCM key, or Vault Transit) wraps it. The wrap authenticates `namespace/vm/volume` and the purpose `dek`, so a wrapped key copied to another volume's Secret, or a TOTP value used as a key, does not unwrap.
3. Only the wrapped form is stored: Secret `zorvia-dek-<vm>-<volume>`, key `wrapped-dek`, annotations for version and provider.
4. Before anything is stored the wrap is unwrapped again and compared; a wrap that does not round-trip is discarded.
5. **Rotation** unwraps with whichever key sealed the value (the local provider keeps old keys via `ZORVIA_KEY_PREVIOUS_FILES`) and re-wraps under the current one, then verifies and replaces the Secret using its `resourceVersion`. The DEK is unchanged, so no data is re-encrypted. If the value already uses the current key, nothing changes.

Plaintext handling: the DEK type has no `Debug`/`Clone`/`Serialize`, is zeroed on drop, and is never logged, returned by the API, put in audit details or written to disk or a Secret. Caveat: zeroing is best effort (`fill(0)`); copies made inside the AES/HTTP libraries and the Vault response buffer are not zeroed.

## API (cluster.admin only, except the guest redeem route)

| Route | Purpose |
|-------|---------|
| `GET /volume-encryption/status` | provider name, readiness |
| `POST /volume-encryption/plan` | preflight: key provider present, StorageClass encrypts, no existing key Secret |
| `POST /volume-encryption/apply` | wrap key, create the Secret, create the DataVolume; audited (`volume-encryption`, High) |
| `POST /volume-encryption/rotate` | re-wrap; audited |
| `POST /volume-encryption/guest-luks/token` | guest LUKS: mint a one-time unlock token and guest cloud-config; audited |
| `POST /volume-encryption/guest-luks/redeem/{ns}/{vm}/{volume}` | guest LUKS: **unauthenticated**, bearer one-time token; returns the passphrase to the guest; audited, rate limited |

Request: `{"namespace","vm","volume","size","storage_class","mode":"storage_class","acknowledge_unverified_encryption":false}`. Responses contain Secret and DataVolume names, never key material. Apply deletes the key Secret again if the DataVolume cannot be created.

## Encrypted volume modes

* `storage_class` (implemented): the DataVolume uses a StorageClass whose parameters encrypt. The only provisioner Zorvia recognises is Ceph-CSI RBD (`parameters.encrypted: "true"` and `encryptionKMSID`); for any other provisioner the plan is blocked unless `acknowledge_unverified_encryption` is set.
* `guest_luks` (**Experimental, partial**): LUKS inside the guest, unlocked with the Zorvia data key delivered by a one-time token. See [Guest LUKS](#guest-luks-experimental-partial) below.

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
| Guest LUKS (cryptsetup) | token delivery and cloud-init templates implemented | partial, unit-tested only; no guest was booted |

## Not verified

Live DataVolume/Secret creation, RBAC needed for Secret create/replace by the Zorvia service account (add `secrets` create/get/update in the target namespaces), a real Vault, restore/backup of encrypted volumes, and live migration of encrypted volumes.

## Web console

`/app/volume-encryption` (admin) shows provider status and offers plan, apply (behind a confirmation) and rotate. Apply is enabled only after a ready plan for the same inputs. A 403 `EXPERIMENTAL_DISABLED` shows an experimental, disabled notice.

## Guest LUKS (Experimental, partial)

### Delivery mechanism (decision)

A plaintext key must never rest in a Secret or on a disk, and the Zyvor guest agent channel is a host-to-guest virtio channel that Zorvia does not currently speak (it is KubeVirt's QGA socket, and `guest-exec` would put the key in a command line), so Zorvia uses a **Zorvia-served unwrap endpoint called by the guest with a one-time token**:

1. `POST /volume-encryption/guest-luks/token` (cluster.admin, audited) mints a token bound to `namespace/vm/volume`: random, single use, TTL 30 to 900 seconds (default 600). Only a SHA-256 of the token secret is kept (in memory), compared in constant time. A new token for the same volume revokes the previous one. The response carries the token and a ready cloud-config; it never carries a key.
2. The cloud-config (user data for the VM) writes the token to `/run/zorvia-luks/token` (tmpfs, 0600) and installs a script and systemd unit.
3. At boot the script calls `POST <ZORVIA_GUEST_LUKS_URL>/api/volume-encryption/guest-luks/redeem/<ns>/<vm>/<volume>` with `Authorization: Bearer <token>` (curl config on stdin, so neither URL nor token appears in a process list; https only, TLS 1.2+, optional pinned CA). This route is the only unauthenticated route; it denies generically (403), is rate limited (5 failures per minute per volume, 120 attempts per minute overall, 429), and audits every outcome. A valid token is consumed first; only then is the wrapped key read and unwrapped. A token used for another volume is burned.
4. The response body is the passphrase (base64 of the data key, `Cache-Control: no-store`). The script holds it in a shell variable and pipes it to `cryptsetup luksFormat`/`luksOpen --key-file=-`. It is never written to disk or argv. The token file is deleted after the call.

This endpoint is the **one place a key leaves Zorvia**, and it goes to the holder of a valid, unexpired, unused token for exactly that volume. Anyone who can read the user data before first boot (cloud-init Secret, the VMI spec) can use the token first, within its TTL; keep the TTL short and the user data Secret restricted.

### Flow

Configure `ZORVIA_GUEST_LUKS_URL` (https URL guests use to reach Zorvia; required) and optionally `ZORVIA_GUEST_LUKS_CA_FILE` (PEM of a public CA certificate to trust). Then:

1. `zorvia volume encryption plan <vm> <volume> --size 20Gi --storage-class <sc> --guest-luks`, then `apply ... --guest-luks --yes`: creates the wrapped-key Secret (annotated `zorvia.io/encryption-mode: guest-luks`) and a blank DataVolume `<vm>-<volume>`. The storage is **not** encrypted; the response gives the virtio `disk_serial` (`zv<volume>`, hyphens removed, 20 characters max).
2. Attach the DataVolume to the VM as a disk with that serial (Zorvia does not edit the VM for you).
3. `zorvia volume encryption guest-token <vm> <volume> --yes`: returns `cloud_config`. Put it in the VM's cloud-init user data (merge with your own) before start.
4. On first boot the script formats a blank disk (it refuses a disk holding a non-LUKS filesystem), opens it as `/dev/mapper/zv-<volume>`, makes ext4 and mounts `/mnt/<volume>`. `crypttab` and `fstab` lines are added with `noauto,nofail`, and **contain no key** (`none`), so a locked volume never blocks boot.

### Rotation

`POST /volume-encryption/rotate` re-wraps the Secret under the current provider key and is mode independent. The data key, and therefore the LUKS passphrase, does not change, so nothing in the guest needs to change.

Changing the LUKS passphrase itself (`cryptsetup luksChangeKey`) is **not automated and is dangerous**: after it, the Zorvia-wrapped key no longer unlocks the volume. Zorvia cannot yet wrap an operator-supplied or newly generated key into the existing Secret. Until it can, do not run `luksChangeKey` on a Zorvia-managed volume. The safe procedure today is: create a new volume with a new key (`apply`), copy the data across in the guest, then retire the old DataVolume and Secret. The planned procedure, once a "new key version" API exists, is `luksAddKey` with the new key (delivered by token), verify unlock, then `luksKillSlot` for the old slot, then replace the Secret.

### What is missing

* **Every boot needs a fresh token.** Tokens are single use and the delivery is first-boot cloud-init, so a rebooted guest stays locked (and `nofail`-booted) until a new token is placed at `/run/zorvia-luks/token` (tmpfs, lost on reboot) and `systemctl start zorvia-luks-unlock`. There is no automation that injects a token at VM start (no start hook, no config-drive refresh, no guest agent request path). This is the main gap; without it guest LUKS is not usable for unattended restarts.
* Token state is in memory: a Zorvia restart voids outstanding tokens (fail closed), and multiple replicas would not share them. Single replica only.
* The redeem route is reachable by anything that can reach Zorvia; the guest-side network path, TLS trust and hardening are the operator's responsibility. There is no per-source-IP limiting (limits are per volume and global), so a flood of bad attempts can lock out a volume's redemption for a minute.
* The Zyvor guest agent is not used. The generated script was reviewed and the cloud-config is YAML-parsed in tests, but it was **never run in a guest**, and nothing was exercised against a live cluster, KubeVirt, CDI or a real Vault.
* Live migration, snapshot and backup of the volume are unverified (backups contain ciphertext, which is the intent).
* Needs the Zorvia service account to read Secrets in the target namespace (already required above).
