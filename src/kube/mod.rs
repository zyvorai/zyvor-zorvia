pub mod backup_job;
pub mod catalog;
pub mod cdi;
pub mod converter;
pub mod expose;
pub mod guest_metrics;
pub mod guest_ready;
pub mod lifecycle;
pub mod prom;
pub mod rescue;
pub mod ssh;
pub mod status;
pub mod types;

use crate::config::VMConfig;
use crate::utils::ZorviaError;
use anyhow::Result;
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::{
    api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams},
    Client, Config,
};
use serde_json::json;

pub use converter::vm_config_to_kubevirt;
pub use status::{ResourceSummary, VMStatus};
pub use types::*;

/// Outcome of waiting for KubeVirt to actually apply a hotplug patch live.
/// A successful merge-patch only means "the API server accepted my desired
/// spec" -- KubeVirt's live-update reconciliation is asynchronous, so that
/// is not the same thing as "the guest is now running with it."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugOutcome {
    /// The VMI's live status now matches the requested value.
    Converged,
    /// KubeVirt raised a `RestartRequired` condition on the VM -- the
    /// desired spec is staged but only takes effect on the VM's next
    /// restart, not live.
    RestartRequired,
    /// Neither happened within the poll window; KubeVirt may still be
    /// reconciling, or the cluster's `liveUpdateConfiguration` may not be
    /// set (see the `KubeVirt` CR's `.spec.configuration`), in which case
    /// it never will.
    Pending,
}

/// Kubernetes client for managing KubeVirt VMs
#[derive(Clone)]
pub struct KubeClient {
    client: Client,
}

impl KubeClient {
    /// Create a new KubeClient from default kubeconfig
    pub async fn new() -> Result<Self> {
        let config = Config::infer().await?;
        let client = Client::try_from(config)?;
        Ok(Self { client })
    }

    /// Like [`KubeClient::new`], but when no usable kubeconfig exists and
    /// `ZORVIA_ALLOW_NO_CLUSTER=1`, returns a client pointed at an unreachable
    /// address so the web console still starts (every cluster call then fails
    /// with a connection error instead of the whole server refusing to run).
    pub async fn new_or_offline() -> Result<Self> {
        match Self::new().await {
            Ok(c) => Ok(c),
            Err(e) if std::env::var("ZORVIA_ALLOW_NO_CLUSTER").as_deref() == Ok("1") => {
                log::warn!("no usable kubeconfig ({e}); starting without a cluster");
                let uri = "https://127.0.0.1:9".parse::<http::Uri>()?;
                let client = Client::try_from(Config::new(uri))?;
                Ok(Self { client })
            }
            Err(e) => Err(e),
        }
    }

    /// Create a new KubeClient with custom kubeconfig path
    pub async fn with_kubeconfig(path: &str) -> Result<Self> {
        let kubeconfig = kube::config::Kubeconfig::read_from(path)?;
        let config =
            Config::from_custom_kubeconfig(kubeconfig, &kube::config::KubeConfigOptions::default())
                .await?;
        let client = Client::try_from(config)?;
        Ok(Self { client })
    }

    /// Get a clone of the underlying kube::Client
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Get API handle for VirtualMachines in a namespace
    fn vm_api(&self, namespace: &str) -> Api<VirtualMachine> {
        Api::namespaced(self.client.clone(), namespace)
    }

    /// Get API handle for PVCs in a namespace
    fn pvc_api(&self, namespace: &str) -> Api<PersistentVolumeClaim> {
        Api::namespaced(self.client.clone(), namespace)
    }

    /// List all VMs in a namespace
    pub async fn list_vms(&self, namespace: &str) -> Result<Vec<VirtualMachine>> {
        let vms: Api<VirtualMachine> = self.vm_api(namespace);
        let lp = ListParams::default();
        let vm_list = vms.list(&lp).await?;
        Ok(vm_list.items)
    }

    /// List all VMs across all namespaces
    pub async fn list_all_vms(&self) -> Result<Vec<VirtualMachine>> {
        let vms: Api<VirtualMachine> = Api::all(self.client.clone());
        let lp = ListParams::default();
        let vm_list = vms.list(&lp).await?;
        Ok(vm_list.items)
    }

    /// Get a specific VM
    pub async fn get_vm(&self, namespace: &str, name: &str) -> Result<VirtualMachine> {
        let vms: Api<VirtualMachine> = self.vm_api(namespace);
        let vm = vms.get(name).await?;
        Ok(vm)
    }

    /// Create a VM from VMConfig
    pub async fn create_vm(&self, config: &VMConfig) -> Result<VirtualMachine> {
        let vms: Api<VirtualMachine> = self.vm_api(&config.namespace);

        // Check if VM already exists
        match vms.get(&config.name).await {
            Ok(_) => return Err(ZorviaError::VmExists(config.name.clone()).into()),
            Err(kube::Error::Api(ae)) if ae.code == 404 => {} // VM doesn't exist, proceed
            Err(e) => return Err(e.into()), // Propagate other errors (network, auth, etc.)
        }

        // Convert VMConfig to KubeVirt VirtualMachine
        let vm = vm_config_to_kubevirt(config)?;

        // Create the VM
        let pp = PostParams::default();
        let created = vms.create(&pp, &vm).await?;

        Ok(created)
    }

    /// Delete a VM
    pub async fn delete_vm(&self, namespace: &str, name: &str) -> Result<()> {
        let vms: Api<VirtualMachine> = self.vm_api(namespace);

        // Check if VM exists
        match vms.get(name).await {
            Ok(_) => {}
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                return Err(ZorviaError::VmNotFound(name.to_string()).into());
            }
            Err(e) => return Err(e.into()),
        }

        let dp = DeleteParams::default();
        vms.delete(name, &dp).await?;

        Ok(())
    }

    /// Start a VM (set running: true)
    pub async fn start_vm(&self, namespace: &str, name: &str) -> Result<VirtualMachine> {
        let vms: Api<VirtualMachine> = self.vm_api(namespace);

        // Check if VM exists
        match vms.get(name).await {
            Ok(_) => {}
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                return Err(ZorviaError::VmNotFound(name.to_string()).into());
            }
            Err(e) => return Err(e.into()),
        }

        let patch = json!({
            "spec": {
                "running": true
            }
        });

        let pp = PatchParams::default();
        let patched = vms.patch(name, &pp, &Patch::Merge(&patch)).await?;

        Ok(patched)
    }

    /// Stop a VM (set running: false)
    pub async fn stop_vm(&self, namespace: &str, name: &str) -> Result<VirtualMachine> {
        let vms: Api<VirtualMachine> = self.vm_api(namespace);

        // Check if VM exists
        match vms.get(name).await {
            Ok(_) => {}
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                return Err(ZorviaError::VmNotFound(name.to_string()).into());
            }
            Err(e) => return Err(e.into()),
        }

        let patch = json!({
            "spec": {
                "running": false
            }
        });

        let pp = PatchParams::default();
        let patched = vms.patch(name, &pp, &Patch::Merge(&patch)).await?;

        Ok(patched)
    }

    /// Restart a VM (stop then start, polling for shutdown)
    pub async fn restart_vm(&self, namespace: &str, name: &str) -> Result<VirtualMachine> {
        self.stop_vm(namespace, name).await?;
        // Poll for VM to stop (max 30 seconds)
        let mut stopped = false;
        for _ in 0..30 {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            match self.is_running(namespace, name).await {
                Ok(false) => {
                    stopped = true;
                    break;
                }
                Ok(true) => continue,
                Err(e) => {
                    log::warn!("Error checking VM status during restart: {}", e);
                    return Err(e);
                }
            }
        }
        if !stopped {
            anyhow::bail!("VM '{}' did not stop within 30 seconds", name);
        }
        self.start_vm(namespace, name).await
    }

    /// Check if a VM is running
    pub async fn is_running(&self, namespace: &str, name: &str) -> Result<bool> {
        let vm = self.get_vm(namespace, name).await?;
        Ok(vm.status.as_ref().and_then(|s| s.ready).unwrap_or(false))
    }

    /// Get VM status string
    pub async fn get_status(&self, namespace: &str, name: &str) -> Result<String> {
        let vm = self.get_vm(namespace, name).await?;

        if let Some(status) = &vm.status {
            if let Some(printable) = &status.printable_status {
                return Ok(printable.clone());
            }
            if status.ready.unwrap_or(false) {
                return Ok("Running".to_string());
            }
            if status.created.unwrap_or(false) {
                return Ok("Created".to_string());
            }
        }

        if vm.spec.running.unwrap_or(false) {
            Ok("Starting".to_string())
        } else {
            Ok("Stopped".to_string())
        }
    }

    /// Get the VirtualMachineInstance (running instance) for a VM
    pub async fn get_vmi(&self, namespace: &str, name: &str) -> Result<VirtualMachineInstance> {
        let vmis: Api<VirtualMachineInstance> = Api::namespaced(self.client.clone(), namespace);
        let vmi = vmis.get(name).await?;
        Ok(vmi)
    }

    /// Get the IP address of a running VM via its VMI
    pub async fn get_vm_ip(&self, namespace: &str, name: &str) -> Result<Option<String>> {
        match self.get_vmi(namespace, name).await {
            Ok(vmi) => {
                if let Some(status) = &vmi.status {
                    // Return the first non-empty IP address found
                    for iface in &status.interfaces {
                        if let Some(ip) = &iface.ip_address {
                            if !ip.is_empty() {
                                return Ok(Some(ip.clone()));
                            }
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => {
                // VMI not found means VM is not running — no IP available
                if let Some(kube::Error::Api(ae)) = e.downcast_ref::<kube::Error>() {
                    if ae.code == 404 {
                        return Ok(None);
                    }
                }
                // Propagate real errors (auth, network, etc.)
                Err(e)
            }
        }
    }

    /// Get the node a VM is running on via its VMI
    pub async fn get_vm_node(&self, namespace: &str, name: &str) -> Result<Option<String>> {
        match self.get_vmi(namespace, name).await {
            Ok(vmi) => Ok(vmi.status.and_then(|s| s.node_name)),
            Err(e) => {
                // VMI not found means VM is not running — no node available
                if let Some(kube::Error::Api(ae)) = e.downcast_ref::<kube::Error>() {
                    if ae.code == 404 {
                        return Ok(None);
                    }
                }
                // Propagate real errors
                Err(e)
            }
        }
    }

    /// Create a PVC for a VM disk
    pub async fn create_pvc(
        &self,
        namespace: &str,
        name: &str,
        size: &str,
        storage_class: Option<&str>,
    ) -> Result<PersistentVolumeClaim> {
        use k8s_openapi::api::core::v1::PersistentVolumeClaimSpec;
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
        use std::collections::BTreeMap;

        let pvcs: Api<PersistentVolumeClaim> = self.pvc_api(namespace);

        let mut requests = BTreeMap::new();
        requests.insert("storage".to_string(), Quantity(size.to_string()));

        let pvc = PersistentVolumeClaim {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: Some(vec!["ReadWriteOnce".to_string()]),
                resources: Some(k8s_openapi::api::core::v1::VolumeResourceRequirements {
                    requests: Some(requests),
                    ..Default::default()
                }),
                storage_class_name: storage_class.map(|s| s.to_string()),
                ..Default::default()
            }),
            status: None,
        };

        let pp = PostParams::default();
        let created = pvcs.create(&pp, &pvc).await?;

        Ok(created)
    }

    /// Fetch a PVC by name.
    pub async fn get_pvc(&self, namespace: &str, name: &str) -> Result<PersistentVolumeClaim> {
        let pvcs: Api<PersistentVolumeClaim> = self.pvc_api(namespace);
        Ok(pvcs.get(name).await?)
    }

    /// Pause a running VMI via the KubeVirt `pause` subresource.
    pub async fn pause_vm(&self, namespace: &str, name: &str) -> Result<()> {
        self.vmi_subresource(namespace, name, "pause").await
    }

    /// Resume a paused VMI via the KubeVirt `unpause` subresource.
    pub async fn resume_vm(&self, namespace: &str, name: &str) -> Result<()> {
        self.vmi_subresource(namespace, name, "unpause").await
    }

    /// Whether the VMI currently reports the Paused condition.
    pub async fn is_paused(&self, namespace: &str, name: &str) -> Result<bool> {
        let vmi = self.get_vmi(namespace, name).await?;
        let phase = vmi.status.as_ref().and_then(|s| s.phase.as_deref());
        let conds: Vec<(&str, &str)> = vmi
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|cs| {
                cs.iter()
                    .map(|c| (c.type_.as_str(), c.status.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(lifecycle::status_is_paused(None, phase, &conds))
    }

    async fn vmi_subresource(&self, namespace: &str, name: &str, subresource: &str) -> Result<()> {
        self.vmi_subresource_with_body(namespace, name, subresource, json!({}))
            .await
    }

    /// PUT a VMI subresource with a real JSON body (addvolume/removevolume/
    /// addinterface/removeinterface all require a payload, unlike pause/
    /// unpause/freeze/unfreeze which accept an empty body).
    async fn vmi_subresource_with_body(
        &self,
        namespace: &str,
        name: &str,
        subresource: &str,
        body: serde_json::Value,
    ) -> Result<()> {
        let path = lifecycle::vmi_subresource_path(namespace, name, subresource)?;
        let req = http::Request::builder()
            .method(http::Method::PUT)
            .uri(&path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body)?)
            .map_err(|e| anyhow::anyhow!("failed to build {subresource} request: {e}"))?;

        match self.client.request::<serde_json::Value>(req).await {
            Ok(_) => Ok(()),
            Err(kube::Error::SerdeError(_)) => Ok(()),
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                log::debug!(
                    "{subresource} 404 for '{name}': reason={:?} message={:?}",
                    ae.reason,
                    ae.message
                );
                // A 404 here is ambiguous: it fires both when the VMI itself
                // doesn't exist AND when the subresource route isn't
                // registered on this KubeVirt version (e.g. addinterface/
                // removeinterface without NIC-hotplug support) — the
                // apiserver returns the same status code either way. Only
                // the VMI-missing case actually names the object; a missing
                // route's ErrorResponse is the generic apiserver "could not
                // find the requested resource" with no VMI reference.
                if ae.message.contains(name) || ae.reason.eq_ignore_ascii_case("NotFound") {
                    Err(ZorviaError::VmNotFound(name.to_string()).into())
                } else {
                    Err(anyhow::anyhow!(
                        "UNSUPPORTED: the '{subresource}' subresource is not available on this cluster's KubeVirt version"
                    ))
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Hotplug CPU: `target_vcpus` is the desired total vCPU count. Cores and
    /// threads are fixed at VM-create time, so this solves for a new socket
    /// count and patches `spec.template.spec.domain.cpu.sockets`. Requires
    /// `cpu.maxSockets` to have been declared when the VM was created.
    ///
    /// Returns the outcome of waiting (bounded) for KubeVirt to actually
    /// converge the live VMI to this topology, not just accept the patch --
    /// see `HotplugOutcome`.
    pub async fn hotplug_cpu(
        &self,
        namespace: &str,
        name: &str,
        target_vcpus: u32,
    ) -> Result<(VirtualMachine, HotplugOutcome)> {
        let vm = self.get_vm(namespace, name).await?;
        let cpu = vm
            .spec
            .template
            .spec
            .domain
            .cpu
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VM '{name}' has no CPU topology configured"))?;
        let max_sockets = cpu.max_sockets.ok_or_else(|| {
            anyhow::anyhow!(
                "HOTPLUG_NOT_PROVISIONED: VM '{name}' was not created with cpu.maxSockets; recreate it with hotplug headroom to enable CPU hotplug"
            )
        })?;
        let cores = cpu.cores.unwrap_or(1).max(1);
        let threads = cpu.threads.unwrap_or(1).max(1);
        let per_socket = cores.saturating_mul(threads).max(1);
        let new_sockets = target_vcpus.saturating_add(per_socket - 1) / per_socket;
        let new_sockets = new_sockets.max(1);
        if new_sockets > max_sockets {
            anyhow::bail!(
                "HOTPLUG_LIMIT_EXCEEDED: {target_vcpus} vCPUs requires {new_sockets} sockets, exceeding maxSockets={max_sockets} for VM '{name}'"
            );
        }

        let vms: Api<VirtualMachine> = self.vm_api(namespace);
        let patch = json!({
            "spec": { "template": { "spec": { "domain": { "cpu": { "sockets": new_sockets } } } } }
        });
        let pp = PatchParams::default();
        let patched = vms.patch(name, &pp, &Patch::Merge(&patch)).await?;

        let outcome = self
            .poll_hotplug_convergence(namespace, name, |vmi| {
                vmi.status
                    .as_ref()
                    .and_then(|s| s.current_cpu_topology.as_ref())
                    .and_then(|t| t.sockets)
                    == Some(new_sockets)
            })
            .await;
        Ok((patched, outcome))
    }

    /// Hotplug memory: `target_guest` is an absolute Quantity string (e.g.
    /// "6Gi"). Requires `memory.maxGuest` to have been declared when the VM
    /// was created; rejects targets exceeding that ceiling.
    ///
    /// Returns the outcome of waiting (bounded) for KubeVirt to actually
    /// converge the live VMI to this amount, not just accept the patch --
    /// see `HotplugOutcome`.
    pub async fn hotplug_memory(
        &self,
        namespace: &str,
        name: &str,
        target_guest: &str,
    ) -> Result<(VirtualMachine, HotplugOutcome)> {
        let vm = self.get_vm(namespace, name).await?;
        let memory = vm
            .spec
            .template
            .spec
            .domain
            .memory
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VM '{name}' has no memory configured"))?;
        let max_guest = memory.max_guest.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "HOTPLUG_NOT_PROVISIONED: VM '{name}' was not created with memory.maxGuest; recreate it with hotplug headroom to enable memory hotplug"
            )
        })?;
        let max_bytes = crate::storage::parse_size_to_bytes(&max_guest)
            .ok_or_else(|| anyhow::anyhow!("could not parse maxGuest quantity '{max_guest}'"))?;
        let target_bytes = crate::storage::parse_size_to_bytes(target_guest).ok_or_else(|| {
            anyhow::anyhow!("could not parse requested memory quantity '{target_guest}'")
        })?;
        if target_bytes > max_bytes {
            anyhow::bail!(
                "HOTPLUG_LIMIT_EXCEEDED: requested {target_guest} exceeds maxGuest={max_guest} for VM '{name}'"
            );
        }

        let vms: Api<VirtualMachine> = self.vm_api(namespace);
        let patch = json!({
            "spec": { "template": { "spec": { "domain": { "memory": { "guest": target_guest } } } } }
        });
        let pp = PatchParams::default();
        let patched = vms.patch(name, &pp, &Patch::Merge(&patch)).await?;

        let outcome = self
            .poll_hotplug_convergence(namespace, name, |vmi| {
                vmi.status
                    .as_ref()
                    .and_then(|s| s.memory.as_ref())
                    .and_then(|m| m.guest_current.as_deref())
                    .and_then(crate::storage::parse_size_to_bytes)
                    == Some(target_bytes)
            })
            .await;
        Ok((patched, outcome))
    }

    /// Poll (bounded, ~10s) for a hotplug patch to either converge live on
    /// the VMI (per `converged`) or surface a `RestartRequired` condition on
    /// the VM -- KubeVirt live-update reconciliation is asynchronous, so
    /// neither is guaranteed to have happened by the time the patch call
    /// above returns.
    async fn poll_hotplug_convergence(
        &self,
        namespace: &str,
        name: &str,
        converged: impl Fn(&VirtualMachineInstance) -> bool,
    ) -> HotplugOutcome {
        const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1000);
        const POLL_ATTEMPTS: u32 = 10;
        for attempt in 0..POLL_ATTEMPTS {
            if let Ok(vmi) = self.get_vmi(namespace, name).await {
                if converged(&vmi) {
                    return HotplugOutcome::Converged;
                }
            }
            if let Ok(vm) = self.get_vm(namespace, name).await {
                let restart_required = vm
                    .status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .is_some_and(|conds| {
                        conds
                            .iter()
                            .any(|c| c.type_ == "RestartRequired" && c.status == "True")
                    });
                if restart_required {
                    return HotplugOutcome::RestartRequired;
                }
            }
            if attempt + 1 < POLL_ATTEMPTS {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
        HotplugOutcome::Pending
    }

    /// Rewrite the `cloudinitdisk` volume's `cloudInitNoCloud.userData` on
    /// the VM object. This patches the *desired* spec, not a live volume --
    /// KubeVirt cloud-init is baked in at boot, so the new content only
    /// takes effect the next time the VM (re)starts, same as any other
    /// domain-spec change to a VM that's currently running.
    pub async fn update_cloud_init(
        &self,
        namespace: &str,
        name: &str,
        user_data: &str,
    ) -> Result<VirtualMachine> {
        let vm = self.get_vm(namespace, name).await?;
        let mut volumes = vm
            .spec
            .template
            .spec
            .volumes
            .clone()
            .ok_or_else(|| anyhow::anyhow!("VM '{name}' has no volumes"))?;
        let cloudinit = volumes
            .iter_mut()
            .find(|v| v.cloud_init_no_cloud.is_some())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "VM '{name}' has no cloudInitNoCloud volume -- it wasn't created with cloud-init enabled"
                )
            })?;
        let source = cloudinit.cloud_init_no_cloud.get_or_insert(
            crate::kube::types::CloudInitNoCloudSource {
                user_data: None,
                network_data: None,
            },
        );
        source.user_data = Some(user_data.to_string());

        let vms: Api<VirtualMachine> = self.vm_api(namespace);
        let patch = json!({
            "spec": { "template": { "spec": { "volumes": volumes } } }
        });
        let pp = PatchParams::default();
        Ok(vms.patch(name, &pp, &Patch::Merge(&patch)).await?)
    }

    /// Attach an existing PVC or DataVolume to a running VMI by name via the
    /// `addvolume` subresource.
    pub async fn add_volume(
        &self,
        namespace: &str,
        name: &str,
        volume_name: &str,
        claim_name: &str,
        bus: Option<&str>,
        is_data_volume: bool,
    ) -> Result<()> {
        self.get_pvc(namespace, claim_name).await.map_err(|_| {
            anyhow::anyhow!(
                "VOLUME_NOT_FOUND: PVC or DataVolume '{claim_name}' does not exist in namespace '{namespace}'"
            )
        })?;
        let bus = bus.unwrap_or("scsi");
        let volume_source = if is_data_volume {
            json!({ "dataVolume": { "name": claim_name } })
        } else {
            json!({ "persistentVolumeClaim": { "claimName": claim_name } })
        };
        let body = json!({
            "name": volume_name,
            "disk": { "name": volume_name, "disk": { "bus": bus } },
            "volumeSource": volume_source,
        });
        self.vmi_subresource_with_body(namespace, name, "addvolume", body)
            .await
    }

    /// Detach a previously hotplugged volume by its volume name.
    pub async fn remove_volume(
        &self,
        namespace: &str,
        name: &str,
        volume_name: &str,
    ) -> Result<()> {
        let body = json!({ "name": volume_name });
        self.vmi_subresource_with_body(namespace, name, "removevolume", body)
            .await
    }

    /// Attach an existing Multus `NetworkAttachmentDefinition` to a running
    /// VMI by network name via the `addinterface` subresource (alpha feature,
    /// requires the KubeVirt `HotplugNICs` feature gate).
    pub async fn add_interface(
        &self,
        namespace: &str,
        name: &str,
        iface_name: &str,
        network_name: &str,
    ) -> Result<()> {
        let body = json!({ "name": iface_name, "networkName": network_name });
        self.vmi_subresource_with_body(namespace, name, "addinterface", body)
            .await
    }

    /// Detach a hotplugged interface by its interface name.
    pub async fn remove_interface(
        &self,
        namespace: &str,
        name: &str,
        iface_name: &str,
    ) -> Result<()> {
        let body = json!({ "name": iface_name });
        self.vmi_subresource_with_body(namespace, name, "removeinterface", body)
            .await
    }

    /// Start a live migration by creating a `VirtualMachineInstanceMigration`.
    /// KubeVirt's scheduler selects the target node; there is no user-supplied
    /// destination.
    pub async fn migrate_vm(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<VirtualMachineInstanceMigration> {
        lifecycle::validate_k8s_name("namespace", namespace)?;
        lifecycle::validate_k8s_name("name", name)?;
        let mig_api: Api<VirtualMachineInstanceMigration> =
            Api::namespaced(self.client.clone(), namespace);
        let mig_name = format!(
            "{name}-migrate-{}",
            chrono::Utc::now().format("%Y%m%d%H%M%S")
        );
        let migration = VirtualMachineInstanceMigration {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(mig_name),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: VirtualMachineInstanceMigrationSpec {
                vmi_name: Some(name.to_string()),
            },
            status: None,
        };
        let pp = PostParams::default();
        Ok(mig_api.create(&pp, &migration).await?)
    }

    /// List migrations in a namespace, optionally filtered to one VM.
    pub async fn list_migrations(
        &self,
        namespace: &str,
        vm_name: Option<&str>,
    ) -> Result<Vec<VirtualMachineInstanceMigration>> {
        let mig_api: Api<VirtualMachineInstanceMigration> =
            Api::namespaced(self.client.clone(), namespace);
        let lp = ListParams::default();
        let list = mig_api.list(&lp).await?;
        Ok(list
            .items
            .into_iter()
            .filter(|m| match vm_name {
                Some(vm) => m.spec.vmi_name.as_deref() == Some(vm),
                None => true,
            })
            .collect())
    }

    /// Get one migration by name.
    pub async fn get_migration(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<VirtualMachineInstanceMigration> {
        let mig_api: Api<VirtualMachineInstanceMigration> =
            Api::namespaced(self.client.clone(), namespace);
        Ok(mig_api.get(id).await?)
    }

    /// Cancel a migration. KubeVirt aborts an in-progress migration when its
    /// `VirtualMachineInstanceMigration` object is deleted.
    pub async fn cancel_migration(&self, namespace: &str, id: &str) -> Result<()> {
        let mig_api: Api<VirtualMachineInstanceMigration> =
            Api::namespaced(self.client.clone(), namespace);
        let dp = DeleteParams::default();
        mig_api.delete(id, &dp).await?;
        Ok(())
    }

    /// Grow a PVC's requested storage size. Kubernetes/CSI only support
    /// expansion, never shrinking; callers should reject a smaller target
    /// before calling this (checked here too, defense in depth).
    pub async fn resize_pvc(
        &self,
        namespace: &str,
        pvc_name: &str,
        new_size: &str,
    ) -> Result<PersistentVolumeClaim> {
        let pvcs: Api<PersistentVolumeClaim> = self.pvc_api(namespace);
        let current = pvcs.get(pvc_name).await?;
        let current_size = current
            .spec
            .as_ref()
            .and_then(|s| s.resources.as_ref())
            .and_then(|r| r.requests.as_ref())
            .and_then(|req| req.get("storage"))
            .map(|q| q.0.clone());
        if let (Some(current_size), Some(current_bytes), Some(new_bytes)) = (
            current_size.clone(),
            current_size
                .as_deref()
                .and_then(crate::storage::parse_size_to_bytes),
            crate::storage::parse_size_to_bytes(new_size),
        ) {
            if new_bytes < current_bytes {
                anyhow::bail!(
                    "SHRINK_NOT_SUPPORTED: requested {new_size} is smaller than current {current_size}; PVCs can only grow"
                );
            }
        }

        let patch = json!({ "spec": { "resources": { "requests": { "storage": new_size } } } });
        let pp = PatchParams::default();
        Ok(pvcs.patch(pvc_name, &pp, &Patch::Merge(&patch)).await?)
    }

    /// Resize a VM's named disk by growing the PVC or DataVolume-backed PVC
    /// behind it. Blank (`emptyDisk`) and container disks have no PVC and
    /// can't be resized this way.
    pub async fn resize_disk(
        &self,
        namespace: &str,
        vm_name: &str,
        disk_name: &str,
        new_size: &str,
    ) -> Result<PersistentVolumeClaim> {
        let vm = self.get_vm(namespace, vm_name).await?;
        let volumes = vm
            .spec
            .template
            .spec
            .volumes
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VM '{vm_name}' has no volumes"))?;
        let pvc_name = resolve_disk_pvc_name(volumes, vm_name, disk_name)?;
        self.resize_pvc(namespace, &pvc_name, new_size).await
    }

    /// Apply a CDI DataVolume JSON manifest.
    pub async fn apply_data_volume(
        &self,
        namespace: &str,
        manifest: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        lifecycle::validate_k8s_name("namespace", namespace)?;
        let path = format!("/apis/cdi.kubevirt.io/v1beta1/namespaces/{namespace}/datavolumes");
        let body = serde_json::to_vec(manifest)?;
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(&path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(body)
            .map_err(|e| anyhow::anyhow!("failed to build DataVolume request: {e}"))?;
        Ok(self.client.request::<serde_json::Value>(req).await?)
    }

    /// Create-or-update a CDI DataSource — the stable alias a golden-image
    /// bundle points at its current versioned DataVolume. Unlike DataVolumes
    /// (each version immutable/uniquely named), the same DataSource name is
    /// re-applied on every re-import, so this upserts (create, then merge
    /// patch on 409) rather than failing on conflict.
    pub async fn apply_data_source(
        &self,
        namespace: &str,
        manifest: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        lifecycle::validate_k8s_name("namespace", namespace)?;
        let name = manifest["metadata"]["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("DataSource manifest missing metadata.name"))?;
        let collection =
            format!("/apis/cdi.kubevirt.io/v1beta1/namespaces/{namespace}/datasources");
        let body = serde_json::to_vec(manifest)?;
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(&collection)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(body)
            .map_err(|e| anyhow::anyhow!("failed to build DataSource request: {e}"))?;
        match self.client.request::<serde_json::Value>(req).await {
            Ok(v) => Ok(v),
            Err(kube::Error::Api(ae)) if ae.code == 409 => {
                let object_path = format!("{collection}/{name}");
                let patch_body = serde_json::to_vec(manifest)?;
                let patch_req = http::Request::builder()
                    .method(http::Method::PATCH)
                    .uri(&object_path)
                    .header(http::header::CONTENT_TYPE, "application/merge-patch+json")
                    .body(patch_body)
                    .map_err(|e| {
                        anyhow::anyhow!("failed to build DataSource patch request: {e}")
                    })?;
                Ok(self.client.request::<serde_json::Value>(patch_req).await?)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn get_data_volume(&self, namespace: &str, name: &str) -> Result<serde_json::Value> {
        lifecycle::validate_k8s_name("namespace", namespace)?;
        lifecycle::validate_k8s_name("name", name)?;
        let path =
            format!("/apis/cdi.kubevirt.io/v1beta1/namespaces/{namespace}/datavolumes/{name}");
        let req = http::Request::builder()
            .method(http::Method::GET)
            .uri(&path)
            .body(Vec::new())
            .map_err(|e| anyhow::anyhow!("failed to build DataVolume get: {e}"))?;
        Ok(self.client.request::<serde_json::Value>(req).await?)
    }

    /// Poll a DataVolume until Succeeded, Failed, or timeout.
    pub async fn wait_for_data_volume(
        &self,
        namespace: &str,
        name: &str,
        timeout_secs: u64,
    ) -> Result<cdi::DataVolumeWait> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));
        loop {
            match self.get_data_volume(namespace, name).await {
                Ok(obj) => {
                    let phase = cdi::data_volume_phase_from_object(&obj);
                    match cdi::classify_data_volume_phase(phase.as_deref()) {
                        cdi::DataVolumeWait::Pending if std::time::Instant::now() < deadline => {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                        other => return Ok(other),
                    }
                }
                Err(e) if std::time::Instant::now() < deadline => {
                    log::debug!("wait DataVolume {name}: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Poll a DataVolume until it finishes, reporting CDI's own progress
    /// through `on_progress` (0-100). Unlike `wait_for_data_volume`, a
    /// failure returns CDI's condition message, and transient API errors
    /// are retried until the deadline.
    pub async fn track_data_volume(
        &self,
        namespace: &str,
        name: &str,
        timeout_secs: u64,
        mut on_progress: impl FnMut(u8),
    ) -> std::result::Result<(), String> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));
        loop {
            match self.get_data_volume(namespace, name).await {
                Ok(obj) => {
                    let p = cdi::data_volume_progress_from_object(&obj);
                    match p.wait {
                        cdi::DataVolumeWait::Ready => return Ok(()),
                        cdi::DataVolumeWait::Failed => {
                            return Err(p
                                .failure_reason
                                .unwrap_or_else(|| "DataVolume failed".into()))
                        }
                        cdi::DataVolumeWait::Pending => {
                            if let Some(pct) = p.percent {
                                on_progress(pct);
                            }
                        }
                    }
                }
                Err(e) => log::debug!("track DataVolume {name}: {e}"),
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out after {timeout_secs}s waiting for DataVolume '{name}'"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    pub async fn get_vmi_subresource_json(
        &self,
        namespace: &str,
        name: &str,
        subresource: &str,
    ) -> Result<serde_json::Value> {
        let path = lifecycle::vmi_subresource_path(namespace, name, subresource)?;
        let req = http::Request::builder()
            .method(http::Method::GET)
            .uri(&path)
            .body(Vec::new())
            .map_err(|e| anyhow::anyhow!("failed to build {subresource} get: {e}"))?;
        Ok(self.client.request::<serde_json::Value>(req).await?)
    }

    pub async fn guest_ready_report(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<guest_ready::GuestReadyReport> {
        let status = match self.get_vmi(namespace, name).await {
            Ok(vmi) => vmi.status,
            Err(_) => None,
        };
        Ok(guest_ready::guest_ready_from_status(status.as_ref()))
    }

    pub async fn wait_until_guest_ready(
        &self,
        namespace: &str,
        name: &str,
        timeout_secs: u64,
    ) -> Result<guest_ready::GuestReadyReport> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));
        loop {
            let report = self.guest_ready_report(namespace, name).await?;
            if report.cloud_init_ready {
                return Ok(report);
            }
            if std::time::Instant::now() >= deadline {
                return Ok(report);
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

/// Find the PVC/DataVolume name backing a VM's named disk. Blank
/// (`emptyDisk`) and container disks have no PVC and are rejected.
fn resolve_disk_pvc_name(volumes: &[Volume], vm_name: &str, disk_name: &str) -> Result<String> {
    let volume = volumes
        .iter()
        .find(|v| v.name == disk_name)
        .ok_or_else(|| {
            anyhow::anyhow!("VOLUME_NOT_FOUND: disk '{disk_name}' not found on VM '{vm_name}'")
        })?;
    volume
        .persistent_volume_claim
        .as_ref()
        .map(|p| p.claim_name.clone())
        .or_else(|| volume.data_volume.as_ref().map(|d| d.name.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "UNSUPPORTED: disk '{disk_name}' is not backed by a PVC or DataVolume (blank/container disks can't be resized)"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_conversion() {
        use crate::config::VMConfigBuilder;

        let config = VMConfigBuilder::new("test")
            .namespace("default")
            .cpu(2, 1, 1)
            .memory("4Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_pod_network("default")
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();
        assert_eq!(vm.metadata.name, Some("test".to_string()));
    }

    #[test]
    fn test_vm_conversion_threads_hotplug_headroom() {
        use crate::config::VMConfigBuilder;

        let config = VMConfigBuilder::new("test")
            .namespace("default")
            .cpu(2, 1, 1)
            .cpu_max_sockets(4)
            .memory("4Gi")
            .max_guest_memory("16Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_pod_network("default")
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();
        let domain = &vm.spec.template.spec.domain;
        assert_eq!(domain.cpu.as_ref().and_then(|c| c.max_sockets), Some(4));
        assert_eq!(
            domain.memory.as_ref().and_then(|m| m.max_guest.clone()),
            Some("16Gi".to_string())
        );

        let json = serde_json::to_value(&vm).unwrap();
        assert_eq!(
            json["spec"]["template"]["spec"]["domain"]["cpu"]["maxSockets"],
            4
        );
        assert_eq!(
            json["spec"]["template"]["spec"]["domain"]["memory"]["maxGuest"],
            "16Gi"
        );
    }

    /// CPU hotplug's socket-count math: `new_sockets = ceil(target_vcpus / (cores*threads))`.
    fn hotplug_cpu_new_sockets(target_vcpus: u32, cores: u32, threads: u32) -> u32 {
        let per_socket = cores.saturating_mul(threads).max(1);
        (target_vcpus.saturating_add(per_socket - 1) / per_socket).max(1)
    }

    #[test]
    fn hotplug_cpu_socket_math() {
        assert_eq!(hotplug_cpu_new_sockets(4, 2, 1), 2);
        assert_eq!(hotplug_cpu_new_sockets(5, 2, 1), 3);
        assert_eq!(hotplug_cpu_new_sockets(1, 4, 1), 1);
        assert_eq!(hotplug_cpu_new_sockets(8, 2, 2), 2);
        assert_eq!(hotplug_cpu_new_sockets(9, 2, 2), 3);
        assert_eq!(hotplug_cpu_new_sockets(0, 2, 1), 1);
    }

    #[test]
    fn vmi_subresource_with_body_rejects_unknown_subresource() {
        // Path construction is shared with the empty-body variant and already
        // validated in kube::lifecycle's own tests; this confirms the
        // addvolume/addinterface family round-trips through the same guard.
        assert!(lifecycle::vmi_subresource_path("default", "vm", "addvolume").is_ok());
        assert!(lifecycle::vmi_subresource_path("default", "vm", "addinterface").is_ok());
        assert!(lifecycle::vmi_subresource_path("default", "vm", "removeinterface").is_ok());
        assert!(lifecycle::vmi_subresource_path("default", "vm", "bogus").is_err());
    }

    fn volume(name: &str) -> Volume {
        Volume {
            name: name.to_string(),
            container_disk: None,
            persistent_volume_claim: None,
            data_volume: None,
            cloud_init_no_cloud: None,
            empty_disk: None,
        }
    }

    #[test]
    fn resolves_pvc_backed_disk() {
        let mut vol = volume("rootdisk");
        vol.persistent_volume_claim = Some(PersistentVolumeClaimVolumeSource {
            claim_name: "my-pvc".into(),
        });
        let name = resolve_disk_pvc_name(&[vol], "vm1", "rootdisk").unwrap();
        assert_eq!(name, "my-pvc");
    }

    #[test]
    fn resolves_data_volume_backed_disk() {
        let mut vol = volume("rootdisk");
        vol.data_volume = Some(DataVolumeSource {
            name: "my-dv".into(),
        });
        let name = resolve_disk_pvc_name(&[vol], "vm1", "rootdisk").unwrap();
        assert_eq!(name, "my-dv");
    }

    #[test]
    fn rejects_missing_disk() {
        let vol = volume("rootdisk");
        assert!(resolve_disk_pvc_name(&[vol], "vm1", "other").is_err());
    }

    #[test]
    fn rejects_blank_disk() {
        let mut vol = volume("rootdisk");
        vol.empty_disk = Some(EmptyDiskSource {
            capacity: "20Gi".into(),
        });
        let err = resolve_disk_pvc_name(&[vol], "vm1", "rootdisk").unwrap_err();
        assert!(err.to_string().contains("UNSUPPORTED"));
    }
}
