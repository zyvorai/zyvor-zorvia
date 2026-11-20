// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Passthrough-device inventory and preflight (see `crate::devices`).
//!
//! `GET  /api/v1/devices`            what each node advertises, what KubeVirt permits (cluster.admin)
//! `POST /api/v1/devices/preflight`  would these devices / SR-IOV networks start? (vm.create)
//! VM creation runs the same preflight and refuses (422) a VM that cannot be scheduled.

use super::*;
use crate::config::HostDeviceConfig;
use crate::devices::{
    has_errors, permit_patch, placement_preflight, preflight, unpermit_patch, DeviceInventory,
    Issue, KubevirtDevices, NodeDevices, PermitError, PermitRequest, PlacementRequest,
    SriovNetwork,
};
use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use std::collections::BTreeMap;

const NAD_RESOURCE_ANNOTATION: &str = "k8s.v1.cni.cncf.io/resourceName";

/// A NetworkAttachmentDefinition lookup result.
enum Nad {
    /// The CRD is not installed (no Multus).
    NoCrd,
    NotFound,
    Found(Option<String>),
}

async fn lookup_nad(client: &kube::Client, namespace: &str, name: &str) -> Nad {
    let api = crate::migration_import::networks::attachment_api(client, namespace);
    match api.get(name).await {
        Ok(nad) => Nad::Found(
            nad.metadata
                .annotations
                .and_then(|a| a.get(NAD_RESOURCE_ANNOTATION).cloned()),
        ),
        // 404 for the *kind* (CRD absent) says "could not find the requested resource";
        // 404 for the *object* names it.
        Err(kube::Error::Api(e))
            if e.code == 404 && e.message.contains("could not find the requested resource") =>
        {
            Nad::NoCrd
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Nad::NotFound,
        // Cannot tell (RBAC, transient): report as not found rather than guess installed.
        Err(_) => Nad::NotFound,
    }
}

pub(crate) async fn collect_inventory(client: &kube::Client) -> DeviceInventory {
    let mut inv = DeviceInventory::default();
    let nodes: Api<Node> = Api::all(client.clone());
    if let Ok(list) = nodes.list(&ListParams::default()).await {
        for n in list.items {
            let alloc: BTreeMap<String, String> = n
                .status
                .and_then(|s| s.allocatable)
                .map(|a| a.into_iter().map(|(k, v)| (k, v.0)).collect())
                .unwrap_or_default();
            let labels: BTreeMap<String, String> = n
                .metadata
                .labels
                .clone()
                .map(|l| l.into_iter().collect())
                .unwrap_or_default();
            inv.nodes.push(NodeDevices::from_node(
                n.metadata.name.as_deref().unwrap_or(""),
                &alloc,
                &labels,
            ));
        }
    }
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk("kubevirt.io", "v1", "KubeVirt"));
    let kv: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    if let Ok(list) = kv.list(&ListParams::default()).await {
        if let Some(cr) = list.items.first() {
            let cfg = cr
                .data
                .pointer("/spec/configuration")
                .cloned()
                .unwrap_or_default();
            inv.kubevirt = Some(KubevirtDevices::from_configuration(&cfg));
        }
    }
    inv.multus_installed = !matches!(
        lookup_nad(client, "default", "zorvia-probe").await,
        Nad::NoCrd
    );
    if inv.multus_installed {
        let nads = list_sriov_attachments(client).await;
        inv = inv.with_sriov_pools(&nads);
    }
    inv.with_kind_hints()
}

/// `(namespace, name, resourceName)` of every attachment that names a VF pool. Needs `list` on
/// network-attachment-definitions; without it the pools are simply not listed.
async fn list_sriov_attachments(client: &kube::Client) -> Vec<(String, String, String)> {
    let mut ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "k8s.cni.cncf.io",
        "v1",
        "NetworkAttachmentDefinition",
    ));
    ar.plural = "network-attachment-definitions".into();
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    match api.list(&ListParams::default()).await {
        Ok(list) => list
            .items
            .into_iter()
            .filter_map(|nad| {
                let res = nad
                    .metadata
                    .annotations
                    .as_ref()?
                    .get(NAD_RESOURCE_ANNOTATION)?
                    .clone();
                Some((
                    nad.metadata.namespace.clone().unwrap_or_default(),
                    nad.metadata.name.clone()?,
                    res,
                ))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

async fn resolve_sriov(
    client: &kube::Client,
    namespace: &str,
    names: &[String],
) -> Vec<SriovNetwork> {
    let mut out = Vec::new();
    for n in names {
        let (exists, resource_name) = match lookup_nad(client, namespace, n).await {
            Nad::Found(r) => (true, r),
            _ => (false, None),
        };
        out.push(SriovNetwork {
            name: n.clone(),
            exists,
            resource_name,
        });
    }
    out
}

/// Names of the SR-IOV networks an interface list asks for.
pub(crate) fn sriov_names(interfaces: &[crate::config::InterfaceConfig]) -> Vec<String> {
    interfaces
        .iter()
        .filter_map(|i| match &i.network_type {
            crate::config::NetworkType::SRIOV { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Preflight for VM creation: only touches the cluster when something is requested.
pub(crate) async fn preflight_for_create(
    client: &kube::Client,
    namespace: &str,
    devices: &[HostDeviceConfig],
    sriov: &[String],
    placement: &PlacementRequest,
) -> Vec<Issue> {
    if devices.is_empty() && sriov.is_empty() && placement.is_empty() {
        return Vec::new();
    }
    let inv = collect_inventory(client).await;
    let nets = resolve_sriov(client, namespace, sriov).await;
    let mut issues = Vec::new();
    if !devices.is_empty() || !sriov.is_empty() {
        issues.extend(preflight(devices, &nets, &inv));
    }
    issues.extend(placement_preflight(placement, &inv));
    issues
}

pub async fn devices_inventory_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let client = state.read().await.client().client();
    Json(collect_inventory(&client).await).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PreflightBody {
    #[serde(default)]
    pub devices: Vec<HostDeviceConfig>,
    /// Names of SR-IOV NetworkAttachmentDefinitions in `namespace`.
    #[serde(default)]
    pub sriov_networks: Vec<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Placement the VM will ask for (same meaning as on `POST /api/vms`).
    #[serde(default)]
    pub dedicated_cpus: bool,
    #[serde(default)]
    pub numa_passthrough: bool,
    #[serde(default)]
    pub hugepages: Option<String>,
}

pub async fn devices_preflight_handler(
    State(state): State<SharedState>,
    Json(body): Json<PreflightBody>,
) -> impl IntoResponse {
    for d in &body.devices {
        if let Err(e) = d.validate() {
            let (st, j) = err_json(400, "INVALID", &e);
            return (st, j).into_response();
        }
    }
    let s = state.read().await;
    let default_ns = s.namespace.clone();
    let client = s.client().client();
    drop(s);
    let ns = body.namespace.unwrap_or(default_ns);
    let placement = PlacementRequest {
        dedicated_cpus: body.dedicated_cpus,
        numa_passthrough: body.numa_passthrough,
        hugepages: body.hugepages.clone(),
    };
    let issues = preflight_for_create(
        &client,
        &ns,
        &body.devices,
        &body.sriov_networks,
        &placement,
    )
    .await;
    Json(serde_json::json!({ "ok": !has_errors(&issues), "issues": issues })).into_response()
}

// ── Permitting devices (cluster.admin) ───────────────────────────────────────────────
//
// `POST   /api/v1/devices/permitted`                      add a device to the KubeVirt CR's permittedHostDevices
// `DELETE /api/v1/devices/permitted?resource_name=vendor.com/name`   remove it
//
// Changing which hardware VMs may take is a cluster setting, so it needs cluster.admin, is audited,
// and needs the service account to be allowed to patch the KubeVirt CR (Helm `devices.managePermitted`).

async fn kubevirt_cr(
    client: &kube::Client,
) -> Result<(DynamicObject, Api<DynamicObject>), (u16, &'static str, String)> {
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk("kubevirt.io", "v1", "KubeVirt"));
    let all: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    let list = all
        .list(&ListParams::default())
        .await
        .map_err(|e| (500, "KUBEVIRT_UNREADABLE", sanitize_error(&e)))?;
    let cr = list.items.into_iter().next().ok_or((
        404,
        "KUBEVIRT_NOT_FOUND",
        "no KubeVirt CR in the cluster".to_string(),
    ))?;
    let ns = cr.metadata.namespace.clone().unwrap_or_default();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &ns, &ar);
    Ok((cr, api))
}

fn patch_error(e: kube::Error) -> (u16, &'static str, String) {
    match e {
        kube::Error::Api(a) if a.code == 403 => (
            403,
            "RBAC_DENIED",
            "Zorvia's service account may not patch the KubeVirt CR; enable `devices.managePermitted` in the Helm values (or grant `patch` on kubevirts)".into(),
        ),
        kube::Error::Api(a) if a.code == 409 => (
            409,
            "CONFLICT",
            "the KubeVirt CR changed while this was being applied; retry".into(),
        ),
        other => (500, "PATCH_FAILED", sanitize_error(&other)),
    }
}

fn permit_error(e: PermitError) -> (u16, &'static str, String) {
    match e {
        PermitError::Invalid(m) => (400, "INVALID", m),
        PermitError::Conflict(m) => (409, "CONFLICT", m),
        PermitError::NotFound(m) => (404, "NOT_FOUND", m),
    }
}

pub async fn devices_permit_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(req): Json<PermitRequest>,
) -> impl IntoResponse {
    let client = state.read().await.client().client();
    let name = req.resource_name().to_string();
    let outcome: Result<serde_json::Value, (u16, &'static str, String)> = async {
        let (cr, api) = kubevirt_cr(&client).await?;
        let cfg = cr
            .data
            .pointer("/spec/configuration")
            .cloned()
            .unwrap_or_default();
        let rv = cr.metadata.resource_version.clone();
        match permit_patch(&cfg, &req, rv.as_deref()).map_err(permit_error)? {
            None => Ok(serde_json::json!({ "resource_name": name, "changed": false })),
            Some(patch) => {
                api.patch(
                    cr.metadata.name.as_deref().unwrap_or("kubevirt"),
                    &kube::api::PatchParams::default(),
                    &kube::api::Patch::Merge(&patch),
                )
                .await
                .map_err(patch_error)?;
                Ok(serde_json::json!({ "resource_name": name, "changed": true }))
            }
        }
    }
    .await;
    audit_permit(&state, &headers, &name, "permit", outcome.as_ref().err()).await;
    reply(outcome)
}

#[derive(Debug, Deserialize)]
pub struct UnpermitQuery {
    pub resource_name: String,
}

pub async fn devices_unpermit_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<UnpermitQuery>,
) -> impl IntoResponse {
    let client = state.read().await.client().client();
    let name = q.resource_name.clone();
    let outcome: Result<serde_json::Value, (u16, &'static str, String)> = async {
        let (cr, api) = kubevirt_cr(&client).await?;
        let cfg = cr
            .data
            .pointer("/spec/configuration")
            .cloned()
            .unwrap_or_default();
        let rv = cr.metadata.resource_version.clone();
        let patch = unpermit_patch(&cfg, &name, rv.as_deref()).map_err(permit_error)?;
        api.patch(
            cr.metadata.name.as_deref().unwrap_or("kubevirt"),
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&patch),
        )
        .await
        .map_err(patch_error)?;
        Ok(serde_json::json!({ "resource_name": name, "changed": true }))
    }
    .await;
    audit_permit(&state, &headers, &name, "unpermit", outcome.as_ref().err()).await;
    reply(outcome)
}

fn reply(
    outcome: Result<serde_json::Value, (u16, &'static str, String)>,
) -> axum::response::Response {
    match outcome {
        Ok(v) => Json(v).into_response(),
        Err((code, c, m)) => {
            let (st, j) = err_json(code, c, &m);
            (st, j).into_response()
        }
    }
}

async fn audit_permit(
    state: &SharedState,
    headers: &HeaderMap,
    resource: &str,
    verb: &str,
    err: Option<&(u16, &'static str, String)>,
) {
    record_audit(
        state,
        headers,
        crate::audit_trail::AuditAction::ConfigChange,
        "kubevirt-permitted-device",
        resource,
        err.is_none(),
        Some(match err {
            None => format!("{verb} {resource}"),
            Some((_, c, m)) => format!("{verb} {resource} failed: {c}: {m}"),
        }),
    )
    .await;
}
