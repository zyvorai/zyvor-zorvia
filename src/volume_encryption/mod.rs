// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Envelope encryption for per-VM volumes (Experimental, `volume-encryption`).
//!
//! A random 256-bit data key (DEK) is generated per volume and immediately wrapped
//! by the configured key provider (`crate::keys`: a local key or Vault Transit).
//! Only the wrapped form is ever stored (a Kubernetes Secret). Rotation unwraps with
//! whatever key sealed it and re-wraps under the current one; the DEK itself does
//! not change, so no data is re-encrypted.
//!
//! Hard rules, enforced by types and tests:
//! * a [`DataKey`] has no `Debug` output, no `Clone`, no `Serialize`, and is zeroed on drop;
//! * nothing in this module returns, logs or persists an unwrapped key;
//! * the wrap is bound to `namespace/vm/volume` (authenticated data), so a wrapped
//!   key copied to another volume's Secret does not unwrap.
//!
//! See docs/VOLUME_ENCRYPTION.md for what is and is not verified.

pub mod cloud_kms;
pub mod guest_luks;

use crate::keys::KeyProvider;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const FEATURE_ID: &str = "volume-encryption";
/// Purpose label authenticated into every wrap.
const PURPOSE: &str = "dek";
pub const SECRET_PREFIX: &str = "zorvia-dek-";
pub const ANN_VERSION: &str = "zorvia.io/dek-version";
pub const ANN_PROVIDER: &str = "zorvia.io/dek-provider";
pub const ANN_DEK_SECRET: &str = "zorvia.io/dek-secret";
pub const ANN_MODE: &str = "zorvia.io/encryption-mode";
pub const LABEL_MANAGED: &str = "zorvia.io/volume-encryption";

fn zero(buf: &mut [u8]) {
    buf.fill(0);
    std::hint::black_box(&*buf);
}

/// A plaintext data key. Deliberately minimal: see the module docs.
pub struct DataKey([u8; 32]);

impl DataKey {
    pub fn generate() -> Self {
        let mut k = [0u8; 32];
        rand::rng().fill_bytes(&mut k);
        Self(k)
    }
    #[cfg(test)]
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }
    /// Constant-time equality, for the post-wrap verification.
    pub fn same_as(&self, other: &DataKey) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            == 0
    }
}

impl Drop for DataKey {
    fn drop(&mut self) {
        zero(&mut self.0);
    }
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(<redacted>)")
    }
}

/// Something that can wrap and unwrap data keys. `KeyProvider` (local, Vault) is the
/// real implementation, including the Experimental AWS / GCP KMS wrappers in [`cloud_kms`].
pub trait DataKeyWrapper {
    fn provider_name(&self) -> String;
    /// Wrap `key`, binding it to `context`.
    fn wrap(&self, context: &str, key: &DataKey) -> Result<String>;
    fn unwrap(&self, context: &str, wrapped: &str) -> Result<DataKey>;
    /// True when `wrapped` was sealed under something other than the current key.
    fn needs_rewrap(&self, wrapped: &str) -> bool;
}

impl DataKeyWrapper for KeyProvider {
    fn provider_name(&self) -> String {
        match self {
            KeyProvider::Local { current, .. } => format!("local:{current}"),
            KeyProvider::Vault(_) => "vault-transit".into(),
            KeyProvider::Cloud(c) => format!("{}-kms", c.id()),
        }
    }

    fn wrap(&self, context: &str, key: &DataKey) -> Result<String> {
        let mut text = B64.encode(key.0).into_bytes();
        let r = self.seal_for(PURPOSE, context, std::str::from_utf8(&text)?);
        zero(&mut text);
        r
    }

    fn unwrap(&self, context: &str, wrapped: &str) -> Result<DataKey> {
        let text = self.open_for(PURPOSE, context, wrapped)?;
        let mut bytes = text.into_bytes();
        let decoded = B64.decode(&bytes);
        zero(&mut bytes);
        let mut raw = decoded.map_err(|_| anyhow!("unwrapped value is not a data key"))?;
        let r = <[u8; 32]>::try_from(raw.as_slice())
            .map(DataKey)
            .map_err(|_| anyhow!("unwrapped value is not a 256-bit data key"));
        zero(&mut raw);
        r
    }

    fn needs_rewrap(&self, wrapped: &str) -> bool {
        self.needs_seal(wrapped)
    }
}

/// `namespace/vm/volume`, the authenticated context of a wrap.
pub fn context(namespace: &str, vm: &str, volume: &str) -> String {
    format!("{namespace}/{vm}/{volume}")
}

pub fn dek_secret_name(vm: &str, volume: &str) -> String {
    format!("{SECRET_PREFIX}{vm}-{volume}")
}

/// Generate a data key, wrap it, and prove the wrap opens before returning it.
pub fn new_wrapped(w: &dyn DataKeyWrapper, ctx: &str) -> Result<String> {
    let key = DataKey::generate();
    let wrapped = w.wrap(ctx, &key)?;
    let back = w.unwrap(ctx, &wrapped)?;
    if !key.same_as(&back) {
        bail!("wrapped data key did not round-trip; refusing to store it");
    }
    Ok(wrapped)
}

/// Re-wrap `old` under the provider's current key. The new value is verified against
/// the old one before it is returned, so a bad re-wrap can never replace a good one.
pub fn rewrap(w: &dyn DataKeyWrapper, ctx: &str, old: &str) -> Result<String> {
    let key = w.unwrap(ctx, old)?;
    let fresh = w.wrap(ctx, &key)?;
    let back = w.unwrap(ctx, &fresh)?;
    if !key.same_as(&back) {
        bail!("re-wrapped data key did not round-trip; keeping the old one");
    }
    Ok(fresh)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The StorageClass encrypts (e.g. Ceph-CSI RBD with a KMS).
    #[default]
    StorageClass,
    /// LUKS inside the guest; the key is delivered by one-time token
    /// ([`guest_luks`]). Experimental and partial.
    GuestLuks,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptRequest {
    pub namespace: String,
    pub vm: String,
    /// Disk name; the DataVolume is `<vm>-<volume>`.
    pub volume: String,
    pub size: String,
    pub storage_class: String,
    #[serde(default)]
    pub mode: Mode,
    /// Accept a StorageClass whose provisioner Zorvia cannot recognise as encrypting.
    #[serde(default)]
    pub acknowledge_unverified_encryption: bool,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::StorageClass => "storage-class",
            Mode::GuestLuks => "guest-luks",
        }
    }
}

impl EncryptRequest {
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        for (k, v) in [
            ("namespace", &self.namespace),
            ("vm", &self.vm),
            ("volume", &self.volume),
            ("storage_class", &self.storage_class),
        ] {
            if let Err(e) = crate::kube::lifecycle::validate_k8s_name(k, v) {
                p.push(e.to_string());
            }
        }
        // The DataVolume and Secret names are built from vm + volume.
        if self.vm.len() + self.volume.len() + 1 > 63 {
            p.push("vm and volume together must be at most 62 characters".into());
        }
        if !matches!(crate::storage::parse_size_to_bytes(&self.size), Some(b) if b > 0) {
            p.push(format!("size '{}' is not a valid positive size", self.size));
        }
        p
    }
    pub fn dv_name(&self) -> String {
        format!("{}-{}", self.vm, self.volume)
    }
    pub fn ctx(&self) -> String {
        context(&self.namespace, &self.vm, &self.volume)
    }
}

/// Judge a StorageClass for encryption. Returns `(verified, warnings)`; the only
/// provisioner Zorvia recognises is Ceph-CSI RBD (`encrypted: "true"` plus an
/// `encryptionKMSID`). Anything else is unverified.
pub fn check_storage_class(
    provisioner: &str,
    params: &std::collections::BTreeMap<String, String>,
) -> (bool, Vec<String>) {
    let mut warn = Vec::new();
    if provisioner.ends_with("rbd.csi.ceph.com") {
        let on = params.get("encrypted").map(String::as_str) == Some("true");
        if !on {
            warn.push("StorageClass does not set parameters.encrypted=\"true\"".into());
        }
        if !params.contains_key("encryptionKMSID") {
            warn.push(
                "StorageClass has no parameters.encryptionKMSID (Ceph-CSI needs a KMS entry)"
                    .into(),
            );
        }
        return (on && warn.is_empty(), warn);
    }
    warn.push(format!(
        "provisioner '{provisioner}' is not one Zorvia can verify encrypts volumes"
    ));
    (false, warn)
}

/// The Secret holding ONLY the wrapped key and its bookkeeping.
pub fn secret_manifest(req: &EncryptRequest, wrapped: &str, version: u32, provider: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": dek_secret_name(&req.vm, &req.volume),
            "namespace": req.namespace,
            "labels": { LABEL_MANAGED: "true", "zorvia.io/vm": req.vm },
            "annotations": {
                ANN_VERSION: version.to_string(),
                ANN_PROVIDER: provider,
                ANN_MODE: req.mode.as_str(),
            },
        },
        "type": "Opaque",
        "stringData": { "wrapped-dek": wrapped },
    })
}

pub fn data_volume_manifest(req: &EncryptRequest) -> Value {
    json!({
        "apiVersion": "cdi.kubevirt.io/v1beta1",
        "kind": "DataVolume",
        "metadata": {
            "name": req.dv_name(),
            "namespace": req.namespace,
            "labels": { LABEL_MANAGED: "true", "zorvia.io/vm": req.vm },
            "annotations": {
                ANN_DEK_SECRET: dek_secret_name(&req.vm, &req.volume),
                "zorvia.io/encryption": req.mode.as_str(),
            },
        },
        "spec": {
            "source": { "blank": {} },
            "storage": {
                "accessModes": ["ReadWriteOnce"],
                "storageClassName": req.storage_class,
                "resources": { "requests": { "storage": req.size } },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    fn local(id: &str, byte: u8) -> KeyProvider {
        let mut keys = HashMap::new();
        keys.insert(id.to_string(), [byte; 32]);
        KeyProvider::local(id, keys).unwrap()
    }

    fn req() -> EncryptRequest {
        EncryptRequest {
            namespace: "prod".into(),
            vm: "db1".into(),
            volume: "data".into(),
            size: "20Gi".into(),
            storage_class: "rbd-enc".into(),
            mode: Mode::StorageClass,
            acknowledge_unverified_encryption: false,
        }
    }

    #[test]
    fn wrap_unwrap_round_trip_and_context_binding() {
        let p = local("k1", 7);
        let key = DataKey::from_bytes([9u8; 32]);
        let w = p.wrap("prod/db1/data", &key).unwrap();
        assert!(p.unwrap("prod/db1/data", &w).unwrap().same_as(&key));
        // randomised
        assert_ne!(w, p.wrap("prod/db1/data", &key).unwrap());
        // copied to another volume's Secret
        assert!(p.unwrap("prod/db1/other", &w).is_err());
        // wrong key
        assert!(local("k1", 8).unwrap("prod/db1/data", &w).is_err());
        // a TOTP-purpose seal is not a data key
        let totp = p.seal("prod/db1/data", "x").unwrap();
        assert!(p.unwrap("prod/db1/data", &totp).is_err());
    }

    #[test]
    fn tampering_is_detected() {
        let p = local("k1", 7);
        let w = new_wrapped(&p, "a/b/c").unwrap();
        let mut t = w.clone().into_bytes();
        let n = t.len() - 3;
        t[n] = if t[n] == b'A' { b'B' } else { b'A' };
        assert!(p.unwrap("a/b/c", &String::from_utf8(t).unwrap()).is_err());
        assert!(p.unwrap("a/b/c", "not-sealed").is_err());
        assert!(p.unwrap("a/b/c", &w[..w.len() - 8]).is_err());
    }

    #[test]
    fn rotation_rewraps_without_changing_the_key() {
        let old = local("k1", 1);
        let key = DataKey::generate();
        let wrapped_old = old.wrap("n/v/d", &key).unwrap();
        let mut keys = HashMap::new();
        keys.insert("k1".to_string(), [1u8; 32]);
        keys.insert("k2".to_string(), [2u8; 32]);
        let new = KeyProvider::local("k2", keys).unwrap();
        assert!(new.needs_rewrap(&wrapped_old));
        let fresh = rewrap(&new, "n/v/d", &wrapped_old).unwrap();
        assert!(!new.needs_rewrap(&fresh));
        assert!(fresh.contains(":k2:"));
        assert!(new.unwrap("n/v/d", &fresh).unwrap().same_as(&key));
        // k2-only provider can open the new value, not the old one.
        assert!(local("k2", 2).unwrap("n/v/d", &fresh).is_ok());
        assert!(local("k2", 2).unwrap("n/v/d", &wrapped_old).is_err());
        // Re-wrapping a corrupted value fails and returns nothing.
        assert!(rewrap(&new, "n/v/d", "enc:v1:k1:AAAA").is_err());
    }

    #[test]
    fn no_plaintext_key_in_manifests_debug_or_errors() {
        let p = local("k1", 7);
        let key = DataKey::from_bytes([0x5Au8; 32]);
        let b64 = B64.encode([0x5Au8; 32]);
        let hex = "5a".repeat(32);
        let wrapped = p.wrap("prod/db1/data", &key).unwrap();
        let r = req();
        let all = format!(
            "{}{}{:?}{}",
            secret_manifest(&r, &wrapped, 1, &p.provider_name()),
            data_volume_manifest(&r),
            key,
            p.unwrap("prod/db1/other", &wrapped).unwrap_err()
        );
        assert!(!all.contains(&b64) && !all.contains(&hex));
        assert!(all.contains("<redacted>") && all.contains(&wrapped));
        // The wrapped string itself does not contain the key.
        assert!(!wrapped.contains(&b64));
        // The raw key bytes are not what the Secret stores.
        let s = secret_manifest(&r, &wrapped, 1, "local:k1");
        assert_eq!(s["stringData"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn key_is_zeroed_on_drop_helper() {
        let mut b = [7u8; 16];
        zero(&mut b);
        assert_eq!(b, [0u8; 16]);
    }

    #[test]
    fn request_validation_and_storage_class_checks() {
        assert!(req().problems().is_empty());
        let mut bad = req();
        bad.vm = "Bad_Name".into();
        bad.size = "lots".into();
        bad.mode = Mode::GuestLuks;
        // guest_luks is a valid mode now; only the name and size are wrong.
        assert_eq!(bad.problems().len(), 2);
        assert!(serde_json::from_str::<EncryptRequest>(
            r#"{"namespace":"a","vm":"b","volume":"c","size":"1Gi","storage_class":"s","x":1}"#
        )
        .is_err());

        let mut p = BTreeMap::new();
        p.insert("encrypted".to_string(), "true".to_string());
        p.insert("encryptionKMSID".to_string(), "vault".to_string());
        assert!(check_storage_class("rbd.csi.ceph.com", &p).0);
        p.remove("encryptionKMSID");
        let (ok, w) = check_storage_class("rook-ceph.rbd.csi.ceph.com", &p);
        assert!(!ok && w.len() == 1);
        assert!(!check_storage_class("hostpath.csi.k8s.io", &p).0);
    }

    #[test]
    fn manifests_are_shaped_for_the_storage_class_flow() {
        let r = req();
        let dv = data_volume_manifest(&r);
        assert_eq!(dv["metadata"]["name"], "db1-data");
        assert_eq!(dv["spec"]["storage"]["storageClassName"], "rbd-enc");
        assert_eq!(
            dv["metadata"]["annotations"][ANN_DEK_SECRET],
            "zorvia-dek-db1-data"
        );
        let s = secret_manifest(&r, "enc:v1:k1:x", 2, "local:k1");
        assert_eq!(s["metadata"]["annotations"][ANN_VERSION], "2");
    }
}
