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
* Cloud KMS (AWS/GCP/Azure): only the `DataKeyWrapper` trait and `CloudKmsStub`, which fails every call. No vetted client is a dependency.

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
