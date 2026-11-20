// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

use super::inventory::{
    ATTRIBUTE_PREFIX, CLUSTER_LABEL, DATACENTER_LABEL, FOLDER_LABEL, TAG_PREFIX,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataKind {
    Tag,
    Attribute,
    Datacenter,
    Cluster,
    Folder,
}

pub fn qualified_key(kind: MetadataKind, key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        return Err(anyhow!("metadata key cannot be empty"));
    }
    if key.len() > 63 {
        return Err(anyhow!(
            "metadata key '{}' exceeds Kubernetes 63-character name limit",
            key
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(anyhow!(
            "metadata key '{}' contains unsupported characters",
            key
        ));
    }
    Ok(match kind {
        MetadataKind::Tag => format!("{}{}", TAG_PREFIX, key),
        MetadataKind::Attribute => format!("{}{}", ATTRIBUTE_PREFIX, key),
        MetadataKind::Datacenter => DATACENTER_LABEL.into(),
        MetadataKind::Cluster => CLUSTER_LABEL.into(),
        MetadataKind::Folder => FOLDER_LABEL.into(),
    })
}

pub fn merge_patch(kind: MetadataKind, key: &str, value: Option<&str>) -> Result<Value> {
    let q = qualified_key(kind, key)?;
    let mut values = Map::new();
    values.insert(
        q,
        value.map_or(Value::Null, |v| Value::String(v.to_string())),
    );
    let mut metadata = Map::new();
    let field = if kind == MetadataKind::Attribute {
        "annotations"
    } else {
        "labels"
    };
    metadata.insert(field.into(), Value::Object(values));
    let mut root = Map::new();
    root.insert("metadata".into(), Value::Object(metadata));
    Ok(Value::Object(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_tag_patch() {
        let p = merge_patch(MetadataKind::Tag, "tier", Some("gold")).unwrap();
        assert_eq!(
            p.pointer("/metadata/labels/tag.zorvia.io~1tier")
                .and_then(Value::as_str),
            Some("gold")
        );
    }
    #[test]
    fn removal_uses_json_null() {
        let p = merge_patch(MetadataKind::Attribute, "owner", None).unwrap();
        assert!(p
            .pointer("/metadata/annotations/attr.zorvia.io~1owner")
            .unwrap()
            .is_null());
    }
    #[test]
    fn inventory_labels_use_stable_keys() {
        assert_eq!(
            qualified_key(MetadataKind::Datacenter, "ignored").unwrap(),
            DATACENTER_LABEL
        );
    }
    #[test]
    fn rejects_invalid_key() {
        assert!(qualified_key(MetadataKind::Tag, "bad/key").is_err());
    }
}
