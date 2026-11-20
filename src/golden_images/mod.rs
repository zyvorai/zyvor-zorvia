// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! CDI golden-image bundle generation.
//!
//! A bundle contains a versioned DataVolume plus a stable CDI DataSource alias.
//! VMs can reference the stable DataSource while image promotion moves the alias
//! to a new immutable PVC/DataVolume revision.

#[cfg(feature = "web")]
pub mod convert;
pub mod jobs;

use crate::storage::parse_size_to_bytes;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageSourceType {
    Http,
    Registry,
}

impl ImageSourceType {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "http" | "https" => Some(Self::Http),
            "registry" | "container" | "oci" => Some(Self::Registry),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenImageSpec {
    pub name: String,
    pub version: String,
    pub namespace: String,
    pub source_type: ImageSourceType,
    pub source: String,
    pub size: String,
    pub storage_class: Option<String>,
    pub checksum: Option<String>,
}

impl GoldenImageSpec {
    pub fn validate(&self) -> Result<()> {
        validate_dns_name(&self.name)?;
        if self.version.trim().is_empty() {
            return Err(anyhow!("image version must not be empty"));
        }
        if self.source.trim().is_empty() {
            return Err(anyhow!("image source must not be empty"));
        }
        if !matches!(parse_size_to_bytes(&self.size), Some(bytes) if bytes > 0) {
            return Err(anyhow!("invalid Kubernetes storage size '{}'", self.size));
        }
        match self.source_type {
            ImageSourceType::Http => {
                if !(self.source.starts_with("http://") || self.source.starts_with("https://")) {
                    return Err(anyhow!(
                        "HTTP image source must begin with http:// or https://"
                    ));
                }
            }
            ImageSourceType::Registry => {
                if !self.source.starts_with("docker://") {
                    return Err(anyhow!(
                        "registry image source must use CDI docker:// URL syntax"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn versioned_name(&self) -> String {
        let suffix = sanitize_version(&self.version)
            .chars()
            .take(32)
            .collect::<String>();
        let max_base = 63usize.saturating_sub(suffix.len() + 1);
        let mut base = self.name.chars().take(max_base).collect::<String>();
        while base.ends_with('-') {
            base.pop();
        }
        format!("{}-{}", base, suffix)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenImageBundle {
    pub image: GoldenImageSpec,
    pub versioned_name: String,
    pub data_volume: Value,
    pub data_source: Value,
    pub promotion_notes: Vec<String>,
}

impl GoldenImageBundle {
    pub fn build(spec: GoldenImageSpec) -> Result<Self> {
        spec.validate()?;
        let versioned_name = spec.versioned_name();
        let data_volume = data_volume_manifest(&spec, &versioned_name);
        let data_source = data_source_manifest(&spec, &versioned_name);

        Ok(Self {
            image: spec,
            versioned_name,
            data_volume,
            data_source,
            promotion_notes: vec![
                "Apply the DataVolume and wait until CDI import reaches Succeeded".to_string(),
                "Validate the imported PVC with a disposable VM before promotion".to_string(),
                "Apply the stable DataSource alias only after validation succeeds".to_string(),
                "Keep the previous versioned PVC until rollback retention expires".to_string(),
            ],
        })
    }

    pub fn to_multi_document_yaml(&self) -> Result<String> {
        let data_volume = serde_yaml::to_string(&self.data_volume)?;
        let data_source = serde_yaml::to_string(&self.data_source)?;
        Ok(format!("---\n{}---\n{}", data_volume, data_source))
    }
}

/// CDI annotation asking for immediate PVC binding on WaitForFirstConsumer
/// storage classes.
pub const BIND_IMMEDIATE_ANNOTATION: &str = "cdi.kubevirt.io/storage.bind.immediate.requested";

fn data_volume_manifest(spec: &GoldenImageSpec, versioned_name: &str) -> Value {
    let source = match spec.source_type {
        ImageSourceType::Http => json!({"http": {"url": spec.source.clone()}}),
        ImageSourceType::Registry => json!({"registry": {"url": spec.source.clone()}}),
    };

    let mut storage = Map::new();
    storage.insert(
        "resources".to_string(),
        json!({"requests": {"storage": spec.size.clone()}}),
    );
    // ReadWriteOnce is the standard default for a single-VM boot disk. CDI
    // can normally infer this from the target StorageClass's StorageProfile,
    // but not every cluster has one configured (e.g. k3s's built-in
    // "local-path" ships without a StorageProfile access mode) — CDI then
    // rejects the DataVolume with ErrClaimNotValid instead of importing,
    // so set it explicitly rather than depending on cluster-specific setup.
    storage.insert("accessModes".to_string(), json!(["ReadWriteOnce"]));
    if let Some(storage_class) = &spec.storage_class {
        storage.insert(
            "storageClassName".to_string(),
            Value::String(storage_class.clone()),
        );
    }

    let mut metadata = image_metadata(spec, versioned_name, false);
    // Import now, don't wait for a consumer: on a WaitForFirstConsumer
    // StorageClass (k3s local-path, most CSI defaults) CDI otherwise leaves
    // the DataVolume pending until a VM uses it, so an image download or
    // capture job could never complete on its own. Must be set at creation;
    // CDI decides binding when it creates the PVC.
    metadata["annotations"][BIND_IMMEDIATE_ANNOTATION] = Value::String("true".into());
    json!({
        "apiVersion": "cdi.kubevirt.io/v1beta1",
        "kind": "DataVolume",
        "metadata": metadata,
        "spec": {
            "source": source,
            "storage": Value::Object(storage)
        }
    })
}

fn data_source_manifest(spec: &GoldenImageSpec, versioned_name: &str) -> Value {
    let metadata = image_metadata(spec, &spec.name, true);
    json!({
        "apiVersion": "cdi.kubevirt.io/v1beta1",
        "kind": "DataSource",
        "metadata": metadata,
        "spec": {
            "source": {
                "pvc": {
                    "name": versioned_name,
                    "namespace": spec.namespace.clone()
                }
            }
        }
    })
}

fn image_metadata(spec: &GoldenImageSpec, name: &str, stable_alias: bool) -> Value {
    let mut annotations = Map::new();
    annotations.insert(
        "zorvia.io/image-version".to_string(),
        Value::String(spec.version.clone()),
    );
    annotations.insert(
        "zorvia.io/image-source".to_string(),
        Value::String(spec.source.clone()),
    );
    if let Some(checksum) = &spec.checksum {
        annotations.insert(
            "zorvia.io/image-checksum".to_string(),
            Value::String(checksum.clone()),
        );
    }

    json!({
        "name": name,
        "namespace": spec.namespace.clone(),
        "labels": {
            "app.kubernetes.io/managed-by": "zorvia",
            "zorvia.io/golden-image": "true",
            "zorvia.io/stable-alias": if stable_alias { "true" } else { "false" }
        },
        "annotations": Value::Object(annotations)
    })
}

fn validate_dns_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        return Err(anyhow!("image name must contain 1-63 characters"));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(anyhow!("image name must not start or end with '-'"));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(anyhow!(
            "image name must use lowercase DNS-label characters: a-z, 0-9 and '-'"
        ));
    }
    Ok(())
}

fn sanitize_version(version: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    for ch in version.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            previous_dash = false;
        } else if !previous_dash && !out.is_empty() {
            out.push('-');
            previous_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "v1".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GoldenImageSpec {
        GoldenImageSpec {
            name: "ubuntu-golden".to_string(),
            version: "24.04-r2".to_string(),
            namespace: "images".to_string(),
            source_type: ImageSourceType::Http,
            source: "https://example.invalid/ubuntu.qcow2".to_string(),
            size: "40Gi".to_string(),
            storage_class: Some("fast".to_string()),
            checksum: Some("sha256:abc123".to_string()),
        }
    }

    #[test]
    fn source_type_parser_accepts_aliases() {
        assert_eq!(ImageSourceType::parse("https"), Some(ImageSourceType::Http));
        assert_eq!(
            ImageSourceType::parse("oci"),
            Some(ImageSourceType::Registry)
        );
        assert_eq!(ImageSourceType::parse("pvc"), None);
    }

    #[test]
    fn version_name_is_dns_safe() {
        let mut value = spec();
        value.version = "2026.09 RC-1".to_string();
        assert_eq!(value.versioned_name(), "ubuntu-golden-2026-09-rc-1");
    }

    #[test]
    fn http_bundle_contains_datavolume_source() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(bundle.data_volume["kind"], "DataVolume");
        assert_eq!(
            bundle.data_volume["spec"]["source"]["http"]["url"],
            "https://example.invalid/ubuntu.qcow2"
        );
    }

    #[test]
    fn data_volume_sets_an_explicit_access_mode() {
        // Regression: verified against a real cluster — CDI rejects a
        // DataVolume with ErrClaimNotValid on any StorageClass whose
        // StorageProfile doesn't declare an access mode (e.g. k3s's
        // built-in "local-path"), unless one is set explicitly here.
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(
            bundle.data_volume["spec"]["storage"]["accessModes"],
            serde_json::json!(["ReadWriteOnce"])
        );
    }

    #[test]
    fn datasource_points_to_versioned_pvc() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(bundle.data_source["metadata"]["name"], "ubuntu-golden");
        assert_eq!(
            bundle.data_source["spec"]["source"]["pvc"]["name"],
            bundle.versioned_name
        );
    }

    #[test]
    fn data_volume_requests_immediate_binding_but_data_source_does_not() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(
            bundle.data_volume["metadata"]["annotations"][BIND_IMMEDIATE_ANNOTATION],
            "true"
        );
        // Existing annotations are kept alongside it.
        assert!(bundle.data_volume["metadata"]["annotations"]
            .get("zorvia.io/image-checksum")
            .is_some());
        assert!(bundle.data_source["metadata"]["annotations"]
            .get(BIND_IMMEDIATE_ANNOTATION)
            .is_none());
    }

    #[test]
    fn storage_class_is_preserved() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(
            bundle.data_volume["spec"]["storage"]["storageClassName"],
            "fast"
        );
    }

    #[test]
    fn checksum_becomes_annotation() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        assert_eq!(
            bundle.data_volume["metadata"]["annotations"]["zorvia.io/image-checksum"],
            "sha256:abc123"
        );
    }

    #[test]
    fn invalid_size_is_rejected() {
        let mut value = spec();
        value.size = "forty".to_string();
        assert!(GoldenImageBundle::build(value).is_err());
    }

    #[test]
    fn invalid_http_source_is_rejected() {
        let mut value = spec();
        value.source = "file:///tmp/image.qcow2".to_string();
        assert!(GoldenImageBundle::build(value).is_err());
    }

    #[test]
    fn registry_source_requires_docker_scheme() {
        let mut value = spec();
        value.source_type = ImageSourceType::Registry;
        value.source = "quay.io/example/disk:latest".to_string();
        assert!(GoldenImageBundle::build(value.clone()).is_err());
        value.source = "docker://quay.io/example/disk:latest".to_string();
        assert!(GoldenImageBundle::build(value).is_ok());
    }

    #[test]
    fn bundle_renders_two_yaml_documents() {
        let bundle = GoldenImageBundle::build(spec()).unwrap();
        let yaml = bundle.to_multi_document_yaml().unwrap();
        assert!(yaml.contains("kind: DataVolume"));
        assert!(yaml.contains("kind: DataSource"));
        assert_eq!(yaml.matches("---\n").count(), 2);
    }
}
