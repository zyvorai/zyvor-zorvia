// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Collect platform status from the Kubernetes cluster.

use anyhow::Result;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, ListParams};
use kube::Client;

use super::ansi::ErrorCount;
use super::format::{FeatureStatus, PlatformStatus, PodStateCount};

const KUBEVIRT_NS: &[&str] = &["kubevirt", "kubevirt-system"];
const CDI_NS: &[&str] = &["cdi", "cdi-system"];
const ZORVIA_NS: &[&str] = &["zorvia-system"];
const ROOK_NS: &[&str] = &["rook-ceph"];
const SNAPSHOT_NS: &[&str] = &["kube-system", "snapshot-controller"];

const KUBEVIRT_DEPS: &[&str] = &["virt-operator", "virt-api", "virt-controller"];
const KUBEVIRT_DS: &[&str] = &["virt-handler"];
const CDI_DEPS: &[&str] = &["cdi-operator", "cdi-deployment", "cdi-apiserver"];
const ZORVIA_DEPS: &[&str] = &["zorvia-api"];
const ROOK_DEPS: &[&str] = &["rook-ceph-operator"];
const SNAPSHOT_DEPS: &[&str] = &["snapshot-controller"];

/// Probe the cluster and build a [`PlatformStatus`].
pub async fn collect(client: Client) -> PlatformStatus {
    let mut status = PlatformStatus::default();

    // --- KubeVirt ---
    let kv = probe_group(
        &client,
        KUBEVIRT_NS,
        KUBEVIRT_DEPS,
        KUBEVIRT_DS,
        &mut status,
    )
    .await;
    status.components.insert("kubevirt".into(), kv);

    // --- CDI ---
    let cdi = probe_group(&client, CDI_NS, CDI_DEPS, &[], &mut status).await;
    status.components.insert("cdi".into(), cdi);

    // --- Zorvia API ---
    let za = probe_group(&client, ZORVIA_NS, ZORVIA_DEPS, &[], &mut status).await;
    status.components.insert("zorvia-api".into(), za);

    // --- Snapshots (controller + CRD) ---
    let mut snap = probe_group(&client, SNAPSHOT_NS, SNAPSHOT_DEPS, &[], &mut status).await;
    if snap.disabled {
        // CRD alone is enough to count as present-but-maybe-degraded
        match crd_exists(&client, "volumesnapshots.snapshot.storage.k8s.io").await {
            Ok(true) => {
                snap.disabled = false;
            }
            Ok(false) => {}
            Err(e) => status
                .collection_errors
                .push(format!("snapshots CRD probe: {e}")),
        }
    }
    status.components.insert("snapshots".into(), snap);

    // --- Rook ---
    let rook = probe_group(&client, ROOK_NS, ROOK_DEPS, &[], &mut status).await;
    status.components.insert("rook".into(), rook);

    // --- Cluster VMs ---
    {
        use crate::kube::types::VirtualMachine;
        let vms: Api<VirtualMachine> = Api::all(client.clone());
        match vms.list(&ListParams::default()).await {
            Ok(list) => {
                status.vms_total = list.items.len();
                status.vms_running = list
                    .items
                    .iter()
                    .filter(|vm| vm.spec.running.unwrap_or(false))
                    .count();
            }
            Err(e) => status.collection_errors.push(format!("list VMs: {e}")),
        }
    }

    status.features = collect_features(&status);
    status
}

async fn probe_group(
    client: &Client,
    namespaces: &[&str],
    deployments: &[&str],
    daemonsets: &[&str],
    status: &mut PlatformStatus,
) -> ErrorCount {
    let mut found_any = false;
    let mut ec = ErrorCount::default();

    for ns in namespaces {
        for name in deployments {
            match get_deployment(client, ns, name).await {
                Ok(Some(dep)) => {
                    found_any = true;
                    ingest_deployment(name, &dep, status, &mut ec);
                    ingest_pods_for(client, ns, name, status).await;
                }
                Ok(None) => {}
                Err(e) => {
                    status
                        .collection_errors
                        .push(format!("deployment {ns}/{name}: {e}"));
                }
            }
        }
        for name in daemonsets {
            match get_daemonset(client, ns, name).await {
                Ok(Some(ds)) => {
                    found_any = true;
                    ingest_daemonset(name, &ds, status, &mut ec);
                    ingest_pods_for(client, ns, name, status).await;
                }
                Ok(None) => {}
                Err(e) => {
                    status
                        .collection_errors
                        .push(format!("daemonset {ns}/{name}: {e}"));
                }
            }
        }
    }

    if !found_any {
        ec.disabled = true;
    }
    ec
}

fn ingest_deployment(
    name: &str,
    dep: &Deployment,
    status: &mut PlatformStatus,
    ec: &mut ErrorCount,
) {
    let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let ready = dep
        .status
        .as_ref()
        .and_then(|s| s.ready_replicas)
        .unwrap_or(0);
    let available = dep
        .status
        .as_ref()
        .and_then(|s| s.available_replicas)
        .unwrap_or(0);
    let unavailable = dep
        .status
        .as_ref()
        .and_then(|s| s.unavailable_replicas)
        .unwrap_or(0);

    status.pod_state.insert(
        name.to_string(),
        PodStateCount {
            type_name: "Deployment".into(),
            desired,
            ready,
            available,
            unavailable,
        },
    );

    if ready < desired {
        ec.errors
            .push(format!("{name}: only {ready}/{desired} ready"));
        let entry = status
            .errors
            .entry(name.to_string())
            .or_default()
            .entry(name.to_string())
            .or_default();
        entry.errors.push(format!("only {ready}/{desired} ready"));
    }
}

fn ingest_daemonset(name: &str, ds: &DaemonSet, status: &mut PlatformStatus, ec: &mut ErrorCount) {
    let desired = ds
        .status
        .as_ref()
        .map(|s| s.desired_number_scheduled)
        .unwrap_or(0);
    let ready = ds.status.as_ref().map(|s| s.number_ready).unwrap_or(0);
    let available = ds
        .status
        .as_ref()
        .and_then(|s| s.number_available)
        .unwrap_or(0);
    let unavailable = ds
        .status
        .as_ref()
        .and_then(|s| s.number_unavailable)
        .unwrap_or(0);

    status.pod_state.insert(
        name.to_string(),
        PodStateCount {
            type_name: "DaemonSet".into(),
            desired,
            ready,
            available,
            unavailable,
        },
    );

    if desired > 0 && ready < desired {
        ec.warnings
            .push(format!("{name}: only {ready}/{desired} ready"));
        let entry = status
            .errors
            .entry(name.to_string())
            .or_default()
            .entry(name.to_string())
            .or_default();
        entry.warnings.push(format!("only {ready}/{desired} ready"));
    }
}

async fn ingest_pods_for(client: &Client, ns: &str, app_name: &str, status: &mut PlatformStatus) {
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    // Match by name prefix (virt-handler-*, zorvia-api-*, …)
    let lp = ListParams::default();
    let Ok(list) = pods.list(&lp).await else {
        return;
    };
    for pod in list.items {
        let pname = pod.metadata.name.unwrap_or_default();
        if !pname.starts_with(app_name) {
            continue;
        }
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".into());
        *status
            .phase_count
            .entry(app_name.to_string())
            .or_default()
            .entry(phase)
            .or_default() += 1;

        if let Some(spec) = &pod.spec {
            for c in spec.containers.iter() {
                *status
                    .image_count
                    .entry(app_name.to_string())
                    .or_default()
                    .entry(c.image.clone().unwrap_or_default())
                    .or_default() += 1;
            }
        }
    }
}

async fn get_deployment(client: &Client, ns: &str, name: &str) -> Result<Option<Deployment>> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), ns);
    match api.get(name).await {
        Ok(d) => Ok(Some(d)),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn get_daemonset(client: &Client, ns: &str, name: &str) -> Result<Option<DaemonSet>> {
    let api: Api<DaemonSet> = Api::namespaced(client.clone(), ns);
    match api.get(name).await {
        Ok(d) => Ok(Some(d)),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn crd_exists(client: &Client, name: &str) -> Result<bool> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    match api.get(name).await {
        Ok(_) => Ok(true),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn collect_features(status: &PlatformStatus) -> Vec<FeatureStatus> {
    let kv_ok = status
        .components
        .get("kubevirt")
        .map(|c| !c.disabled && c.errors.is_empty())
        .unwrap_or(false);
    let snap_ok = status
        .components
        .get("snapshots")
        .map(|c| !c.disabled)
        .unwrap_or(false);
    let rook_ok = status
        .components
        .get("rook")
        .map(|c| !c.disabled)
        .unwrap_or(false);
    let kryton = std::env::var("KRYTON_URL")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let atlas = std::env::var("ATLAS_URL")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);

    let ok = || ErrorCount::default();
    let disabled = || ErrorCount {
        disabled: true,
        ..Default::default()
    };
    let from_bool = |b: bool| if b { ok() } else { disabled() };

    vec![
        FeatureStatus {
            name: "Backup".into(),
            state: from_bool(snap_ok || kv_ok),
        },
        FeatureStatus {
            name: "HA".into(),
            state: from_bool(kv_ok),
        },
        FeatureStatus {
            name: "Live migration".into(),
            state: from_bool(kv_ok),
        },
        FeatureStatus {
            name: "Security".into(),
            state: ok(), // CLI-native
        },
        FeatureStatus {
            name: "Cost".into(),
            state: ok(),
        },
        FeatureStatus {
            name: "Multi-tenancy".into(),
            state: ok(),
        },
        FeatureStatus {
            name: "Observability".into(),
            state: ok(),
        },
        FeatureStatus {
            name: "Golden images".into(),
            state: from_bool(
                status
                    .components
                    .get("cdi")
                    .map(|c| !c.disabled)
                    .unwrap_or(false),
            ),
        },
        FeatureStatus {
            name: "Kryton".into(),
            state: from_bool(kryton),
        },
        FeatureStatus {
            name: "Atlas".into(),
            state: from_bool(atlas),
        },
        FeatureStatus {
            name: "GitOps".into(),
            state: ok(),
        },
        FeatureStatus {
            name: "Rook/Ceph".into(),
            state: from_bool(rook_ok),
        },
    ]
}

/// Collect using a freshly inferred kubeconfig.
pub async fn collect_default() -> Result<PlatformStatus> {
    let client = Client::try_default().await?;
    Ok(collect(client).await)
}
