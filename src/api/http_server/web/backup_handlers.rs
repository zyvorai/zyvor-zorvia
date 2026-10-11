//! Real VM backup API backed by `crate::snapshots::SnapshotManager` -- the
//! same live VolumeSnapshot infrastructure `Snapshots.tsx` already uses,
//! reframed fleet-wide as "backups" instead of one VM at a time.
//!
//! Fixes a live bug: `web/src/api/backup.ts`'s `createBackup()` is already
//! called today by `VMList.tsx`'s bulk-backup action, but `/api/backups`
//! never existed anywhere in the router -- it silently 404'd.
//!
//! Every real snapshot is a complete point-in-time capture (there is no
//! incremental-backup engine here), so `backup_type` is accepted on create
//! but always reported back as `"full"`. `retention_days` is stored as a
//! label for future use; nothing currently enforces it automatically --
//! `SnapshotManager::apply_retention_policy` exists and works, it's just
//! not wired to a scheduler yet (that's `BackupScheduler`, a bigger lift
//! for its own slice).

use super::*;
use crate::snapshots::{SnapshotConfig, SnapshotInfo, SnapshotManager, SnapshotStatus};
use axum::extract::Json as AxumJson;
use serde_json::json;

const LABEL_RETENTION_DAYS: &str = "zorvia.io/retention-days";

fn backup_json(s: &SnapshotInfo) -> serde_json::Value {
    let status = match s.status {
        SnapshotStatus::Succeeded => "completed",
        SnapshotStatus::InProgress => "in_progress",
        SnapshotStatus::Failed => "failed",
        SnapshotStatus::Unknown => "in_progress",
    };
    let retention_days: Option<u32> = s
        .labels
        .get(LABEL_RETENTION_DAYS)
        .and_then(|v| v.parse().ok());
    let expires_at = match (s.created_at, retention_days) {
        (Some(created), Some(days)) => {
            Some((created + chrono::Duration::days(days as i64)).to_rfc3339())
        }
        _ => None,
    };
    let warning = s.captured_no_volumes().then(|| {
        format!(
            "VM '{}' has no PVC/DataVolume-backed disks -- this backup captured only the VM's configuration, not any disk data. There is nothing to restore from disk.",
            s.vm_name
        )
    });
    json!({
        "id": s.name,
        "vm_name": s.vm_name,
        "backup_type": "full",
        "size_bytes": s.size.as_deref().and_then(|sz| sz.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().ok()).unwrap_or(0),
        "compressed": false,
        "created": s.created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
        "status": status,
        "storage_location": format!("kubevirt-snapshot:{}/{}", s.namespace, s.name),
        "retention_days": retention_days.unwrap_or(0),
        "expires_at": expires_at,
        "metadata": serde_json::Value::Null,
        "warning": warning,
    })
}

fn backup_job_json(s: &SnapshotInfo) -> serde_json::Value {
    let status = match s.status {
        SnapshotStatus::Succeeded => "completed",
        SnapshotStatus::InProgress => "running",
        SnapshotStatus::Failed => "failed",
        SnapshotStatus::Unknown => "running",
    };
    let warning = s.captured_no_volumes().then(|| {
        format!(
            "VM '{}' has no PVC/DataVolume-backed disks -- this backup captured only the VM's configuration, not any disk data. There is nothing to restore from disk.",
            s.vm_name
        )
    });
    json!({
        "id": s.name,
        "backup_id": s.name,
        "vm_name": s.vm_name,
        "operation": "backup",
        "status": status,
        "progress": if s.ready_to_use { 100 } else { 0 },
        "started_at": s.created_at.map(|t| t.to_rfc3339()),
        "completed_at": s.completed_at.map(|t| t.to_rfc3339()),
        "error": s.error,
        "warning": warning,
    })
}

fn backup_error(action: &str, e: impl std::fmt::Display) -> axum::response::Response {
    // Classify on the full cause chain (`{:#}`): an `anyhow` context wrapper such
    // as "Failed to get snapshot 'x'" hides the Kubernetes NotFound underneath,
    // and `sanitize_error` would otherwise mask it as a generic 500.
    let full = format!("{e:#}").to_ascii_lowercase();
    let (code, kind) = if full.contains("notfound") || full.contains("not found") {
        (404, "NOT_FOUND")
    } else if full.contains("already exists") || full.contains("conflict") {
        (409, "CONFLICT")
    } else {
        (500, "BACKUP_FAILED")
    };
    let raw = if code == 404 {
        "not found".to_string()
    } else {
        sanitize_error(&e)
    };
    let (st, j) = err_json(code, kind, &format!("Failed to {action}: {raw}"));
    (st, j).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListBackupsQuery {
    #[serde(default)]
    pub vm: Option<String>,
}

pub async fn list_backups_handler(
    State(state): State<SharedState>,
    Query(q): Query<ListBackupsQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    let manager = match SnapshotManager::new(&namespace).await {
        Ok(m) => m,
        Err(e) => return backup_error("connect to cluster", e),
    };
    match manager.list_snapshots_sorted(q.vm.as_deref()).await {
        Ok(snaps) => Json(snaps.iter().map(backup_json).collect::<Vec<_>>()).into_response(),
        Err(e) => backup_error("list backups", e),
    }
}

/// Core of "create a backup" -- shared by the manual `POST /backups` handler
/// and `BackupScheduler`'s execution loop, so a scheduled run does exactly
/// what a manual one does.
pub(crate) async fn run_backup(
    namespace: &str,
    vm_name: &str,
    backup_type: &str,
    retention_days: Option<u32>,
    description: Option<String>,
) -> anyhow::Result<SnapshotInfo> {
    let manager = SnapshotManager::new(namespace).await?;
    let backup_name = format!(
        "backup-{}-{}",
        vm_name,
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let mut config = SnapshotConfig::new(vm_name, &backup_name)
        .with_label("zorvia.io/backup", "true")
        .with_label("zorvia.io/backup-type", backup_type);
    if let Some(days) = retention_days {
        config = config.with_label(LABEL_RETENTION_DAYS, days.to_string());
    }
    if let Some(desc) = description {
        config = config.with_description(desc);
    }
    manager.create_snapshot(&config).await
}

#[derive(Debug, Deserialize)]
pub struct CreateBackupBody {
    pub vm_name: String,
    #[serde(default)]
    pub backup_type: Option<String>,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

pub async fn create_backup_handler(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<CreateBackupBody>,
) -> impl IntoResponse {
    if body.vm_name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "vm_name is required");
        return (st, j).into_response();
    }
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    match run_backup(
        &namespace,
        &body.vm_name,
        body.backup_type.as_deref().unwrap_or("full"),
        body.retention_days,
        body.description,
    )
    .await
    {
        Ok(info) => (StatusCode::CREATED, Json(backup_job_json(&info))).into_response(),
        Err(e) => backup_error("create backup", e),
    }
}

pub async fn delete_backup_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    let manager = match SnapshotManager::new(&namespace).await {
        Ok(m) => m,
        Err(e) => return backup_error("connect to cluster", e),
    };
    match manager.delete_snapshot(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => backup_error("delete backup", e),
    }
}

fn restore_job_json(r: &crate::snapshots::RestoreInfo) -> serde_json::Value {
    let status = match r.status {
        crate::snapshots::RestoreStatus::Succeeded => "completed",
        crate::snapshots::RestoreStatus::InProgress => "running",
        crate::snapshots::RestoreStatus::Failed => "failed",
        crate::snapshots::RestoreStatus::Unknown => "running",
    };
    json!({
        "id": r.name,
        "backup_id": r.snapshot_name,
        "vm_name": r.target_vm_name,
        "operation": "restore",
        "status": status,
        "progress": if matches!(r.status, crate::snapshots::RestoreStatus::Succeeded) { 100 } else { 0 },
        "started_at": r.created_at.map(|t| t.to_rfc3339()),
        "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
        "error": r.error,
    })
}

#[derive(Debug, Deserialize)]
pub struct RestoreBackupBody {
    pub backup_id: String,
    #[serde(default)]
    pub target_vm_name: Option<String>,
    // Accepted for API-shape compatibility with `web/src/api/backup.ts`'s
    // `RestoreOptions` -- a full point-in-time snapshot restore always
    // restores config+disks+state together (there's no incremental/partial
    // restore engine here, matching how `backup_type` works on create).
    #[serde(default)]
    #[allow(dead_code)]
    pub restore_config: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    pub restore_disks: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    pub restore_state: Option<bool>,
}

/// `POST /backups/restore` -- restores a backup either in-place (onto its
/// original VM, which must be stopped) or to a new VM (when `target_vm_name`
/// names a different VM than the one the snapshot was taken from). Wraps
/// the same `RestoreManager` the working `POST /vms/:name/snapshots/:id/revert`
/// endpoint already uses.
pub async fn restore_backup_handler(
    State(state): State<SharedState>,
    AxumJson(body): AxumJson<RestoreBackupBody>,
) -> impl IntoResponse {
    if body.backup_id.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "backup_id is required");
        return (st, j).into_response();
    }
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    let manager = match SnapshotManager::new(&namespace).await {
        Ok(m) => m,
        Err(e) => return backup_error("connect to cluster", e),
    };
    let snapshot = match manager.get_snapshot(&body.backup_id).await {
        Ok(s) => s,
        Err(e) => return backup_error("find backup", e),
    };

    let restore_manager = match crate::snapshots::RestoreManager::new(&namespace).await {
        Ok(rm) => rm,
        Err(e) => return backup_error("connect to cluster", e),
    };
    let result = match &body.target_vm_name {
        Some(target) if *target != snapshot.vm_name => {
            restore_manager
                .restore_to_new_vm(&body.backup_id, target, true)
                .await
        }
        _ => {
            restore_manager
                .restore_in_place(&snapshot.vm_name, &body.backup_id)
                .await
        }
    };
    match result {
        Ok(info) => (StatusCode::CREATED, Json(restore_job_json(&info))).into_response(),
        Err(e) => {
            let raw = format!("{e:#}");
            log::warn!("restore of backup '{}' failed: {raw}", body.backup_id);
            let (code, kind, msg) =
                crate::kube::lifecycle::classify_restore_error("restore this backup", &raw);
            let (st, j) = err_json(code, kind, &msg);
            (st, j).into_response()
        }
    }
}

/// `GET /backups/jobs` -- fleet-wide backup+restore job status, reusing the
/// same `backup_job_json` shape `create_backup_handler` already returns.
pub async fn list_backup_jobs_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    let manager = match SnapshotManager::new(&namespace).await {
        Ok(m) => m,
        Err(e) => return backup_error("connect to cluster", e),
    };
    match manager.list_snapshots_sorted(None).await {
        Ok(snaps) => Json(snaps.iter().map(backup_job_json).collect::<Vec<_>>()).into_response(),
        Err(e) => backup_error("list backup jobs", e),
    }
}

/// `GET /backups/jobs/:id` -- single backup job status by id.
pub async fn get_backup_job_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = s.namespace.clone();
    drop(s);

    let manager = match SnapshotManager::new(&namespace).await {
        Ok(m) => m,
        Err(e) => return backup_error("connect to cluster", e),
    };
    match manager.get_snapshot(&id).await {
        Ok(info) => Json(backup_job_json(&info)).into_response(),
        Err(e) => backup_error("find backup job", e),
    }
}

#[cfg(test)]
mod backup_error_tests {
    use super::*;

    #[test]
    fn wrapped_kube_not_found_maps_to_404() {
        let e = anyhow::anyhow!("ApiError: virtualmachinesnapshots \"j1\" not found: NotFound")
            .context("Failed to get snapshot 'j1'");
        assert_eq!(backup_error("find backup job", e).status(), 404);
    }

    #[test]
    fn unknown_failure_stays_500() {
        let e = anyhow::anyhow!("boom").context("Failed to get snapshot 'j1'");
        assert_eq!(backup_error("find backup job", e).status(), 500);
    }
}
