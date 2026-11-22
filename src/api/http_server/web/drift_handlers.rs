// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Drift detection and change planning, exposed as REST for the first time —
//! `zorvia drift`/`zorvia plan` are real, well-tested logic (`DriftEngine`,
//! `ChangePlanner`) that DEVELOPMENT.md names as core day-2 surface, but were
//! previously CLI-only. The REST shape differs only in where "desired" comes
//! from: the CLI reads a file, this reads it from the request body — the
//! comparison/planning logic itself is untouched and fully reused.

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

use crate::change_plan::ChangePlanner;
use crate::config::VMConfig;
use crate::gitops::drift::{DriftEngine, DriftOptions};
use crate::kube::vm_config_to_kubevirt;

#[derive(Debug, Deserialize)]
pub struct DriftRequestBody {
    /// A KubeVirt `VirtualMachine` manifest (must have `kind: "VirtualMachine"`)
    /// or a Zorvia `VMConfig`, in either case as a JSON object.
    pub desired: serde_json::Value,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub include_status: bool,
}

/// Mirrors the CLI's `load_vm_manifest` (src/handlers/gitops.rs), but
/// resolves an already-parsed JSON body instead of reading+parsing a file.
fn resolve_desired_manifest(desired: serde_json::Value) -> Result<serde_json::Value, String> {
    if desired.get("kind").and_then(serde_json::Value::as_str) == Some("VirtualMachine") {
        return Ok(desired);
    }
    let config: VMConfig = serde_json::from_value(desired).map_err(|e| {
        format!("desired manifest is neither a KubeVirt VirtualMachine (missing kind: \"VirtualMachine\") nor a valid Zorvia VMConfig: {e}")
    })?;
    let vm = vm_config_to_kubevirt(&config)
        .map_err(|e| format!("failed to convert VMConfig to KubeVirt: {e}"))?;
    serde_json::to_value(vm)
        .map_err(|e| format!("failed to serialize converted VirtualMachine: {e}"))
}

/// (status, error code, message) — small and Copy-friendly, unlike returning
/// a full `axum::response::Response` as an `Err` variant (clippy::result_large_err).
type DriftError = (u16, &'static str, String);

async fn compare_against_live(
    state: &SharedState,
    name: &str,
    body: DriftRequestBody,
) -> Result<crate::gitops::drift::DriftReport, DriftError> {
    let desired_value =
        resolve_desired_manifest(body.desired).map_err(|e| (400, "INVALID_MANIFEST", e))?;

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let actual = client
        .get_vm(&namespace, name)
        .await
        .map_err(|e| (404, "NOT_FOUND", sanitize_error(&e)))?;
    let actual_value =
        serde_json::to_value(&actual).map_err(|e| (500, "SERIALIZE_FAILED", e.to_string()))?;

    let options = DriftOptions {
        ignore_paths: body.ignore,
        include_status: body.include_status,
        normalize_kubernetes_metadata: true,
    };
    Ok(DriftEngine::new(options).compare(&desired_value, &actual_value))
}

fn drift_error_response((code, kind, msg): DriftError) -> axum::response::Response {
    let (st, j) = err_json(code, kind, &msg);
    (st, j).into_response()
}

pub async fn fabric_vm_drift(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<DriftRequestBody>,
) -> impl IntoResponse {
    match compare_against_live(&state, &name, body).await {
        Ok(report) => Json(json!(report)).into_response(),
        Err(e) => drift_error_response(e),
    }
}

pub async fn fabric_vm_change_plan(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<DriftRequestBody>,
) -> impl IntoResponse {
    match compare_against_live(&state, &name, body).await {
        Ok(report) => {
            let plan = ChangePlanner::plan(&report);
            Json(json!(plan)).into_response()
        }
        Err(e) => drift_error_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_a_raw_kubevirt_manifest() {
        let manifest = json!({
            "kind": "VirtualMachine",
            "apiVersion": "kubevirt.io/v1",
            "metadata": { "name": "test" },
        });
        let resolved = resolve_desired_manifest(manifest.clone()).unwrap();
        assert_eq!(resolved, manifest);
    }

    #[test]
    fn converts_a_vmconfig_to_kubevirt() {
        let config = json!({
            "name": "test-vm",
            "namespace": "default",
            "cpu": { "cores": 2 },
            "memory": { "size": "4Gi" },
            "disks": [{
                "name": "rootdisk",
                "size": "20Gi",
                "boot_order": 1,
                "source": { "type": "blank" },
            }],
            "interfaces": [{
                "name": "default",
                "network": "default",
                "network_type": "pod",
            }],
        });
        let resolved = resolve_desired_manifest(config).unwrap();
        assert_eq!(resolved["kind"], "VirtualMachine");
        assert_eq!(resolved["metadata"]["name"], "test-vm");
    }

    #[test]
    fn rejects_neither_kubevirt_nor_vmconfig() {
        let junk = json!({ "totally": "unrelated" });
        let err = resolve_desired_manifest(junk).unwrap_err();
        assert!(err.contains("neither a KubeVirt VirtualMachine"));
    }
}
