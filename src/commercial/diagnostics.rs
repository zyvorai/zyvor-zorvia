// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Diagnostic bundle: versions, sanitized VM specs, recent events, storage
//! and scheduling conditions, operation failures, component health and a
//! configuration summary.
//!
//! Safety model: the bundle is built only from an allow-list of fields, and
//! then every value in it passes through [`redact_json`] as a second line of
//! defence. Secrets, kubeconfigs, tokens, cloud-init user data, guest disks
//! and memory dumps are never read in the first place.

use super::redact::{redact_json, redact_text};
use chrono::{DateTime, Utc};
use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Categories that are never collected. Shown in every preview.
pub const EXCLUDED: &[&str] = &[
    "credentials",
    "Secret contents",
    "kubeconfigs",
    "tokens",
    "cloud-init user data",
    "guest disks",
    "memory dumps",
];

const MAX_VMS: u32 = 500;
const MAX_EVENTS: usize = 200;
const MAX_ITEMS: u32 = 500;

#[derive(Debug, Clone, Serialize)]
pub struct Bundle {
    pub schema: u32,
    pub generated_at: DateTime<Utc>,
    pub generated_by: String,
    pub excluded: Vec<&'static str>,
    pub redaction: &'static str,
    pub sections: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectionInfo {
    pub name: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub size_bytes: usize,
    pub sha256: String,
    pub sections: Vec<SectionInfo>,
    pub excluded: Vec<&'static str>,
}

impl Bundle {
    /// Serialized form used for both download and upload.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    pub fn manifest(&self) -> Manifest {
        let bytes = self.to_bytes();
        let mut h = Sha256::new();
        h.update(&bytes);
        Manifest {
            size_bytes: bytes.len(),
            sha256: h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            sections: self
                .sections
                .iter()
                .map(|(k, v)| SectionInfo {
                    name: k.clone(),
                    bytes: v.to_string().len(),
                })
                .collect(),
            excluded: self.excluded.clone(),
        }
    }
}

fn resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: group.into(),
        version: version.into(),
        api_version: if group.is_empty() {
            version.into()
        } else {
            format!("{group}/{version}")
        },
        kind: kind.into(),
        plural: plural.into(),
    }
}

async fn list_all(
    client: &kube::Client,
    ar: &ApiResource,
    lp: ListParams,
) -> Result<Vec<DynamicObject>, String> {
    let api: Api<DynamicObject> = Api::all_with(client.clone(), ar);
    api.list(&lp)
        .await
        .map(|l| l.items)
        .map_err(|e| redact_text(&e.to_string()))
}

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

fn ident(o: &DynamicObject) -> Value {
    json!({ "namespace": o.metadata.namespace, "name": o.metadata.name })
}

fn error_section(e: String) -> Value {
    json!({ "error": e })
}

/// Build a bundle. Every section degrades independently: a failure becomes
/// `{ "error": ... }` for that section, and the rest is still produced.
pub async fn collect(
    client: &kube::Client,
    generated_by: &str,
    operation_failures: Vec<Value>,
    config_summary: Value,
) -> Bundle {
    let mut sections = Map::new();

    // Custom resources reused for versions and component health.
    let kubevirt = list_all(
        client,
        &resource("kubevirt.io", "v1", "KubeVirt", "kubevirts"),
        ListParams::default(),
    )
    .await;
    let cdi = list_all(
        client,
        &resource("cdi.kubevirt.io", "v1beta1", "CDI", "cdis"),
        ListParams::default(),
    )
    .await;

    let k8s_version = match client.apiserver_version().await {
        Ok(info) => serde_json::to_value(&info)
            .ok()
            .and_then(|v| v.get("gitVersion").cloned())
            .unwrap_or(Value::Null),
        Err(e) => error_section(redact_text(&e.to_string())),
    };
    let cr_version = |list: &Result<Vec<DynamicObject>, String>, field: &str| match list {
        Ok(items) => items
            .first()
            .and_then(|o| s(&o.data, &["status", field]))
            .map_or(Value::Null, |v| json!(v)),
        Err(e) => error_section(e.clone()),
    };
    sections.insert(
        "versions".into(),
        json!({
            "zorvia": env!("CARGO_PKG_VERSION"),
            "kubernetes": k8s_version,
            "kubevirt": cr_version(&kubevirt, "observedKubeVirtVersion"),
            "cdi": cr_version(&cdi, "observedVersion"),
        }),
    );

    let conditions = |list: &Result<Vec<DynamicObject>, String>| match list {
        Ok(items) => items
            .first()
            .and_then(|o| o.data.pointer("/status/conditions").cloned())
            .unwrap_or(Value::Null),
        Err(e) => error_section(e.clone()),
    };
    let nodes = match list_all(
        client,
        &resource("", "v1", "Node", "nodes"),
        ListParams::default().limit(200),
    )
    .await
    {
        Ok(items) => Value::Array(
            items
                .iter()
                .map(|n| {
                    let ready = n
                        .data
                        .pointer("/status/conditions")
                        .and_then(Value::as_array)
                        .and_then(|c| c.iter().find(|c| s(c, &["type"]) == Some("Ready")))
                        .and_then(|c| s(c, &["status"]).map(str::to_string));
                    json!({
                        "name": n.metadata.name,
                        "ready": ready,
                        "kubelet": s(&n.data, &["status", "nodeInfo", "kubeletVersion"]),
                    })
                })
                .collect(),
        ),
        Err(e) => error_section(e),
    };
    sections.insert(
        "component_health".into(),
        json!({
            "kubevirt_conditions": conditions(&kubevirt),
            "cdi_conditions": conditions(&cdi),
            "nodes": nodes,
        }),
    );

    // Sanitized VM specifications. Cloud-init and secret-bearing keys are
    // dropped by redaction; the allow-list keeps everything else out.
    let vms = match list_all(
        client,
        &resource("kubevirt.io", "v1", "VirtualMachine", "virtualmachines"),
        ListParams::default().limit(MAX_VMS),
    )
    .await
    {
        Ok(items) => Value::Array(
            items
                .iter()
                .map(|o| {
                    json!({
                        "vm": ident(o),
                        "printable_status": s(&o.data, &["status", "printableStatus"]),
                        "ready": o.data.pointer("/status/ready"),
                        "spec": o.data.get("spec"),
                        "conditions": o.data.pointer("/status/conditions"),
                    })
                })
                .collect(),
        ),
        Err(e) => error_section(e),
    };
    sections.insert("vms".into(), vms);

    let events = match list_all(
        client,
        &resource("", "v1", "Event", "events"),
        ListParams::default().limit(MAX_ITEMS),
    )
    .await
    {
        Ok(mut items) => {
            let stamp = |o: &DynamicObject| {
                s(&o.data, &["lastTimestamp"])
                    .or_else(|| s(&o.data, &["eventTime"]))
                    .unwrap_or("")
                    .to_string()
            };
            items.sort_by_key(|o| std::cmp::Reverse(stamp(o)));
            Value::Array(
                items
                    .iter()
                    .take(MAX_EVENTS)
                    .map(|o| {
                        json!({
                            "namespace": o.metadata.namespace,
                            "type": s(&o.data, &["type"]),
                            "reason": s(&o.data, &["reason"]),
                            "message": s(&o.data, &["message"]),
                            "object": {
                                "kind": s(&o.data, &["involvedObject", "kind"]),
                                "name": s(&o.data, &["involvedObject", "name"]),
                            },
                            "count": o.data.get("count"),
                            "last_seen": stamp(o),
                        })
                    })
                    .collect(),
            )
        }
        Err(e) => error_section(e),
    };
    sections.insert("events".into(), events);

    // Storage: claims that are not Bound and data volumes that are not done.
    let pvcs = match list_all(
        client,
        &resource("", "v1", "PersistentVolumeClaim", "persistentvolumeclaims"),
        ListParams::default().limit(MAX_ITEMS),
    )
    .await
    {
        Ok(items) => Value::Array(
            items
                .iter()
                .filter(|o| s(&o.data, &["status", "phase"]) != Some("Bound"))
                .map(|o| {
                    json!({
                        "pvc": ident(o),
                        "phase": s(&o.data, &["status", "phase"]),
                        "storage_class": s(&o.data, &["spec", "storageClassName"]),
                    })
                })
                .collect(),
        ),
        Err(e) => error_section(e),
    };
    let dvs = match list_all(
        client,
        &resource("cdi.kubevirt.io", "v1beta1", "DataVolume", "datavolumes"),
        ListParams::default().limit(MAX_ITEMS),
    )
    .await
    {
        Ok(items) => Value::Array(
            items
                .iter()
                .filter(|o| s(&o.data, &["status", "phase"]) != Some("Succeeded"))
                .map(|o| {
                    json!({
                        "data_volume": ident(o),
                        "phase": s(&o.data, &["status", "phase"]),
                        "progress": s(&o.data, &["status", "progress"]),
                        "conditions": o.data.pointer("/status/conditions"),
                    })
                })
                .collect(),
        ),
        Err(e) => error_section(e),
    };
    sections.insert(
        "storage_conditions".into(),
        json!({ "unbound_claims": pvcs, "incomplete_data_volumes": dvs }),
    );

    // Scheduling: pods stuck Pending and why.
    let pending = match list_all(
        client,
        &resource("", "v1", "Pod", "pods"),
        ListParams::default().fields("status.phase=Pending").limit(MAX_ITEMS),
    )
    .await
    {
        Ok(items) => Value::Array(
            items
                .iter()
                .map(|o| {
                    let sched = o
                        .data
                        .pointer("/status/conditions")
                        .and_then(Value::as_array)
                        .and_then(|c| c.iter().find(|c| s(c, &["type"]) == Some("PodScheduled")))
                        .map(|c| json!({ "status": s(c, &["status"]), "reason": s(c, &["reason"]), "message": s(c, &["message"]) }));
                    json!({ "pod": ident(o), "scheduled": sched })
                })
                .collect(),
        ),
        Err(e) => error_section(e),
    };
    sections.insert("scheduling".into(), json!({ "pending_pods": pending }));

    sections.insert(
        "operation_failures".into(),
        Value::Array(operation_failures),
    );
    sections.insert("configuration".into(), config_summary);
    sections.insert(
        "logs".into(),
        json!({ "included": false, "reason": "Zorvia does not retain application logs, so none are collected. Any log text added in future passes through redaction first." }),
    );

    // Second line of defence over everything above.
    let mut root = Value::Object(sections);
    redact_json(&mut root);
    let Value::Object(sections) = root else {
        unreachable!("root is an object")
    };

    Bundle {
        schema: 1,
        generated_at: Utc::now(),
        generated_by: generated_by.to_string(),
        excluded: EXCLUDED.to_vec(),
        redaction: "Sensitive keys and secret-shaped values are replaced with [REDACTED].",
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_sections_and_exclusions() {
        let mut sections = Map::new();
        sections.insert("versions".into(), json!({"zorvia": "x"}));
        let b = Bundle {
            schema: 1,
            generated_at: Utc::now(),
            generated_by: "t".into(),
            excluded: EXCLUDED.to_vec(),
            redaction: "r",
            sections,
        };
        let m = b.manifest();
        assert_eq!(m.sha256.len(), 64);
        assert_eq!(m.sections[0].name, "versions");
        assert!(m.excluded.contains(&"Secret contents"));
        assert_eq!(m.size_bytes, b.to_bytes().len());
    }

    #[test]
    fn section_error_shape() {
        assert_eq!(error_section("boom".into())["error"], "boom");
    }
}
