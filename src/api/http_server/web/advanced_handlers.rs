// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Real backend for the VM detail page's "Advanced" tab: boot/firmware
//! config, CPU model, watchdog, serial console status, and a couple of
//! cluster-wide capability endpoints.
//!
//! Every route here matches the *actual* frontend contract in
//! `web/src/api/devices.ts` / `firmware.ts` / `system.ts` -- an earlier
//! pass at this feature built four GET routes against a guessed contract
//! (`/firmware`, `/cpu-config`, `/numa`, `/serial`) that the frontend never
//! called. These replace that dead code.
//!
//! Two frontend-assumed concepts (`DisplayConfig`'s port/password/
//! listen_address, `SerialConfig`'s unix/tcp socket types) don't correspond
//! to anything KubeVirt actually exposes per-VM -- access is always via the
//! VNC/console subresource, and there is always exactly one pty-type serial
//! console. Rather than fabricate values for those, the GET handlers report
//! the real fixed behavior and the mutating handlers reject with a clear
//! message instead of a false success.

use super::*;
use axum::extract::Json as AxumJson;
use serde_json::json;

type VmApi = kube::Api<crate::kube::types::VirtualMachine>;

fn vm_api(client: &crate::kube::KubeClient, namespace: &str) -> VmApi {
    kube::Api::namespaced(client.client(), namespace)
}

async fn patch_vm(
    api: &VmApi,
    name: &str,
    patch: &serde_json::Value,
) -> Result<crate::kube::types::VirtualMachine, kube::Error> {
    api.patch(
        name,
        &kube::api::PatchParams::default(),
        &kube::api::Patch::Merge(patch),
    )
    .await
}

// ---- Boot config -----------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SetBootBody {
    #[serde(default)]
    pub firmware: Option<String>,
    #[serde(default)]
    pub secure_boot: Option<bool>,
    #[serde(default)]
    pub boot_order: Option<Vec<String>>,
}

fn boot_config_json(vm: &crate::kube::types::VirtualMachine) -> serde_json::Value {
    let domain = &vm.spec.template.spec.domain;
    let firmware = domain.firmware.as_ref();
    let bootloader = firmware.and_then(|f| f.bootloader.as_ref());
    let is_uefi = bootloader.map(|b| b.efi.is_some()).unwrap_or(false);
    let secure_boot = bootloader
        .and_then(|b| b.efi.as_ref())
        .and_then(|efi| efi.secure_boot)
        .unwrap_or(false);
    let mut disks: Vec<(u32, String)> = domain
        .devices
        .as_ref()
        .and_then(|d| d.disks.as_ref())
        .map(|disks| {
            disks
                .iter()
                .filter_map(|d| d.boot_order.map(|order| (order, d.name.clone())))
                .collect()
        })
        .unwrap_or_default();
    disks.sort_by_key(|(order, _)| *order);
    let boot_order: Vec<String> = disks.into_iter().map(|(_, name)| name).collect();

    json!({
        "boot_order": boot_order,
        "firmware": if is_uefi { "uefi" } else { "bios" },
        "secure_boot": secure_boot,
        "kernel": null,
        "initrd": null,
        "kernel_args": null,
    })
}

pub async fn fabric_get_boot(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_vm(&namespace, &name).await {
        Ok(vm) => Json(boot_config_json(&vm)).into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_set_boot(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<SetBootBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    if let Some(fw) = &body.firmware {
        if fw != "bios" && fw != "uefi" {
            let (st, j) = err_json(
                400,
                "INVALID_FIRMWARE",
                "firmware must be \"bios\" or \"uefi\"",
            );
            return (st, j).into_response();
        }
    }

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let api = vm_api(&client, &namespace);
    let mut patch = json!({});

    if body.firmware.is_some() || body.secure_boot.is_some() {
        let currently_uefi = vm
            .spec
            .template
            .spec
            .domain
            .firmware
            .as_ref()
            .and_then(|f| f.bootloader.as_ref())
            .map(|b| b.efi.is_some())
            .unwrap_or(false);
        let want_uefi = body
            .firmware
            .as_deref()
            .map(|f| f == "uefi")
            .unwrap_or(currently_uefi);
        if !want_uefi && body.secure_boot == Some(true) {
            let (st, j) = err_json(
                400,
                "VALIDATION_FAILED",
                "secure_boot requires UEFI firmware",
            );
            return (st, j).into_response();
        }
        patch = bootloader_patch(want_uefi, body.secure_boot.unwrap_or(false));
    }

    if let Some(order) = &body.boot_order {
        let mut disks = vm
            .spec
            .template
            .spec
            .domain
            .devices
            .as_ref()
            .and_then(|d| d.disks.clone())
            .unwrap_or_default();
        for disk in disks.iter_mut() {
            disk.boot_order = None;
        }
        for (idx, device_name) in order.iter().enumerate() {
            if let Some(disk) = disks.iter_mut().find(|d| &d.name == device_name) {
                disk.boot_order = Some(idx as u32 + 1);
            }
        }
        let devices_patch = json!({ "disks": disks });
        if patch.get("spec").is_none() {
            patch["spec"] =
                json!({ "template": { "spec": { "domain": { "devices": devices_patch } } } });
        } else {
            patch["spec"]["template"]["spec"]["domain"]["devices"] = devices_patch;
        }
    }

    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "BOOT_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

// ---- Display -----------------------------------------------------------
// KubeVirt has no per-VM configurable VNC port/password/listen-address --
// access is always via the `/vnc` subresource the console already uses.
// Report that reality on GET; reject the unsupported knobs on POST rather
// than pretending they took effect.

pub async fn fabric_get_display(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_vm(&namespace, &name).await {
        Ok(vm) => {
            let enabled = vm
                .spec
                .template
                .spec
                .domain
                .devices
                .as_ref()
                .and_then(|d| d.autoattach_graphics_device)
                .unwrap_or(true);
            Json(json!({
                "type": "vnc",
                "listen_address": if enabled { "kubevirt-subresource" } else { "" },
                "port": 0,
                "tls_port": null,
                "password": null,
                "keymap": null,
            }))
            .into_response()
        }
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_set_display(
    State(_state): State<SharedState>,
    Path(_name): Path<String>,
) -> impl IntoResponse {
    let (st, j) = err_json(
        400,
        "NOT_CONFIGURABLE",
        "Display access is provided through KubeVirt's VNC/console subresource -- listen address, port, and password aren't independently configurable per VM",
    );
    (st, j).into_response()
}

// ---- CPU model -----------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SetCpuModelBody {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub features_add: Option<Vec<String>>,
    #[serde(default)]
    pub features_remove: Option<Vec<String>>,
}

fn cpu_model_json(cpu: Option<&crate::kube::types::CPU>) -> serde_json::Value {
    let model = cpu
        .and_then(|c| c.model.clone())
        .unwrap_or_else(|| "host-model".to_string());
    let mode = match model.as_str() {
        "host-model" | "host-passthrough" => model.clone(),
        _ => "custom".to_string(),
    };
    let features = cpu.and_then(|c| c.features.clone()).unwrap_or_default();
    let features_add: Vec<String> = features
        .iter()
        .filter(|f| !matches!(f.policy.as_deref(), Some("disable") | Some("forbid")))
        .map(|f| f.name.clone())
        .collect();
    let features_remove: Vec<String> = features
        .iter()
        .filter(|f| matches!(f.policy.as_deref(), Some("disable") | Some("forbid")))
        .map(|f| f.name.clone())
        .collect();
    json!({
        "model": model,
        "mode": mode,
        "features_add": features_add,
        "features_remove": features_remove,
        "vendor": null,
    })
}

pub async fn fabric_get_cpu_model(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_vm(&namespace, &name).await {
        Ok(vm) => Json(cpu_model_json(vm.spec.template.spec.domain.cpu.as_ref())).into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_set_cpu_model(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<SetCpuModelBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let existing_model = vm
        .spec
        .template
        .spec
        .domain
        .cpu
        .as_ref()
        .and_then(|c| c.model.clone());
    let new_model = match body.mode.as_deref() {
        Some("host-model") => "host-model".to_string(),
        Some("host-passthrough") => "host-passthrough".to_string(),
        Some("custom") => match &body.model {
            Some(m) => m.clone(),
            None => {
                let (st, j) = err_json(
                    400,
                    "VALIDATION_FAILED",
                    "model is required when mode is \"custom\"",
                );
                return (st, j).into_response();
            }
        },
        _ => body
            .model
            .or(existing_model)
            .unwrap_or_else(|| "host-model".to_string()),
    };

    let mut cpu_patch = json!({ "model": new_model });
    if body.features_add.is_some() || body.features_remove.is_some() {
        let mut features = Vec::new();
        for name in body.features_add.unwrap_or_default() {
            features.push(json!({ "name": name, "policy": "require" }));
        }
        for name in body.features_remove.unwrap_or_default() {
            features.push(json!({ "name": name, "policy": "forbid" }));
        }
        cpu_patch["features"] = json!(features);
    }

    let api = vm_api(&client, &namespace);
    let patch = json!({ "spec": { "template": { "spec": { "domain": { "cpu": cpu_patch } } } } });
    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "CPU_MODEL_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

// ---- Watchdog -----------------------------------------------------------
// Real KubeVirt schema (confirmed against the cluster's installed CRD):
// `i6300esb` is the only supported model, with actions limited to
// `poweroff`/`reset`/`shutdown` -- there is no `ib700` model or
// `pause`/`none` action, despite the frontend's WatchdogConfig type
// allowing for them. Reject those rather than silently accepting and
// dropping them, which is exactly what the previous (wrong) WatchdogDevice
// shape did.

const VALID_WATCHDOG_MODELS: &[&str] = &["i6300esb"];
const VALID_WATCHDOG_ACTIONS: &[&str] = &["reset", "shutdown", "poweroff"];

fn watchdog_config_json(
    watchdog: Option<&crate::kube::types::WatchdogDevice>,
) -> serde_json::Value {
    match watchdog.and_then(|w| w.i6300esb.as_ref()) {
        Some(i6300esb) => json!({
            "model": "i6300esb",
            "action": i6300esb.action.clone().unwrap_or_else(|| "reset".to_string()),
        }),
        None => serde_json::Value::Null,
    }
}

pub async fn fabric_get_watchdog(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_vm(&namespace, &name).await {
        Ok(vm) => {
            let watchdog = vm
                .spec
                .template
                .spec
                .domain
                .devices
                .as_ref()
                .and_then(|d| d.watchdog.as_ref());
            Json(watchdog_config_json(watchdog)).into_response()
        }
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SetWatchdogBody {
    pub model: String,
    pub action: String,
}

pub async fn fabric_set_watchdog(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<SetWatchdogBody>,
) -> impl IntoResponse {
    if !VALID_WATCHDOG_MODELS.contains(&body.model.as_str()) {
        let (st, j) = err_json(
            400,
            "INVALID_MODEL",
            &format!("model must be one of {VALID_WATCHDOG_MODELS:?}"),
        );
        return (st, j).into_response();
    }
    if !VALID_WATCHDOG_ACTIONS.contains(&body.action.as_str()) {
        let (st, j) = err_json(
            400,
            "INVALID_ACTION",
            &format!("action must be one of {VALID_WATCHDOG_ACTIONS:?}"),
        );
        return (st, j).into_response();
    }

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let api = vm_api(&client, &namespace);
    let patch = json!({
        "spec": { "template": { "spec": { "domain": { "devices": {
            "watchdog": { "name": "watchdog0", "i6300esb": { "action": body.action } }
        } } } } }
    });
    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "WATCHDOG_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

// ---- Serial console -------------------------------------------------------
// KubeVirt attaches exactly one pty-type serial console to every VMI
// automatically (the same channel `ws_console` already uses) -- there is no
// per-VM list of configurable serial devices to add to.

pub async fn fabric_get_serials(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    if let Err(e) = client.get_vm(&namespace, &name).await {
        let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
        return (st, j).into_response();
    }
    Json(json!([{ "type": "pty" }])).into_response()
}

pub async fn fabric_add_serial(
    State(_state): State<SharedState>,
    Path(_name): Path<String>,
) -> impl IntoResponse {
    let (st, j) = err_json(
        400,
        "NOT_CONFIGURABLE",
        "KubeVirt provides one fixed pty-type serial console per VM automatically; custom serial devices (unix/tcp sockets) aren't supported",
    );
    (st, j).into_response()
}

// ---- Firmware status / UEFI / secure boot / NVRAM reset -------------------

fn firmware_status_json(vm: &crate::kube::types::VirtualMachine) -> serde_json::Value {
    let domain = &vm.spec.template.spec.domain;
    let efi = domain
        .firmware
        .as_ref()
        .and_then(|f| f.bootloader.as_ref())
        .and_then(|b| b.efi.as_ref());
    let is_uefi = efi.is_some();
    let secure_boot_enabled = efi.and_then(|e| e.secure_boot).unwrap_or(false);
    let tpm_enabled = domain
        .devices
        .as_ref()
        .and_then(|d| d.tpm.as_ref())
        .is_some();
    // KubeVirt bundles OVMF and always uses these container paths for UEFI
    // guests; not read from this specific cluster, so left empty for BIOS
    // guests rather than implying a per-VM value that doesn't exist.
    let (code_path, vars_path) = if is_uefi {
        (
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
        )
    } else {
        ("", "")
    };
    json!({
        "firmware_type": if is_uefi { "uefi" } else { "bios" },
        "code_path": code_path,
        "vars_path": vars_path,
        "secure_boot_enabled": secure_boot_enabled,
        "tpm_enabled": tpm_enabled,
        "tpm_version": if tpm_enabled { Some("2.0") } else { None },
    })
}

pub async fn fabric_get_firmware_status(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    match client.get_vm(&namespace, &name).await {
        Ok(vm) => Json(firmware_status_json(&vm)).into_response(),
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnableUefiBody {
    #[serde(default)]
    pub secure_boot: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub tpm_version: Option<String>,
}

/// Build a merge-patch that sets `firmware.bootloader` to EFI (with the
/// given `secureBoot` value) or BIOS, always explicitly nulling out the
/// other option. KubeVirt's admission webhook rejects a VM with both
/// `bios` and `efi` present under `bootloader` at once, and JSON merge
/// patch only adds/overwrites keys it's given -- patching just `efi`
/// leaves a pre-existing sibling `bios` key in place, which is exactly
/// what made the first version of this handler fail live with "has both
/// EFI and BIOS configured, but they are mutually exclusive".
///
/// Also: KubeVirt requires `features.smm.enabled: true` whenever
/// `secureBoot` is true (confirmed against the cluster's CRD: "Requires
/// SMM to be enabled"), so this always folds that in alongside secure
/// boot rather than leaving callers to discover the dependency themselves.
///
/// Deliberately leaves `persistent` unset (KubeVirt's own default is
/// `false`) rather than forcing `true` -- persisting EFI NVRAM across
/// reboots requires the cluster-wide `VMPersistentState` feature gate,
/// confirmed live to be disabled on this cluster (the same gate that
/// blocks the Create VM wizard's UEFI option); forcing it on would make
/// every UEFI enable/secure-boot/reset call fail on any cluster without
/// that gate, for a property nothing here actually needs.
fn bootloader_patch(want_uefi: bool, secure_boot: bool) -> serde_json::Value {
    let bootloader = if want_uefi {
        json!({ "bios": null, "efi": { "secureBoot": secure_boot } })
    } else {
        json!({ "efi": null, "bios": {} })
    };
    let mut patch = json!({
        "spec": { "template": { "spec": { "domain": { "firmware": { "bootloader": bootloader } } } } }
    });
    if want_uefi && secure_boot {
        patch["spec"]["template"]["spec"]["domain"]["features"] =
            json!({ "smm": { "enabled": true } });
    }
    patch
}

pub async fn fabric_enable_uefi(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    AxumJson(body): AxumJson<EnableUefiBody>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let mut patch = bootloader_patch(true, body.secure_boot);
    if body.tpm_version.is_some() {
        patch["spec"]["template"]["spec"]["domain"]["devices"] = json!({ "tpm": {} });
    }
    let api = vm_api(&client, &namespace);
    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "FIRMWARE_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

async fn set_secure_boot(
    state: SharedState,
    name: String,
    enabled: bool,
) -> axum::response::Response {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let is_uefi = vm
        .spec
        .template
        .spec
        .domain
        .firmware
        .as_ref()
        .and_then(|f| f.bootloader.as_ref())
        .map(|b| b.efi.is_some())
        .unwrap_or(false);
    if !is_uefi {
        let (st, j) = err_json(
            400,
            "VALIDATION_FAILED",
            "Secure Boot requires UEFI firmware -- enable UEFI first",
        );
        return (st, j).into_response();
    }
    let api = vm_api(&client, &namespace);
    let patch = bootloader_patch(true, enabled);
    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "FIRMWARE_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

pub async fn fabric_enable_secureboot(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    set_secure_boot(state, name, true).await
}

pub async fn fabric_disable_secureboot(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    set_secure_boot(state, name, false).await
}

pub async fn fabric_reset_nvram(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let is_uefi = vm
        .spec
        .template
        .spec
        .domain
        .firmware
        .as_ref()
        .and_then(|f| f.bootloader.as_ref())
        .map(|b| b.efi.is_some())
        .unwrap_or(false);
    if !is_uefi {
        let (st, j) = err_json(
            400,
            "VALIDATION_FAILED",
            "NVRAM reset only applies to UEFI guests",
        );
        return (st, j).into_response();
    }
    let api = vm_api(&client, &namespace);
    let patch = bootloader_patch(true, false);
    match patch_vm(&api, &name, &patch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (st, j) = err_json(500, "FIRMWARE_UPDATE_FAILED", &sanitize_error(&e));
            (st, j).into_response()
        }
    }
}

// ---- CPU affinity ---------------------------------------------------------
// The pinned physical CPU IDs for a dedicated-placement VM live in the
// launcher pod's cgroup/cpuset, which this API has no access to -- report
// an honest empty list rather than fabricate pinning data.

pub async fn fabric_get_cpu_affinity(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);
    if let Err(e) = client.get_vm(&namespace, &name).await {
        let (st, j) = err_json(404, "NOT_FOUND", &sanitize_error(&e));
        return (st, j).into_response();
    }
    Json(json!(Vec::<u32>::new())).into_response()
}

// ---- System-wide capability endpoints --------------------------------------

pub async fn fabric_list_cpu_models() -> impl IntoResponse {
    // A representative, commonly-available QEMU/libvirt CPU model list --
    // not detected from this cluster's actual node hardware.
    Json(json!([
        { "name": "host-model", "vendor": "any", "features": [] },
        { "name": "host-passthrough", "vendor": "any", "features": [] },
        { "name": "Skylake-Client", "vendor": "Intel", "features": ["ss", "hle", "rtm"] },
        { "name": "Cascadelake-Server", "vendor": "Intel", "features": ["avx512f", "avx512dq"] },
        { "name": "Haswell", "vendor": "Intel", "features": ["vme", "ss", "hle", "rtm"] },
        { "name": "EPYC", "vendor": "AMD", "features": ["ibpb", "virt-ssbd"] },
        { "name": "EPYC-Rome", "vendor": "AMD", "features": ["ibpb", "virt-ssbd", "clzero"] },
    ]))
}

pub async fn fabric_firmware_capabilities() -> impl IntoResponse {
    // Reported as generally true for a modern (>=0.59) KubeVirt install --
    // this cluster's specific KubeVirt CR/feature gates aren't probed here.
    Json(json!({
        "ovmf_available": true,
        "secureboot_available": true,
        "tpm_available": true,
    }))
}
