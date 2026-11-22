// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Golden-image capture: clone a VM's persistent disk into a standalone CDI
//! DataVolume. Each capture is a durable operation (`operations` module):
//! the job stays `Running` with CDI's own progress until the DataVolume
//! reports Succeeded, a CDI failure marks it `Failed` with CDI's reason, and
//! a restart of Zorvia resumes tracking the existing DataVolume.

use crate::kube::cdi::{
    data_volume_clone_manifest, data_volume_progress_from_object, CdiPvcCloneSpec, DataVolumeWait,
};
use crate::kube::KubeClient;
use crate::operations::runtime::{HandlerFuture, OpContext, Outcome};
use crate::operations::{NewOperation, OpState, Operation, OperationsDb};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Operation `kind` for golden-image capture.
pub const OP_KIND: &str = "golden-image-convert";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConvertStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertJob {
    pub id: String,
    pub status: ConvertStatus,
    pub progress: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

/// Why a conversion could not even be started -- distinct from a job that
/// started and then failed (that's reported as a `Failed` `ConvertJob`
/// instead, same as `DownloadRegistry`).
#[derive(Debug)]
pub enum ConvertStartError {
    VmNotFound(String),
    NoPersistentDisk(String),
    Queue(String),
}

impl std::fmt::Display for ConvertStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VmNotFound(e) => write!(f, "{e}"),
            Self::NoPersistentDisk(e) => write!(f, "{e}"),
            Self::Queue(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConvertParams {
    namespace: String,
    source_pvc: String,
    size: String,
    target_name: String,
}

/// View over the durable operations store, keeping the `ConvertJob` shape
/// the API and frontend already use.
pub struct ConvertRegistry;

fn job_from_op(op: &Operation) -> ConvertJob {
    let output_path = op
        .result
        .as_ref()
        .and_then(|r| r.get("output_path"))
        .and_then(|p| p.as_str())
        .map(String::from);
    match op.state {
        OpState::Queued | OpState::Running => ConvertJob {
            id: op.id.clone(),
            status: ConvertStatus::Running,
            progress: op.progress,
            error: None,
            output_path: None,
        },
        OpState::Succeeded => ConvertJob {
            id: op.id.clone(),
            status: ConvertStatus::Completed,
            progress: 100,
            error: None,
            output_path,
        },
        OpState::Failed | OpState::Cancelled => ConvertJob {
            id: op.id.clone(),
            status: ConvertStatus::Failed,
            progress: op.progress,
            error: Some(op.error.clone().unwrap_or_else(|| "cancelled".into())),
            output_path: None,
        },
    }
}

impl ConvertRegistry {
    pub fn global() -> &'static Self {
        static REG: ConvertRegistry = ConvertRegistry;
        &REG
    }

    pub fn get(&self, id: &str) -> Option<ConvertJob> {
        match OperationsDb::global().get(id) {
            Ok(Some(op)) if op.kind == OP_KIND => Some(job_from_op(&op)),
            _ => None,
        }
    }

    /// Validate `vm_name`'s persistent disk (PVC- or DataVolume-backed) and
    /// queue a durable operation that clones it into a standalone
    /// DataVolume named after `image_name`. Ephemeral
    /// `emptyDisk`/`containerDisk`-only VMs have no persistent bytes to
    /// capture and are rejected before any operation is created.
    pub async fn start(
        &self,
        vm_name: &str,
        image_name: &str,
        namespace: &str,
        client: &KubeClient,
    ) -> Result<ConvertJob, ConvertStartError> {
        let vm = client
            .get_vm(namespace, vm_name)
            .await
            .map_err(|e| ConvertStartError::VmNotFound(format!("VM '{vm_name}' not found: {e}")))?;

        let no_disk_err = || {
            ConvertStartError::NoPersistentDisk(format!(
                "VM '{vm_name}' has no persistent disk to capture (blank/containerdisk disks \
                 are ephemeral) -- golden image capture requires a VM created with a PVC- or \
                 DataVolume-backed disk"
            ))
        };
        let volumes = vm
            .spec
            .template
            .spec
            .volumes
            .as_ref()
            .ok_or_else(no_disk_err)?;

        let (source_pvc, size) =
            if let Some(vol) = volumes.iter().find(|v| v.persistent_volume_claim.is_some()) {
                let claim_name = vol
                    .persistent_volume_claim
                    .as_ref()
                    .expect("checked is_some above")
                    .claim_name
                    .clone();
                let size = match client.get_pvc(namespace, &claim_name).await {
                    Ok(pvc) => pvc
                        .spec
                        .as_ref()
                        .and_then(|s| s.resources.as_ref())
                        .and_then(|r| r.requests.as_ref())
                        .and_then(|req| req.get("storage"))
                        .map(|q| q.0.clone())
                        .unwrap_or_else(|| "20Gi".into()),
                    Err(_) => "20Gi".into(),
                };
                (claim_name, size)
            } else if let Some(vol) = volumes.iter().find(|v| v.data_volume.is_some()) {
                let name = vol
                    .data_volume
                    .as_ref()
                    .expect("checked is_some above")
                    .name
                    .clone();
                (name, "20Gi".to_string())
            } else {
                return Err(no_disk_err());
            };

        let target_name = slug_image_name(image_name);
        let db = OperationsDb::global();

        // One capture per target DataVolume at a time: a repeated request
        // while one is queued/running returns that job instead of racing it.
        let dup = db.list_active().unwrap_or_default().into_iter().find(|o| {
            o.kind == OP_KIND
                && o.namespace == namespace
                && o.params.get("target_name").and_then(|t| t.as_str())
                    == Some(target_name.as_str())
        });
        if let Some(op) = dup {
            return Ok(job_from_op(&op));
        }

        let mut new = NewOperation::new(OP_KIND, vm_name, namespace);
        new.params = serde_json::to_value(ConvertParams {
            namespace: namespace.to_string(),
            source_pvc,
            size,
            target_name,
        })
        .unwrap_or_default();
        match db.create(new) {
            Ok((op, _)) => Ok(job_from_op(&op)),
            Err(e) => Err(ConvertStartError::Queue(format!(
                "could not queue capture: {e}"
            ))),
        }
    }
}

/// Handler run by the operations reconciler. Safe to re-run: if the clone
/// DataVolume already exists (previous attempt, or Zorvia restarted), it
/// resumes tracking it instead of creating it again.
pub fn run_op(ctx: OpContext) -> HandlerFuture {
    Box::pin(run_convert(ctx))
}

async fn run_convert(ctx: OpContext) -> Outcome {
    let params: ConvertParams = match serde_json::from_value(ctx.op.params.clone()) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(format!("invalid operation params: {e}")),
    };
    let ns = params.namespace.as_str();
    let dv = params.target_name.as_str();

    ctx.progress("creating-datavolume", 1);
    let spec = CdiPvcCloneSpec {
        source_namespace: params.namespace.clone(),
        source_pvc: params.source_pvc.clone(),
        target_namespace: params.namespace.clone(),
        target_name: params.target_name.clone(),
        size: params.size.clone(),
        storage_class: None,
    };
    let manifest = match data_volume_clone_manifest(&spec) {
        Ok(m) => m,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    if let Err(e) = ctx.client.apply_data_volume(ns, &manifest).await {
        // Already there (resumed attempt) is fine; anything else retries.
        if ctx.client.get_data_volume(ns, dv).await.is_err() {
            return Outcome::Retry(format!("failed to create DataVolume: {e}"));
        }
    }

    let timeout = Duration::from_secs(
        std::env::var("ZORVIA_IMAGE_JOB_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3600),
    );
    let started = std::time::Instant::now();
    loop {
        if ctx.cancelled() {
            return Outcome::Cancelled;
        }
        match ctx.client.get_data_volume(ns, dv).await {
            Ok(obj) => {
                let p = data_volume_progress_from_object(&obj);
                match p.wait {
                    // "datavolume:" tells fabric_create_vm's image parser to
                    // attach this disk via add_data_volume_disk.
                    DataVolumeWait::Ready => {
                        return Outcome::Succeeded(Some(
                            serde_json::json!({"output_path": format!("datavolume:{dv}")}),
                        ))
                    }
                    DataVolumeWait::Failed => {
                        return Outcome::Failed(
                            p.failure_reason
                                .unwrap_or_else(|| "DataVolume failed".into()),
                        )
                    }
                    DataVolumeWait::Pending => {
                        ctx.progress("cloning", p.percent.unwrap_or(1).clamp(1, 99));
                    }
                }
            }
            Err(e) => log::debug!("golden-image {}: DataVolume {dv}: {e}", ctx.op.id),
        }
        if started.elapsed() >= timeout {
            return Outcome::Failed(format!(
                "timed out after {}s waiting for DataVolume '{dv}'",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// DNS-label-safe DataVolume name derived from the user-supplied image name.
fn slug_image_name(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "golden-image".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_strips_invalid_chars() {
        assert_eq!(slug_image_name("My Golden_Image!"), "my-golden-image");
    }

    #[test]
    fn slug_falls_back_when_fully_invalid() {
        assert_eq!(slug_image_name("___"), "golden-image");
    }

    fn op(state: OpState) -> Operation {
        let db = OperationsDb::open(":memory:").unwrap();
        let (mut op, _) = db
            .create(NewOperation::new(OP_KIND, "vm", "default"))
            .unwrap();
        op.state = state;
        op
    }

    #[test]
    fn maps_operation_states_to_job_status() {
        assert_eq!(
            job_from_op(&op(OpState::Queued)).status,
            ConvertStatus::Running
        );
        assert_eq!(
            job_from_op(&op(OpState::Running)).status,
            ConvertStatus::Running
        );

        let mut done = op(OpState::Succeeded);
        done.result = Some(serde_json::json!({"output_path": "datavolume:img"}));
        let j = job_from_op(&done);
        assert_eq!(j.status, ConvertStatus::Completed);
        assert_eq!(j.progress, 100);
        assert_eq!(j.output_path.as_deref(), Some("datavolume:img"));

        let mut failed = op(OpState::Failed);
        failed.error = Some("clone pod crashed".into());
        let j = job_from_op(&failed);
        assert_eq!(j.status, ConvertStatus::Failed);
        assert_eq!(j.error.as_deref(), Some("clone pod crashed"));

        assert_eq!(
            job_from_op(&op(OpState::Cancelled)).error.as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn convert_start_error_displays_message() {
        let err = ConvertStartError::NoPersistentDisk("no disk here".to_string());
        assert_eq!(err.to_string(), "no disk here");
    }
}
