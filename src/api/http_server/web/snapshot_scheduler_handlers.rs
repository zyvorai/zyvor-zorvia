//! Scheduled per-VM snapshots, backed by `crate::snapshots::schedule`.
//!
//! `spawn_snapshot_scheduler_loop` ticks every 60s (same cadence and leader gating as the
//! backup scheduler). For each due schedule it snapshots every matching VM under a
//! deterministic per-slot name, so a restart mid-run re-creates nothing twice, then prunes
//! only the snapshots that schedule itself created.

use super::*;
use crate::backup::schedule::{ScheduleType, VMSelector};
use crate::snapshots::schedule::{
    snapshots_to_prune, validate_schedule_name, SnapshotSchedule, SnapshotScheduleManager,
    SCHEDULED_BY_LABEL,
};
use crate::snapshots::{RetentionPolicy, SnapshotConfig, SnapshotManager};
use axum::extract::Json as AxumJson;
use chrono::Utc;
use serde_json::json;

fn schedule_type_label(t: &ScheduleType) -> &'static str {
    match t {
        ScheduleType::Hourly { .. } => "hourly",
        ScheduleType::Daily { .. } => "daily",
        ScheduleType::Weekly { .. } => "weekly",
        ScheduleType::Monthly { .. } => "monthly",
        ScheduleType::Cron { .. } => "cron",
    }
}

fn schedule_json(s: &SnapshotSchedule) -> serde_json::Value {
    let vm_names: Vec<String> = match &s.vm_selector {
        VMSelector::All => Vec::new(),
        VMSelector::ByName(names) => names.clone(),
        VMSelector::ByLabel { key, value } => vec![format!("{key}={value}")],
    };
    json!({
        "id": s.name,
        "name": s.name,
        "vm_names": vm_names,
        "schedule_type": schedule_type_label(&s.schedule_type),
        "retention": {
            "max_snapshots": s.retention.max_snapshots,
            "max_age_days": s.retention.max_age_days,
            "keep_last_n": s.retention.keep_last_n,
        },
        "enabled": s.enabled,
        "last_run": s.last_run.map(|t| t.to_rfc3339()),
        "next_run": s.next_run.map(|t| t.to_rfc3339()),
        "last_error": s.last_error,
    })
}

pub async fn list_snapshot_schedules_handler() -> impl IntoResponse {
    let mgr = SnapshotScheduleManager::load();
    Json(mgr.all().iter().map(schedule_json).collect::<Vec<_>>()).into_response()
}

pub async fn list_vm_snapshot_schedules_handler(Path(vm): Path<String>) -> impl IntoResponse {
    let mgr = SnapshotScheduleManager::load();
    Json(
        mgr.schedules_for_vm(&vm)
            .into_iter()
            .map(schedule_json)
            .collect::<Vec<_>>(),
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotScheduleBody {
    pub name: String,
    /// "hourly" | "daily" | "weekly" | "monthly"
    pub schedule_type: String,
    #[serde(default)]
    pub hour: Option<u32>,
    #[serde(default)]
    pub minute: Option<u32>,
    /// Required for "weekly" (e.g. "sunday").
    #[serde(default)]
    pub weekday: Option<String>,
    /// Required for "monthly" (1-31).
    #[serde(default)]
    pub day_of_month: Option<u32>,
    /// Snapshots every VM in the namespace when omitted or empty.
    #[serde(default)]
    pub vm_names: Option<Vec<String>>,
    #[serde(default)]
    pub max_snapshots: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub keep_last_n: Option<u32>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn create_snapshot_schedule_handler(
    AxumJson(body): AxumJson<CreateSnapshotScheduleBody>,
) -> impl IntoResponse {
    if let Err(msg) = validate_schedule_name(&body.name) {
        let (st, j) = err_json(400, "INVALID_NAME", &msg);
        return (st, j).into_response();
    }
    let hour = body.hour.unwrap_or(2).min(23);
    let minute = body.minute.unwrap_or(0).min(59);
    let schedule_type = match body.schedule_type.as_str() {
        "hourly" => ScheduleType::hourly(minute),
        "daily" => ScheduleType::daily(hour, minute),
        "weekly" => {
            let Some(weekday) = body.weekday.as_deref().and_then(parse_weekday) else {
                let (st, j) = err_json(
                    400,
                    "INVALID_WEEKDAY",
                    "weekly schedules require a valid `weekday` (e.g. \"sunday\")",
                );
                return (st, j).into_response();
            };
            ScheduleType::weekly(weekday, hour, minute)
        }
        "monthly" => ScheduleType::monthly(body.day_of_month.unwrap_or(1), hour, minute),
        other => {
            let (st, j) = err_json(
                400,
                "INVALID_SCHEDULE_TYPE",
                &format!("Unknown schedule_type '{other}' (use hourly, daily, weekly, or monthly)"),
            );
            return (st, j).into_response();
        }
    };

    let mut mgr = SnapshotScheduleManager::load();
    if mgr.get_schedule(&body.name).is_some() {
        let (st, j) = err_json(
            409,
            "CONFLICT",
            "A snapshot schedule with this name already exists",
        );
        return (st, j).into_response();
    }

    let mut schedule = SnapshotSchedule::new(&body.name, schedule_type);
    if let Some(names) = body.vm_names.filter(|v| !v.is_empty()) {
        schedule.vm_selector = VMSelector::ByName(names);
    }
    let defaults = RetentionPolicy::default();
    schedule.retention = RetentionPolicy {
        max_snapshots: body.max_snapshots.or(defaults.max_snapshots),
        max_age_days: body.max_age_days.or(defaults.max_age_days),
        keep_last_n: body.keep_last_n.or(defaults.keep_last_n),
    };
    if body.enabled == Some(false) {
        schedule.enabled = false;
    }
    schedule.next_run = schedule.calculate_next_run(Utc::now());

    let response = schedule_json(&schedule);
    mgr.add_schedule(schedule);
    (StatusCode::CREATED, Json(response)).into_response()
}

pub async fn delete_snapshot_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    let mut mgr = SnapshotScheduleManager::load();
    if mgr.remove_schedule(&name) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such snapshot schedule");
        (st, j).into_response()
    }
}

async fn set_schedule_enabled(name: &str, enabled: bool) -> impl IntoResponse {
    let mut mgr = SnapshotScheduleManager::load();
    let Some(mut schedule) = mgr.get_schedule(name).cloned() else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such snapshot schedule");
        return (st, j).into_response();
    };
    schedule.enabled = enabled;
    schedule.next_run = if enabled {
        schedule.calculate_next_run(Utc::now())
    } else {
        None
    };
    let response = schedule_json(&schedule);
    mgr.update_schedule(schedule);
    Json(response).into_response()
}

pub async fn enable_snapshot_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_schedule_enabled(&name, true).await
}

pub async fn disable_snapshot_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_schedule_enabled(&name, false).await
}

/// True when the error chain contains a Kubernetes 409 AlreadyExists, which for a
/// deterministic per-slot name means an earlier tick already created this snapshot.
fn is_already_exists(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|c| c.downcast_ref::<kube::Error>())
        .any(|e| matches!(e, kube::Error::Api(resp) if resp.code == 409))
}

pub fn spawn_snapshot_scheduler_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if crate::api::leader::skip_if_follower("snapshot-scheduler") {
                continue;
            }
            run_due_snapshot_schedules(&state).await;
        }
    });
}

async fn run_due_snapshot_schedules(state: &SharedState) {
    let now = Utc::now();
    let due: Vec<SnapshotSchedule> = SnapshotScheduleManager::load()
        .get_due_schedules(now)
        .into_iter()
        .cloned()
        .collect();
    if due.is_empty() {
        return;
    }

    let (namespace, kube_client) = {
        let s = state.read().await;
        (s.namespace.clone(), s.kube_client.clone())
    };
    let manager = SnapshotManager::from_client(kube_client.client(), namespace.clone());

    for schedule in due {
        let vm_names: Vec<String> = match &schedule.vm_selector {
            VMSelector::All => match kube_client.list_vms(&namespace).await {
                Ok(vms) => vms.into_iter().filter_map(|vm| vm.metadata.name).collect(),
                Err(e) => {
                    // Cluster unreachable: leave next_run alone so the next tick retries.
                    log::error!(
                        "Snapshot scheduler: failed to list VMs for schedule '{}': {e}",
                        schedule.name
                    );
                    continue;
                }
            },
            VMSelector::ByName(names) => names.clone(),
            VMSelector::ByLabel { .. } => {
                log::warn!(
                    "Snapshot scheduler: schedule '{}' uses a label selector, which isn't wired to real VM label lookup -- skipping this run",
                    schedule.name
                );
                continue;
            }
        };

        let slot = schedule.next_run.unwrap_or(now);
        let mut failures: Vec<String> = Vec::new();
        for vm in &vm_names {
            let snap_name = schedule.snapshot_name(vm, slot);
            // Let a database in the guest flush first when the agent is connected.
            let _ = crate::guest_rpc::pre_snapshot_hooks(&kube_client, &namespace, vm).await;
            let cfg = SnapshotConfig::new(vm, &snap_name)
                .with_description(format!("scheduled snapshot ({})", schedule.name))
                .with_label(SCHEDULED_BY_LABEL, &schedule.name);
            let result = manager.create_snapshot(&cfg).await;
            let ok = match &result {
                Ok(_) => true,
                Err(e) => is_already_exists(e),
            };
            record_audit(
                state,
                &HeaderMap::new(),
                crate::audit_trail::AuditAction::Snapshot,
                "vm",
                vm,
                ok,
                result
                    .as_ref()
                    .err()
                    .filter(|_| !ok)
                    .map(|e| sanitize_error(e)),
            )
            .await;
            if ok {
                log::info!(
                    "Snapshot scheduler: snapshot {snap_name} for VM '{vm}' (schedule '{}')",
                    schedule.name
                );
                prune_scheduled(&manager, vm, &schedule).await;
            } else if let Err(e) = result {
                log::error!(
                    "Snapshot scheduler: snapshot for VM '{vm}' (schedule '{}') failed: {e:#}",
                    schedule.name
                );
                failures.push(format!("{vm}: {}", sanitize_error(&e)));
            }
        }

        // Advance regardless of per-VM failures: no retry storm, the failure is recorded
        // on the schedule and the next slot tries again.
        let mut mgr = SnapshotScheduleManager::load();
        if let Some(mut current) = mgr.get_schedule(&schedule.name).cloned() {
            current.last_run = Some(now);
            current.next_run = current.calculate_next_run(now);
            current.last_error = if failures.is_empty() {
                None
            } else {
                Some(failures.join("; "))
            };
            mgr.update_schedule(current);
        }
    }
}

/// Apply the schedule's retention to the snapshots it created for `vm`.
async fn prune_scheduled(manager: &SnapshotManager, vm: &str, schedule: &SnapshotSchedule) {
    let snapshots = match manager.list_snapshots_for_vm(vm).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Snapshot scheduler: could not list snapshots of '{vm}' for retention: {e}");
            return;
        }
    };
    for name in snapshots_to_prune(&snapshots, &schedule.name, &schedule.retention, Utc::now()) {
        if manager.is_snapshot_in_use(&name).await.unwrap_or(true) {
            log::warn!("Snapshot scheduler: not pruning '{name}', it may be in use by a restore");
            continue;
        }
        match manager.delete_snapshot(&name).await {
            Ok(()) => log::info!(
                "Snapshot scheduler: pruned {name} (schedule '{}')",
                schedule.name
            ),
            Err(e) => log::warn!("Snapshot scheduler: failed to prune {name}: {e}"),
        }
    }
}
