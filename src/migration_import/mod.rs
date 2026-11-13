//! VMware -> KubeVirt import driven from Zorvia by running `h2kvmctl` as a
//! Kubernetes Job (h2kvm is an external, separately licensed tool: Zorvia only
//! invokes its container image, it never links it).
//!
//! One `vm-import` operation per source VM; a *wave* is a batch of them queued
//! together and run under a concurrency cap. `h2kvmctl --cmd vsphere ...
//! --deploy-k8s` does the whole pipeline (export from vCenter, GuestKit
//! offline repair, convert, upload via CDI, create the VirtualMachine); Zorvia
//! supplies parameters, credentials (from a Secret), scratch space, follows the
//! stage markers h2kvm logs, verifies the VM exists, applies target networking,
//! and optionally starts it. Failed imports preserve disks for inspection.

pub mod job;
pub mod networks;
pub mod runtime;

use crate::operations::{NewOperation, Operation, OperationsDb};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const OP_KIND: &str = "vm-import";
pub const MAX_WAVE: usize = 50;

/// Fail closed when checking names. Only an explicit Kubernetes 404 proves
/// absence; permissions and connectivity failures must never permit an import.
pub async fn ensure_target_available(
    client: &kube::Client,
    namespace: &str,
    target: &str,
) -> Result<()> {
    use kube::api::Api;
    use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
    for (group, version, kind, name) in [
        ("kubevirt.io", "v1", "VirtualMachine", target.to_string()),
        ("", "v1", "PersistentVolumeClaim", pvc_name(target)),
        ("cdi.kubevirt.io", "v1beta1", "DataVolume", pvc_name(target)),
    ] {
        let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(group, version, kind));
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
        match api.get(&name).await {
            Ok(_) => bail!("{kind} '{name}' already exists in '{namespace}'"),
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(e) => {
                bail!("cannot establish whether {kind} '{name}' exists in '{namespace}': {e}")
            }
        }
    }
    Ok(())
}

/// Import orchestration mode. `vsphere` is production (h2kvmctl against
/// vCenter). `stub` / `local` exercise Job create/follow/cancel on a lab
/// without vCenter — the Job emits h2kvm-shaped stage logs and does not
/// need a licensed h2kvm image.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    #[default]
    Vsphere,
    /// Lab-only: busybox (or `ZORVIA_H2KVM_IMAGE`) prints stage markers; no VM.
    Stub,
    /// Lab-only: same Job as stub, then creates a stopped emptyDisk VirtualMachine.
    Local,
}

impl ImportMode {
    /// Stub or local: unprivileged Job, no vCenter / h2kvm image required.
    pub fn is_lab_stub(self) -> bool {
        matches!(self, Self::Stub | Self::Local)
    }

    pub fn is_local(self) -> bool {
        matches!(self, Self::Local)
    }
}

/// Where the source VMs live. The password never appears here: it is read by
/// the Job from `secret_name` in the target namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VsphereSource {
    pub vcenter: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub datacenter: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    /// Secret (in the target namespace) holding the vCenter credentials.
    pub secret_name: String,
    #[serde(default)]
    pub username_key: Option<String>,
    #[serde(default)]
    pub password_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportVm {
    /// VM name as vCenter knows it.
    pub source_vm: String,
    /// Defaults to a DNS-label form of `source_vm`.
    #[serde(default)]
    pub target_vm_name: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    /// Size of the root PVC, e.g. "60Gi" (h2kvm detects it when omitted).
    #[serde(default)]
    pub pvc_size: Option<String>,
    #[serde(default)]
    pub cpu: Option<u32>,
    /// e.g. "8Gi".
    #[serde(default)]
    pub memory: Option<String>,
    /// Start the VM once deployed (cutover). Default: leave it stopped.
    #[serde(default)]
    pub start: bool,
    /// Explicit target NIC layout, applied before cutover. Empty keeps the
    /// importer's default networking. Attachments are in the target namespace.
    #[serde(default)]
    pub networks: Vec<networks::ImportNetwork>,
    /// Require a connected guest agent in addition to a Running VMI.
    #[serde(default)]
    pub require_guest_agent: bool,
    /// Override the boot verification timeout (30..=3600 seconds).
    #[serde(default)]
    pub boot_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportRequest {
    pub source: VsphereSource,
    /// Namespace the VMs are created in (and the Job runs in).
    pub namespace: String,
    pub vms: Vec<ImportVm>,
    /// Scratch space for the exported disk and the converted image. Must fit
    /// the largest disk twice over. Default 200Gi.
    #[serde(default)]
    pub scratch_size: Option<String>,
    #[serde(default)]
    pub scratch_storage_class: Option<String>,
    /// ServiceAccount the Job runs as (needs rights to create VMs, PVCs and
    /// DataVolumes in the namespace). Default `zorvia-h2kvm`.
    #[serde(default)]
    pub service_account: Option<String>,
    /// `vsphere` (default), `stub`, or `local`. Stub/local skip vCenter.
    #[serde(default)]
    pub mode: ImportMode,
}

/// What one operation persists. No secret values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOpParams {
    pub wave_id: String,
    pub source: VsphereSource,
    pub namespace: String,
    pub vm: ImportVm,
    pub target_vm_name: String,
    pub scratch_size: String,
    pub scratch_storage_class: Option<String>,
    pub service_account: String,
    #[serde(default)]
    pub mode: ImportMode,
}

pub fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// A value that becomes one argv element of `h2kvmctl`: not a shell string,
/// but must not start with `-` (option injection) or contain control chars.
fn safe_arg(what: &str, s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 256 {
        bail!("{what} must be 1-256 characters");
    }
    if s.starts_with('-') {
        bail!("{what} must not start with '-'");
    }
    if s.chars().any(|c| c.is_control()) {
        bail!("{what} contains control characters");
    }
    if s.contains("$(") {
        bail!("{what} must not contain Kubernetes environment expansion");
    }
    Ok(())
}

fn quantity(what: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty() && s.len() <= 16 && {
        let digits: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let suffix = &s[digits.len()..];
        !digits.is_empty()
            && !digits.starts_with('.')
            && !digits.ends_with('.')
            && digits.bytes().any(|b| matches!(b, b'1'..=b'9'))
            && digits.matches('.').count() <= 1
            && matches!(
                suffix,
                "" | "Ki" | "Mi" | "Gi" | "Ti" | "K" | "M" | "G" | "T"
            )
    };
    if !ok {
        bail!("{what} must be a Kubernetes quantity like 60Gi");
    }
    Ok(())
}

fn hostname(s: &str) -> bool {
    let address = s
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(s);
    if address.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// DNS-label form of a vCenter VM name ("Web Prod_01" -> "web-prod-01").
pub fn default_target_name(source_vm: &str) -> String {
    let mut out: String = source_vm
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let trimmed = out.trim_matches('-');
    let mut name: String = trimmed.chars().take(63).collect();
    name = name.trim_end_matches('-').to_string();
    if name.is_empty() {
        "imported-vm".to_string()
    } else {
        name
    }
}

fn note(p: &mut Vec<String>, r: Result<()>) {
    if let Err(e) = r {
        p.push(e.to_string());
    }
}

impl ImportRequest {
    /// Every problem with the request (empty when valid).
    pub fn problems(&self) -> Vec<String> {
        let mut p: Vec<String> = Vec::new();
        if self.mode.is_lab_stub() {
            if !self.source.vcenter.is_empty() && !hostname(&self.source.vcenter) {
                p.push("source.vcenter is not a valid host name or address".into());
            }
            if !self.source.secret_name.is_empty() && !dns_label(&self.source.secret_name) {
                p.push("source.secret_name must be a DNS label".into());
            }
        } else {
            if !hostname(&self.source.vcenter) {
                p.push("source.vcenter is not a valid host name or address".into());
            }
            if self.source.port == Some(0) {
                p.push("source.port must be 1-65535".into());
            }
            if !dns_label(&self.source.secret_name) {
                p.push("source.secret_name must be a DNS label".into());
            }
            for (k, v) in [
                ("source.username_key", &self.source.username_key),
                ("source.password_key", &self.source.password_key),
                ("source.datacenter", &self.source.datacenter),
            ] {
                if let Some(v) = v {
                    note(&mut p, safe_arg(k, v));
                }
            }
        }
        if !dns_label(&self.namespace) {
            p.push("namespace must be a DNS label".into());
        }
        if let Some(sa) = &self.service_account {
            if !dns_label(sa) {
                p.push("service_account must be a DNS label".into());
            }
        }
        if let Some(s) = &self.scratch_size {
            note(&mut p, quantity("scratch_size", s));
        }
        if let Some(sc) = &self.scratch_storage_class {
            note(&mut p, safe_arg("scratch_storage_class", sc));
        }
        if self.vms.is_empty() {
            p.push("vms is empty".into());
        }
        if self.vms.len() > MAX_WAVE {
            p.push(format!("a wave holds at most {MAX_WAVE} VMs"));
        }
        let mut names = std::collections::HashSet::new();
        for (i, vm) in self.vms.iter().enumerate() {
            let at = format!("vms[{i}]");
            note(&mut p, safe_arg(&format!("{at}.source_vm"), &vm.source_vm));
            let target = vm
                .target_vm_name
                .clone()
                .unwrap_or_else(|| default_target_name(&vm.source_vm));
            if !dns_label(&target) {
                p.push(format!("{at}.target_vm_name must be a DNS label"));
            } else if !names.insert(target.clone()) {
                p.push(format!(
                    "{at}: target name '{target}' is used twice in this wave"
                ));
            }
            if let Some(sc) = &vm.storage_class {
                note(&mut p, safe_arg(&format!("{at}.storage_class"), sc));
            }
            if let Some(s) = &vm.pvc_size {
                note(&mut p, quantity(&format!("{at}.pvc_size"), s));
            }
            if let Some(m) = &vm.memory {
                note(&mut p, quantity(&format!("{at}.memory"), m));
            }
            if let Some(c) = vm.cpu {
                if c == 0 || c > 256 {
                    p.push(format!("{at}.cpu must be 1-256"));
                }
            }
            p.extend(
                networks::problems(&vm.networks)
                    .into_iter()
                    .map(|e| format!("{at}: {e}")),
            );
            if vm.require_guest_agent && !vm.start {
                p.push(format!("{at}.require_guest_agent requires start=true"));
            }
            if vm
                .boot_timeout_secs
                .is_some_and(|t| !(30..=3600).contains(&t))
            {
                p.push(format!("{at}.boot_timeout_secs must be 30-3600"));
            }
        }
        p
    }

    pub fn validate(&self) -> Result<()> {
        let p = self.problems();
        if p.is_empty() {
            Ok(())
        } else {
            bail!("{}", p.join("; "))
        }
    }
}

/// argv for `h2kvmctl` (after the binary). `$(VC_USER)` is expanded by
/// Kubernetes from the Job's env, so the credentials never enter the spec.
/// Not used for stub/local lab Jobs (see `job::build_import_job`).
pub fn h2kvm_args(p: &ImportOpParams) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--cmd".into(),
        "vsphere".into(),
        "--vcenter".into(),
        p.source.vcenter.clone(),
        "--vc-user".into(),
        "$(VC_USER)".into(),
        "--vc-password-env".into(),
        "VC_PASSWORD".into(),
        "--vs-vm".into(),
        p.vm.source_vm.clone(),
        "--output-dir".into(),
        "/work/out".into(),
        "--to-output".into(),
        format!("{}.qcow2", p.target_vm_name),
        "--flatten".into(),
        "--deploy-k8s".into(),
        "--k8s-namespace".into(),
        p.namespace.clone(),
        "--k8s-vm-name".into(),
        p.target_vm_name.clone(),
        "--k8s-pvc-name".into(),
        pvc_name(&p.target_vm_name),
    ];
    if let Some(port) = p.source.port {
        a.extend(["--vc-port".into(), port.to_string()]);
    }
    if p.source.insecure {
        a.push("--vc-insecure".into());
    }
    if let Some(dc) = &p.source.datacenter {
        a.extend(["--dc-name".into(), dc.clone()]);
    }
    if let Some(sc) = &p.vm.storage_class {
        a.extend(["--k8s-storage-class".into(), sc.clone()]);
    }
    if let Some(sz) = &p.vm.pvc_size {
        a.extend(["--k8s-pvc-size".into(), sz.clone()]);
    }
    if let Some(cpu) = p.vm.cpu {
        a.extend(["--k8s-cpu".into(), cpu.to_string()]);
    }
    if let Some(mem) = &p.vm.memory {
        a.extend(["--k8s-memory".into(), mem.clone()]);
    }
    // Cutover is owned by Zorvia after networking and cancellation checks.
    // Never let the importer start a guest before target NICs are applied.
    a
}

/// Root PVC name for an imported VM (deterministic, so rollback can find it).
pub fn pvc_name(target_vm: &str) -> String {
    let mut n = format!("{target_vm}-root");
    n.truncate(63);
    n
}

/// What we can tell from h2kvm's log output.
#[derive(Debug, Default, PartialEq)]
pub struct H2kvmProgress {
    pub current_stage: Option<String>,
    pub stages_started: u32,
    pub stages_done: u32,
    pub deployed_vm: Option<String>,
    pub failure: Option<String>,
}

impl H2kvmProgress {
    /// Rough percentage: h2kvm reports stage boundaries, not bytes. Capped at
    /// 95 until the VM is confirmed deployed.
    pub fn percent(&self) -> u8 {
        if self.deployed_vm.is_some() {
            return 95;
        }
        // Typical run: inspect, export/flatten, fix, convert, validate,
        // upload, deploy... treat 8 stages as the expected total.
        (self.stages_done * 100 / 8).min(90) as u8
    }
}

/// Parse h2kvm log lines (`➡️  Stage: X`, `✅ x completed in Ns`,
/// `✅ Deployed: <vm>`, `❌ x failed: reason`).
pub fn parse_h2kvm_logs(logs: &str) -> H2kvmProgress {
    let mut p = H2kvmProgress::default();
    for raw in logs.lines() {
        let line = raw.trim();
        if let Some(rest) = line.split("Stage:").nth(1).filter(|_| line.contains('➡')) {
            p.stages_started += 1;
            p.current_stage = Some(rest.trim().to_ascii_lowercase());
        } else if line.contains('✅') && line.contains("completed in") {
            p.stages_done += 1;
        } else if let Some(rest) = line
            .split("Deployed:")
            .nth(1)
            .filter(|_| line.contains('✅'))
        {
            let vm = rest.trim();
            if !vm.is_empty() {
                p.deployed_vm = Some(vm.to_string());
            }
        } else if line.contains('❌') {
            let msg: String = line.chars().filter(|c| *c != '❌').collect();
            p.failure = Some(msg.trim().chars().take(500).collect());
        }
    }
    p
}

fn short(id: &str) -> String {
    id.trim_start_matches("op-").chars().take(10).collect()
}

/// Queue one `vm-import` operation per VM under a new wave id.
pub fn enqueue_wave_in(db: &OperationsDb, req: &ImportRequest) -> Result<(String, Vec<Operation>)> {
    req.validate()?;
    let wave_id = format!("wave-{}", short(&uuid::Uuid::new_v4().simple().to_string()));
    let active = db.list_active()?;
    let mut ops = Vec::new();
    for vm in &req.vms {
        let target = vm
            .target_vm_name
            .clone()
            .unwrap_or_else(|| default_target_name(&vm.source_vm));
        if active.iter().any(|o| {
            o.kind == OP_KIND
                && o.namespace == req.namespace
                && o.params.get("target_vm_name").and_then(|t| t.as_str()) == Some(target.as_str())
        }) {
            bail!("an import to '{target}' is already in progress");
        }
        let params = ImportOpParams {
            wave_id: wave_id.clone(),
            source: req.source.clone(),
            namespace: req.namespace.clone(),
            vm: vm.clone(),
            target_vm_name: target.clone(),
            scratch_size: req.scratch_size.clone().unwrap_or_else(|| "200Gi".into()),
            scratch_storage_class: req.scratch_storage_class.clone(),
            service_account: req.service_account.clone().unwrap_or_else(|| {
                if req.mode.is_lab_stub() {
                    "default".into()
                } else {
                    "zorvia-h2kvm".into()
                }
            }),
            mode: req.mode,
        };
        let mut new = NewOperation::new(OP_KIND, &vm.source_vm, &req.namespace);
        new.max_attempts = 1; // an import is not safely repeatable; the user retries
        new.params = serde_json::to_value(&params)?;
        ops.push(db.create(new)?.0);
    }
    Ok((wave_id, ops))
}

/// Counts by state for every operation of a wave.
pub fn wave_summary(db: &OperationsDb, wave_id: &str) -> Result<serde_json::Value> {
    let ops: Vec<Operation> = db
        .list(Some(OP_KIND), 500)?
        .into_iter()
        .filter(|o| o.params.get("wave_id").and_then(|w| w.as_str()) == Some(wave_id))
        .collect();
    let count = |s: crate::operations::OpState| ops.iter().filter(|o| o.state == s).count();
    use crate::operations::OpState::*;
    Ok(serde_json::json!({
        "wave_id": wave_id,
        "total": ops.len(),
        "queued": count(Queued),
        "running": count(Running),
        "succeeded": count(Succeeded),
        "failed": count(Failed),
        "cancelled": count(Cancelled),
        "operations": ops.iter().map(|o| serde_json::json!({
            "id": o.id,
            "source_vm": o.resource,
            "target_vm_name": o.params.get("target_vm_name"),
            "state": o.state,
            "phase": o.phase,
            "progress": o.progress,
            "error": o.error,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
pub(crate) mod test_api {
    use axum::{
        body::{to_bytes, Body},
        extract::{Request, State},
        http::StatusCode,
        response::IntoResponse,
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    pub struct MockApi {
        pub client: kube::Client,
        pub requests: Arc<Mutex<Vec<(String, String, Value)>>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for MockApi {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    pub fn missing() -> (StatusCode, Value) {
        (
            StatusCode::NOT_FOUND,
            json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"missing","code":404}),
        )
    }

    pub async fn mock(replies: Vec<(StatusCode, Value)>) -> MockApi {
        type Replies = Arc<Mutex<VecDeque<(StatusCode, Value)>>>;
        type Requests = Arc<Mutex<Vec<(String, String, Value)>>>;
        async fn handle(
            State((replies, requests)): State<(Replies, Requests)>,
            req: Request<Body>,
        ) -> impl IntoResponse {
            let method = req.method().to_string();
            let path = req.uri().path().to_string();
            let body = to_bytes(req.into_body(), 1024 * 1024).await.unwrap();
            let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            requests.lock().unwrap().push((method, path, value));
            let (status, value) = replies.lock().unwrap().pop_front().unwrap_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"message":"unexpected request","code":500}),
            ));
            (status, Json(value))
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(handle).with_state((
            Arc::new(Mutex::new(VecDeque::from(replies))),
            requests.clone(),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        MockApi {
            client: kube::Client::try_from(kube::Config::new(uri)).unwrap(),
            requests,
            task,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ImportRequest {
        ImportRequest {
            source: VsphereSource {
                vcenter: "vc.example.com".into(),
                port: None,
                datacenter: Some("DC1".into()),
                insecure: false,
                secret_name: "vcenter-creds".into(),
                username_key: None,
                password_key: None,
            },
            namespace: "vms".into(),
            vms: vec![ImportVm {
                source_vm: "Web Prod_01".into(),
                target_vm_name: None,
                storage_class: Some("longhorn".into()),
                pvc_size: Some("60Gi".into()),
                cpu: Some(4),
                memory: Some("8Gi".into()),
                start: true,
                networks: Vec::new(),
                require_guest_agent: false,
                boot_timeout_secs: None,
            }],
            scratch_size: None,
            scratch_storage_class: None,
            service_account: None,
            mode: ImportMode::Vsphere,
        }
    }

    #[test]
    fn stub_mode_skips_vcenter_and_secret() {
        let mut r = req();
        r.mode = ImportMode::Stub;
        r.source.vcenter.clear();
        r.source.secret_name.clear();
        assert!(r.problems().is_empty());
    }

    #[test]
    fn valid_request_passes_and_names_are_derived() {
        assert!(req().problems().is_empty());
        assert_eq!(default_target_name("Web Prod_01"), "web-prod-01");
        assert_eq!(default_target_name("--Bad//Name--"), "bad-name");
        assert_eq!(default_target_name("***"), "imported-vm");
        assert!(dns_label(&default_target_name(&"x".repeat(200))));
    }

    #[test]
    fn rejects_option_injection_and_bad_values() {
        let mut r = req();
        r.vms[0].source_vm = "--vc-insecure".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.source.vcenter = "vc.example.com --deploy-openstack".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.vms[0].pvc_size = Some("60GB; rm".into());
        assert!(r.validate().is_err());
        let mut r = req();
        r.vms[0].memory = Some("-1Gi".into());
        assert!(r.validate().is_err());
        let mut r = req();
        r.vms[0].source_vm = "vm\nname".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.vms[0].cpu = Some(0);
        assert!(r.validate().is_err());
        let mut r = req();
        r.namespace = "Bad_NS".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.vms.clear();
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_targets_within_a_wave() {
        let mut r = req();
        let mut second = r.vms[0].clone();
        second.source_vm = "web prod 01".into(); // same derived name
        r.vms.push(second);
        assert!(r.problems().iter().any(|p| p.contains("used twice")));
    }

    #[test]
    fn rejects_zero_malformed_quantities_and_env_expansion() {
        for value in ["0", "0Gi", ".Gi", "1.Gi", ".1Gi", "00.00Gi"] {
            let mut r = req();
            r.vms[0].memory = Some(value.into());
            assert!(r.validate().is_err(), "{value}");
        }
        for value in ["1", "0.5Gi", "16Gi"] {
            let mut r = req();
            r.vms[0].memory = Some(value.into());
            assert!(r.validate().is_ok(), "{value}");
        }
        let mut r = req();
        r.vms[0].source_vm = "$(VC_PASSWORD)".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.source.port = Some(0);
        assert!(r.validate().is_err());
        let mut r = req();
        r.scratch_storage_class = Some("--bad".into());
        assert!(r.validate().is_err());
    }

    #[test]
    fn validates_boot_policy_and_preserves_legacy_json() {
        let mut r = req();
        r.vms[0].require_guest_agent = true;
        r.vms[0].start = false;
        assert!(r.validate().is_err());
        r.vms[0].start = true;
        r.vms[0].boot_timeout_secs = Some(29);
        assert!(r.validate().is_err());
        r.vms[0].boot_timeout_secs = Some(3600);
        assert!(r.validate().is_ok());
        let mut value = serde_json::to_value(req()).unwrap();
        let vm = value["vms"][0].as_object_mut().unwrap();
        for field in ["networks", "require_guest_agent", "boot_timeout_secs"] {
            vm.remove(field);
        }
        let r: ImportRequest = serde_json::from_value(value).unwrap();
        assert!(r.vms[0].networks.is_empty());
        assert!(!r.vms[0].require_guest_agent);
        assert!(r.validate().is_ok());
    }

    #[test]
    fn validates_vcenter_hosts_and_addresses() {
        for host in [
            "vc.example.com",
            "192.0.2.1",
            "2001:db8::1",
            "[2001:db8::1]",
        ] {
            assert!(hostname(host), "{host}");
        }
        for host in [
            "vc..example.com",
            "vc:443",
            "[invalid]",
            "https://vc.example.com",
            "vc-.example.com",
        ] {
            assert!(!hostname(host), "{host}");
        }
    }

    #[tokio::test]
    async fn preflight_absence_requires_three_explicit_not_found_responses() {
        let api = test_api::mock(vec![
            test_api::missing(),
            test_api::missing(),
            test_api::missing(),
        ])
        .await;
        assert!(ensure_target_available(&api.client, "vms", "web-01")
            .await
            .is_ok());
        let requests = api.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1]
            .1
            .ends_with("/persistentvolumeclaims/web-01-root"));
        assert!(requests[2].1.ends_with("/datavolumes/web-01-root"));
    }

    #[tokio::test]
    async fn preflight_denied_read_or_existing_disk_blocks_import() {
        use axum::http::StatusCode;
        let denied = test_api::mock(vec![(StatusCode::FORBIDDEN, serde_json::json!({"kind":"Status","apiVersion":"v1","reason":"Forbidden","message":"denied","code":403}))]).await;
        assert!(ensure_target_available(&denied.client, "vms", "web-01")
            .await
            .unwrap_err()
            .to_string()
            .contains("cannot establish"));
        assert_eq!(denied.requests.lock().unwrap().len(), 1);
        let occupied = test_api::mock(vec![test_api::missing(), (StatusCode::OK, serde_json::json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":"web-01-root"}}))]).await;
        assert!(ensure_target_available(&occupied.client, "vms", "web-01")
            .await
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        assert_eq!(occupied.requests.lock().unwrap().len(), 2);
    }

    fn op_params() -> ImportOpParams {
        let r = req();
        ImportOpParams {
            wave_id: "wave-1".into(),
            source: r.source.clone(),
            namespace: "vms".into(),
            vm: r.vms[0].clone(),
            target_vm_name: "web-prod-01".into(),
            scratch_size: "200Gi".into(),
            scratch_storage_class: None,
            service_account: "zorvia-h2kvm".into(),
            mode: ImportMode::Vsphere,
        }
    }

    #[test]
    fn builds_h2kvmctl_args_without_credentials() {
        let a = h2kvm_args(&op_params());
        let joined = a.join(" ");
        assert!(joined.starts_with("--cmd vsphere --vcenter vc.example.com"));
        assert!(joined.contains("--vc-user $(VC_USER) --vc-password-env VC_PASSWORD"));
        assert!(joined.contains("--vs-vm Web Prod_01"));
        assert!(joined.contains("--deploy-k8s --k8s-namespace vms --k8s-vm-name web-prod-01"));
        assert!(joined.contains("--k8s-pvc-name web-prod-01-root"));
        assert!(joined.contains("--dc-name DC1"));
        assert!(joined.contains("--k8s-storage-class longhorn --k8s-pvc-size 60Gi"));
        assert!(joined.contains("--k8s-cpu 4 --k8s-memory 8Gi"));
        assert!(!a.contains(&"--k8s-auto-start".to_string()));
        assert!(!a.contains(&"--vc-insecure".to_string()));
        // The VM name is a single argv element, never split on spaces.
        assert!(a.iter().any(|x| x == "Web Prod_01"));
        let mut p = op_params();
        p.source.insecure = true;
        p.source.port = Some(8443);
        p.vm.start = false;
        let a = h2kvm_args(&p);
        assert!(a.contains(&"--vc-insecure".to_string()));
        assert!(a.windows(2).any(|w| w == ["--vc-port", "8443"]));
        assert!(!a.contains(&"--k8s-auto-start".to_string()));
    }

    #[test]
    fn parses_stage_markers_deploy_and_failure() {
        let logs = "starting\n➡️  Stage: INSPECT\n✅ inspect completed in 1.20s\n\
                    ➡️  Stage: FIX\n✅ fix completed in 30.00s\n➡️  Stage: CONVERT\n";
        let p = parse_h2kvm_logs(logs);
        assert_eq!((p.stages_started, p.stages_done), (3, 2));
        assert_eq!(p.current_stage.as_deref(), Some("convert"));
        assert_eq!(p.percent(), 25);
        assert!(p.deployed_vm.is_none() && p.failure.is_none());

        let done = format!("{logs}✅ Deployed: web-prod-01\n   Namespace: vms\n");
        let p = parse_h2kvm_logs(&done);
        assert_eq!(p.deployed_vm.as_deref(), Some("web-prod-01"));
        assert_eq!(p.percent(), 95);

        let failed = "➡️  Stage: FIX\n❌ fix failed: guest has no /etc/fstab\n";
        let p = parse_h2kvm_logs(failed);
        assert_eq!(
            p.failure.as_deref(),
            Some("fix failed: guest has no /etc/fstab")
        );
        assert_eq!(
            parse_h2kvm_logs("nothing useful\n"),
            H2kvmProgress::default()
        );
    }

    #[test]
    fn waves_queue_one_operation_per_vm_and_block_duplicates() {
        let db = OperationsDb::open(":memory:").unwrap();
        let mut r = req();
        let mut second = r.vms[0].clone();
        second.source_vm = "db-01".into();
        r.vms.push(second);
        let (wave, ops) = enqueue_wave_in(&db, &r).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|o| o.kind == OP_KIND && o.max_attempts == 1));
        assert_eq!(ops[0].params["wave_id"], wave.as_str());
        assert_eq!(ops[0].params["scratch_size"], "200Gi");
        // Credentials are never persisted, only the Secret name.
        assert!(!ops[0].params.to_string().contains("\"password\":"));
        assert!(
            enqueue_wave_in(&db, &r).is_err(),
            "same targets already queued"
        );

        let s = wave_summary(&db, &wave).unwrap();
        assert_eq!(s["total"], 2);
        assert_eq!(s["queued"], 2);
        // Newest first; order-independent check.
        let targets: Vec<&str> = s["operations"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|o| o["target_vm_name"].as_str())
            .collect();
        assert!(targets.contains(&"db-01") && targets.contains(&"web-prod-01"));
    }
}
