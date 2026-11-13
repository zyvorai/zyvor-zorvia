//! The `vm-import` operation: run the h2kvm Job, follow its stage markers,
//! verify the VM exists, optionally confirm it boots, and stop its Job if it
//! fails or is cancelled. Imported VM disks are preserved for
//! inspection: names alone do not establish ownership for destructive rollback.

use super::job::{build_import_job, h2kvm_image, stub_image};
use super::{
    ensure_target_available, networks, parse_h2kvm_logs, pvc_name, H2kvmProgress, ImportOpParams,
};
use crate::backup::offcluster::{job_state, pod_config_error, JobState};
use crate::backup::restore::vm_api;
use crate::operations::runtime::{HandlerFuture, OpContext, Outcome};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{
    Api, DeleteParams, ListParams, LogParams, Patch, PatchParams, PostParams, Preconditions,
};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use serde_json::json;
use std::time::Duration;

pub fn run_import_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(async move {
        match import_op(&ctx).await {
            Ok(o) | Err(o) => o,
        }
    })
}

enum Followed {
    Done(H2kvmProgress),
    Failed(String),
    Cancelled,
}

fn deadline_secs() -> u64 {
    std::env::var("ZORVIA_IMPORT_JOB_DEADLINE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12 * 3600)
}

async fn import_op(ctx: &OpContext) -> Result<Outcome, Outcome> {
    let p: ImportOpParams = serde_json::from_value(ctx.op.params.clone())
        .map_err(|e| Outcome::Failed(format!("invalid operation params: {e}")))?;
    let image = if p.mode.is_lab_stub() {
        stub_image()
    } else {
        h2kvm_image().ok_or_else(|| {
            Outcome::Failed(
                "ZORVIA_H2KVM_IMAGE is not set: point it at your h2kvm CLI image (h2kvm is licensed \
             separately and Zorvia does not ship it)"
                    .into(),
            )
        })?
    };
    let client = ctx.client.client();
    let ns = p.namespace.as_str();
    let vms = vm_api(&client, ns);

    if !p.mode.is_lab_stub() || p.mode.is_local() {
        ensure_target_available(&client, ns, &p.target_vm_name)
            .await
            .map_err(|e| Outcome::Failed(e.to_string()))?;
    }
    if ctx.cancelled() {
        return Ok(Outcome::Cancelled);
    }

    let short: String = ctx
        .op
        .id
        .trim_start_matches("op-")
        .chars()
        .take(10)
        .collect();
    let job_name = format!("zorvia-import-{short}");
    ctx.progress(
        if p.mode.is_local() {
            "starting local import"
        } else if p.mode.is_lab_stub() {
            "starting stub import"
        } else {
            "starting h2kvm"
        },
        2,
    );
    let job =
        build_import_job(&job_name, &image, &p).map_err(|e| Outcome::Failed(e.to_string()))?;
    let jobs: Api<Job> = Api::namespaced(client.clone(), ns);
    let created = jobs
        .create(&PostParams::default(), &job)
        .await
        .map_err(|e| Outcome::Failed(format!("cannot create the import Job: {e}")))?;
    let job_uid = created.metadata.uid.as_deref().ok_or_else(|| {
        Outcome::Failed("created import Job has no UID; refusing unguarded cleanup".into())
    })?;

    match follow(ctx, &client, &p, &job_name).await {
        Followed::Done(prog) => {
            // Stub Jobs prove orchestration only — no VirtualMachine is created.
            if matches!(p.mode, super::ImportMode::Stub) {
                stop_job(&client, &p, &job_name, job_uid).await;
                return Ok(Outcome::Succeeded(Some(json!({
                    "vm_name": p.target_vm_name,
                    "namespace": p.namespace,
                    "source_vm": p.vm.source_vm,
                    "mode": "stub",
                    "stub": true,
                    "vm_created": false,
                    "stages_completed": prog.stages_done,
                    "deployed_marker": prog.deployed_vm,
                }))));
            }
            // Local lab mode: create a stopped emptyDisk VM after stage markers.
            if p.mode.is_local() {
                ctx.progress("creating local emptyDisk VM", 90);
                if let Err(e) = create_local_empty_vm(&client, &p).await {
                    stop_job(&client, &p, &job_name, job_uid).await;
                    return Ok(Outcome::Failed(format!(
                        "local import Job finished but VirtualMachine create failed: {e}"
                    )));
                }
            }
            // Trust the cluster, not the log: the VM must actually exist.
            let mut vm = match vms.get(&p.target_vm_name).await {
                Ok(vm) => vm,
                Err(e) => {
                    stop_job(&client, &p, &job_name, job_uid).await;
                    return Ok(Outcome::Failed(
                    format!("h2kvm finished but the VirtualMachine could not be verified: {e}; disks preserved"),
                ));
                }
            };
            if ctx.cancelled() {
                stop_job(&client, &p, &job_name, job_uid).await;
                return Ok(Outcome::Cancelled);
            }
            if !p.vm.networks.is_empty() {
                ctx.progress("configuring target networks", 95);
                match apply_networks(&client, ns, &p.target_vm_name, &vm, &p.vm.networks).await {
                    Ok(patched) => vm = patched,
                    Err(e) => {
                        stop_job(&client, &p, &job_name, job_uid).await;
                        return Ok(Outcome::Failed(format!(
                            "target network mapping failed: {e}; VM and disks preserved"
                        )));
                    }
                }
            }
            let mut result = json!({
                "vm_name": p.target_vm_name,
                "namespace": p.namespace,
                "source_vm": p.vm.source_vm,
                "pvc": pvc_name(&p.target_vm_name),
                "stages_completed": prog.stages_done,
                "networks": p.vm.networks,
                "cutover_requested": p.vm.start,
                "vm_created": true,
            });
            if p.mode.is_local() {
                result["mode"] = json!("local");
                result["stub"] = json!(false);
                result["disk"] = json!("emptyDisk");
                // emptyDisk has no PVC; keep the key for shape compatibility.
                result["pvc"] = json!(null);
            }
            if p.vm.start {
                if ctx.cancelled() {
                    stop_job(&client, &p, &job_name, job_uid).await;
                    return Ok(Outcome::Cancelled);
                }
                let patch = match serde_json::to_value(&vm)
                    .map_err(anyhow::Error::from)
                    .and_then(|value| cutover_patch(&value))
                {
                    Ok(patch) => patch,
                    Err(e) => {
                        stop_job(&client, &p, &job_name, job_uid).await;
                        return Ok(Outcome::Failed(format!(
                            "cutover blocked: {e}; VM and disks preserved"
                        )));
                    }
                };
                if let Err(e) = vms
                    .patch(
                        &p.target_vm_name,
                        &PatchParams::default(),
                        &Patch::Merge(&patch),
                    )
                    .await
                {
                    stop_job(&client, &p, &job_name, job_uid).await;
                    return Ok(Outcome::Failed(format!(
                        "cannot start imported VM: {e}; VM and disks preserved"
                    )));
                }
                ctx.progress("verifying boot", 96);
                match wait_for_boot(ctx, &p).await {
                    Ok(v) => result["validation"] = v,
                    Err(msg) => {
                        stop_job(&client, &p, &job_name, job_uid).await;
                        if ctx.cancelled() {
                            return Ok(Outcome::Cancelled);
                        }
                        // Leave the VM in place for inspection; the import
                        // itself worked but the cutover is not validated.
                        return Ok(Outcome::Failed(format!(
                            "VM '{}' was created but did not boot: {msg} (it was left in place)",
                            p.target_vm_name
                        )));
                    }
                }
            }
            stop_job(&client, &p, &job_name, job_uid).await;
            Ok(Outcome::Succeeded(Some(result)))
        }
        Followed::Failed(msg) => {
            stop_job(&client, &p, &job_name, job_uid).await;
            Ok(Outcome::Failed(format!(
                "{msg}; any imported VM and disks were preserved for inspection"
            )))
        }
        Followed::Cancelled => {
            stop_job(&client, &p, &job_name, job_uid).await;
            Ok(Outcome::Cancelled)
        }
    }
}

/// Create a halted emptyDisk VirtualMachine for `mode=local` lab imports.
async fn create_local_empty_vm(client: &kube::Client, p: &ImportOpParams) -> anyhow::Result<()> {
    let cpu = p.vm.cpu.unwrap_or(1).max(1);
    let memory = p.vm.memory.clone().unwrap_or_else(|| "128Mi".into());
    let disk = p.vm.pvc_size.clone().unwrap_or_else(|| "1Gi".into());
    let body = json!({
        "apiVersion": "kubevirt.io/v1",
        "kind": "VirtualMachine",
        "metadata": {
            "name": p.target_vm_name,
            "namespace": p.namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "zorvia",
                "zorvia.io/import-mode": "local",
                "zorvia.io/wave": p.wave_id,
            }
        },
        "spec": {
            "running": false,
            "template": {
                "metadata": {
                    "labels": {
                        "kubevirt.io/domain": p.target_vm_name,
                        "zorvia.io/import-mode": "local",
                    }
                },
                "spec": {
                    "domain": {
                        "cpu": { "cores": cpu },
                        "resources": { "requests": { "memory": memory } },
                        "devices": {
                            "disks": [{ "name": "rootdisk", "disk": { "bus": "virtio" } }]
                        }
                    },
                    "volumes": [{
                        "name": "rootdisk",
                        "emptyDisk": { "capacity": disk }
                    }]
                }
            }
        }
    });
    let vm: DynamicObject = serde_json::from_value(body)
        .map_err(|e| anyhow::anyhow!("invalid local VM body: {e}"))?;
    let vms = vm_api(client, &p.namespace);
    vms.create(&PostParams::default(), &vm)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

async fn follow(
    ctx: &OpContext,
    client: &kube::Client,
    p: &ImportOpParams,
    job_name: &str,
) -> Followed {
    let ns = p.namespace.as_str();
    let jobs: Api<Job> = Api::namespaced(client.clone(), ns);
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let started = std::time::Instant::now();
    let give_up = Duration::from_secs(deadline_secs() + 300);
    loop {
        if ctx.cancelled() {
            return Followed::Cancelled;
        }
        let pod_list = pods
            .list(&ListParams::default().labels(&format!("job-name={job_name}")))
            .await
            .map(|l| l.items)
            .unwrap_or_default();
        if let Some(msg) = pod_list.iter().find_map(pod_config_error) {
            return Followed::Failed(msg);
        }
        let pod_name = pod_list.first().and_then(|p| p.metadata.name.clone());
        let mut logs = String::new();
        if let Some(name) = &pod_name {
            let lp = LogParams {
                container: Some("h2kvm".into()),
                tail_lines: Some(400),
                ..Default::default()
            };
            logs = pods.logs(name, &lp).await.unwrap_or_default();
        }
        let prog = parse_h2kvm_logs(&logs);
        ctx.progress(
            prog.current_stage.as_deref().unwrap_or("starting"),
            prog.percent().max(2),
        );

        match jobs.get(job_name).await.map(|j| job_state(&j)) {
            Ok(JobState::Succeeded) => return Followed::Done(prog),
            Ok(JobState::Failed(why)) => {
                let mut msg = prog.failure.clone().unwrap_or(why);
                if prog.stages_started == 0 {
                    // Never got to h2kvm: the nbd loader init container is the
                    // usual culprit, so include what it said.
                    if let Some(name) = &pod_name {
                        let lp = LogParams {
                            container: Some("nbd-loader".into()),
                            tail_lines: Some(20),
                            ..Default::default()
                        };
                        if let Ok(l) = pods.logs(name, &lp).await {
                            let l = l.trim();
                            if !l.is_empty() {
                                msg = format!(
                                    "{msg}; nbd-loader: {}",
                                    l.chars().take(300).collect::<String>()
                                );
                            }
                        }
                    }
                }
                return Followed::Failed(msg);
            }
            Ok(JobState::Running) | Err(_) => {}
        }
        if started.elapsed() >= give_up {
            return Followed::Failed("timed out waiting for the import Job".into());
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

fn boot_ready(running: bool, agent: bool, require_agent: bool) -> bool {
    running && (!require_agent || agent)
}

fn cutover_patch(vm: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let version = vm["metadata"]["resourceVersion"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("VM has no resourceVersion"))?;
    if vm["spec"]["running"].as_bool() == Some(true)
        || vm["spec"]["runStrategy"]
            .as_str()
            .is_some_and(|s| s != "Halted")
        || vm["status"]["created"].as_bool() == Some(true)
    {
        anyhow::bail!("imported VM is already active; refusing automatic cutover");
    }
    Ok(json!({"metadata":{"resourceVersion":version},"spec":{"runStrategy":null,"running":true}}))
}

async fn apply_networks(
    client: &kube::Client,
    ns: &str,
    name: &str,
    vm: &DynamicObject,
    mappings: &[networks::ImportNetwork],
) -> anyhow::Result<DynamicObject> {
    let vmi_ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "kubevirt.io",
        "v1",
        "VirtualMachineInstance",
    ));
    let vmis: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &vmi_ar);
    match vmis.get(name).await {
        Ok(_) => anyhow::bail!("VM has a VirtualMachineInstance; stop it before network mapping"),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(e.into()),
    }
    let attachments = networks::attachment_api(client, ns);
    for mapping in mappings {
        if let Some(name) = &mapping.attachment {
            let nad = attachments.get(name).await?;
            networks::validate_attachment(&serde_json::to_value(&nad)?)?;
        }
    }
    let patch = networks::network_patch(&serde_json::to_value(vm)?, mappings)?;
    Ok(vm_api(client, ns)
        .patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?)
}

async fn wait_for_boot(ctx: &OpContext, p: &ImportOpParams) -> Result<serde_json::Value, String> {
    let timeout = Duration::from_secs(
        p.vm.boot_timeout_secs
            .or_else(|| {
                std::env::var("ZORVIA_IMPORT_BOOT_TIMEOUT_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .filter(|t| (30..=3600).contains(t))
            .unwrap_or(600),
    );
    let t0 = std::time::Instant::now();
    let mut last = String::from("no VirtualMachineInstance");
    loop {
        if ctx.cancelled() {
            return Err("boot verification cancelled; VM preserved".into());
        }
        if let Ok(r) = ctx
            .client
            .guest_ready_report(&p.namespace, &p.target_vm_name)
            .await
        {
            last = r.reason.clone();
            if boot_ready(r.running, r.agent_connected, p.vm.require_guest_agent) {
                // Guests migrated from VMware often have no qemu-guest-agent,
                // so it is reported, not required.
                return Ok(json!({
                    "running": true,
                    "guest_agent": r.agent_connected,
                    "boot_seconds": t0.elapsed().as_secs(),
                    "require_guest_agent": p.vm.require_guest_agent,
                }));
            }
        }
        if t0.elapsed() >= timeout {
            return Err(format!(
                "boot policy not satisfied after {}s ({last}); require_guest_agent={}",
                timeout.as_secs(),
                p.vm.require_guest_agent
            ));
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Only remove the exact Job we created. VM/PVC names are not proof of ownership
/// (another actor could create them after preflight), so data cleanup is manual.
async fn stop_job(client: &kube::Client, p: &ImportOpParams, job_name: &str, uid: &str) {
    let ns = p.namespace.as_str();
    let jobs: Api<Job> = Api::namespaced(client.clone(), ns);
    let mut dp = DeleteParams::foreground();
    dp.preconditions = Some(Preconditions {
        uid: Some(uid.into()),
        resource_version: None,
    });
    match jobs.delete(job_name, &dp).await {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => {
            log::error!("import cleanup: cannot delete Job {ns}/{job_name} with UID {uid}: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_running_and_optional_agent() {
        assert!(!boot_ready(false, true, false));
        assert!(boot_ready(true, false, false));
        assert!(!boot_ready(true, false, true));
        assert!(boot_ready(true, true, true));
    }

    #[test]
    fn cutover_is_version_guarded_and_removes_conflicting_run_strategy() {
        let patch = cutover_patch(
            &json!({"metadata":{"resourceVersion":"7"},"spec":{"runStrategy":"Halted"}}),
        )
        .unwrap();
        assert_eq!(patch["metadata"]["resourceVersion"], "7");
        assert!(patch["spec"]["runStrategy"].is_null());
        assert_eq!(patch["spec"]["running"], true);
        assert!(cutover_patch(
            &json!({"metadata":{"resourceVersion":"7"},"spec":{"running":true}})
        )
        .is_err());
        assert!(cutover_patch(&json!({})).is_err());
    }

    #[tokio::test]
    async fn maps_networks_only_after_vmi_and_attachment_checks() {
        use super::super::test_api;
        use axum::http::StatusCode;
        let vm = json!({"apiVersion":"kubevirt.io/v1","kind":"VirtualMachine","metadata":{"name":"web-01","resourceVersion":"7"},"spec":{"running":false}});
        let api = test_api::mock(vec![test_api::missing(),
            (StatusCode::OK, json!({"apiVersion":"k8s.cni.cncf.io/v1","kind":"NetworkAttachmentDefinition","metadata":{"name":"prod-vlan"},"spec":{"config":r#"{"type":"bridge"}"#}})),
            (StatusCode::OK, vm.clone()),
        ]).await;
        let mappings = vec![networks::ImportNetwork {
            name: "prod".into(),
            attachment: Some("prod-vlan".into()),
            source_network: None,
            mac_address: None,
        }];
        apply_networks(
            &api.client,
            "vms",
            "web-01",
            &serde_json::from_value(vm).unwrap(),
            &mappings,
        )
        .await
        .unwrap();
        let requests = api.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].1.ends_with("/virtualmachineinstances/web-01"));
        assert!(requests[1]
            .1
            .ends_with("/network-attachment-definitions/prod-vlan"));
        assert_eq!(requests[2].0, "PATCH");
        assert_eq!(requests[2].2["metadata"]["resourceVersion"], "7");
    }

    #[tokio::test]
    async fn live_vmi_blocks_network_mapping_without_patching() {
        use super::super::test_api;
        let api = test_api::mock(vec![(axum::http::StatusCode::OK, json!({"apiVersion":"kubevirt.io/v1","kind":"VirtualMachineInstance","metadata":{"name":"web-01"}}))]).await;
        let vm = serde_json::from_value(json!({"apiVersion":"kubevirt.io/v1","kind":"VirtualMachine","metadata":{"resourceVersion":"7"}})).unwrap();
        let mappings = vec![networks::ImportNetwork {
            name: "prod".into(),
            attachment: Some("prod-vlan".into()),
            source_network: None,
            mac_address: None,
        }];
        assert!(apply_networks(&api.client, "vms", "web-01", &vm, &mappings)
            .await
            .is_err());
        assert_eq!(api.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cleanup_deletes_only_the_exact_job_uid_and_never_disks() {
        use super::super::test_api;
        let api = test_api::mock(vec![(
            axum::http::StatusCode::OK,
            json!({"kind":"Status","apiVersion":"v1","status":"Success","code":200}),
        )])
        .await;
        let p: ImportOpParams = serde_json::from_value(json!({
            "wave_id":"wave-1", "source":{"vcenter":"vc.example.com","secret_name":"vc-creds"},
            "namespace":"vms", "vm":{"source_vm":"web-01"}, "target_vm_name":"web-01",
            "scratch_size":"200Gi", "scratch_storage_class":null, "service_account":"zorvia-h2kvm",
        }))
        .unwrap();
        stop_job(&api.client, &p, "import-job", "original-uid").await;
        let requests = api.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "DELETE");
        assert!(requests[0].1.ends_with("/jobs/import-job"));
        assert_eq!(requests[0].2["preconditions"]["uid"], "original-uid");
        assert_eq!(requests[0].2["propagationPolicy"], "Foreground");
    }
}
