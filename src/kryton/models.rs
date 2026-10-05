// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse<T> {
    pub items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComputeSpec {
    pub cpu: i32,
    pub memory_mi_b: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpec {
    pub size_gi_b: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DockurOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_ou: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autologin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_boot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oem_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_iso: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_disks_gi_b: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MachineSpec {
    pub name: String,
    pub image: String,
    pub compute: ComputeSpec,
    pub disk: DiskSpec,
    #[serde(default)]
    pub network: NetworkSpec,
    #[serde(default)]
    pub ttl_minutes: i32,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockur: Option<DockurOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRef {
    pub provider: String,
    #[serde(default)]
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub id: String,
    pub project: String,
    pub provider: String,
    pub state: String,
    pub spec: MachineSpec,
    pub provider_ref: ProviderRef,
    #[serde(default)]
    pub ip_addresses: Vec<String>,
    #[serde(default)]
    pub console_url: String,
    #[serde(default)]
    pub rdp_host: String,
    #[serde(default)]
    pub rdp_port: i32,
    #[serde(default)]
    pub rdp_username: String,
    #[serde(default)]
    pub progress_percent: Option<i32>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub project: String,
    pub machine_id: String,
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub id: String,
    pub name: String,
    pub version: String,
    pub family: String,
    pub description: String,
    pub min_cpu: i32,
    pub min_memory_mi_b: i32,
    pub default_disk_gi_b: i32,
    #[serde(default)]
    pub dockur_version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub availability: String,
    #[serde(default)]
    pub storage_source: String,
    #[serde(default)]
    pub storage_namespace: String,
    #[serde(default)]
    pub storage_path: String,
    pub ready: bool,
    #[serde(default)]
    pub certified: bool,
    #[serde(default)]
    pub validation_score: f64,
    #[serde(default)]
    pub passport_build_id: String,
    /// `linux` for cloud-image templates; empty for Windows images.
    #[serde(default)]
    pub os: String,
}

/// A Windows golden-image build on Kryton (dockur install, Sysprep, qcow2
/// capture, then an optional CDI bootstrap into a DataSource).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoldenBuild {
    pub id: String,
    #[serde(default)]
    pub version: String,
    pub image_id: String,
    pub state: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub progress_percent: i32,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub console_url: String,
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub bootstrap_state: String,
    #[serde(default)]
    pub bootstrap_message: String,
    #[serde(default)]
    pub data_source: String,
    #[serde(default)]
    pub certified: bool,
    #[serde(default)]
    pub validation_score: f64,
    #[serde(default)]
    pub passport_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoldenStartRequest {
    pub image_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub provider: String,
    pub snapshots: bool,
    pub networks: bool,
    pub ttl: bool,
    pub live_migration: bool,
    pub console: bool,
    pub golden_images: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DoctorFinding {
    pub check: String,
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub provider: String,
    pub healthy: bool,
    #[serde(default)]
    pub findings: Vec<DoctorFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub project: String,
    pub provider: String,
    pub machines: i32,
    pub running: i32,
    pub stopped: i32,
    pub attention: i32,
    pub cpu: i32,
    pub memory_mi_b: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMachineRequest {
    pub project: String,
    pub name: String,
    pub image: String,
    pub compute: ComputeSpec,
    pub disk: DiskSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_minutes: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockur: Option<DockurOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_contract_matches_kryton_json() {
        let raw = r#"{
          "id":"8e9377df-6fd7-4711-b2d7-43ccbd233c29",
          "project":"finance","provider":"kubevirt","state":"running",
          "spec":{"name":"win11-01","image":"windows-11-enterprise",
            "compute":{"cpu":4,"memoryMiB":8192},
            "disk":{"sizeGiB":80},"network":{},"ttlMinutes":0},
          "providerRef":{"provider":"kubevirt","namespace":"kryton-finance","name":"win11-01"},
          "ipAddresses":["10.42.0.10"],"rdpHost":"10.42.0.10","rdpPort":3389,
          "createdAt":"2026-09-09T10:00:00Z","updatedAt":"2026-09-09T10:01:00Z"
        }"#;
        let machine: Machine = serde_json::from_str(raw).unwrap();
        assert_eq!(machine.spec.compute.memory_mi_b, 8192);
        assert_eq!(machine.provider_ref.namespace, "kryton-finance");
        assert_eq!(machine.rdp_port, 3389);
    }

    #[test]
    fn create_request_uses_kryton_camel_case() {
        let request = CreateMachineRequest {
            project: "finance".into(),
            name: "win11-01".into(),
            image: "windows-11-enterprise".into(),
            compute: ComputeSpec {
                cpu: 4,
                memory_mi_b: 8192,
            },
            disk: DiskSpec {
                size_gi_b: 80,
                storage_class: None,
                volume_mode: None,
            },
            network: None,
            ttl_minutes: Some(60),
            dockur: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["compute"]["memoryMiB"], 8192);
        assert_eq!(value["disk"]["sizeGiB"], 80);
        assert_eq!(value["ttlMinutes"], 60);
    }

    #[test]
    fn golden_build_contract_matches_kryton_json() {
        let raw = r#"{
          "id":"gb-1","version":"2026.10","imageId":"windows-11-enterprise",
          "state":"ready","phase":"capture","progressPercent":100,"message":"captured",
          "outputPath":"/var/lib/kryton/golden/gb-1.qcow2","sha256":"abc",
          "startedAt":"2026-10-05T01:00:00Z","bootstrapState":"succeeded",
          "dataSource":"windows-11-enterprise","certified":true,"validationScore":92.5
        }"#;
        let build: GoldenBuild = serde_json::from_str(raw).unwrap();
        assert_eq!(build.image_id, "windows-11-enterprise");
        assert_eq!(build.progress_percent, 100);
        assert_eq!(build.data_source, "windows-11-enterprise");
        assert!(build.certified);
        assert!(build.console_url.is_empty());
    }

    #[test]
    fn golden_start_request_uses_kryton_camel_case() {
        let value = serde_json::to_value(GoldenStartRequest {
            image_id: "windows-11-pro".into(),
            version: None,
            auto: true,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"imageId":"windows-11-pro","auto":true})
        );
    }
}
