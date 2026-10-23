//! Real, disk-persisted backup scheduling, backed by
//! `crate::backup::schedule::ScheduleManager` -- a fully real, tested
//! struct (JSON-persisted to `$XDG_DATA_HOME/zorvia/backup_schedules.json`,
//! real next-run math via `crate::utils::schedule`) that had zero callers
//! anywhere in the codebase until this. `spawn_backup_scheduler_loop`
//! queues one durable backup operation per VM of a due schedule (see
//! `crate::backup::op`); the operations reconciler runs them and reports
//! success only once the VirtualMachineSnapshot has actually succeeded.

use super::*;
use crate::backup::schedule::{BackupSchedule, ScheduleManager, ScheduleType, VMSelector};
use axum::extract::Json as AxumJson;
use chrono::{Utc, Weekday};
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

fn policy_json(s: &BackupSchedule) -> serde_json::Value {
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
        "backup_type": s.backup_type,
        "retention_days": s.retention_days,
        "enabled": s.enabled,
        "last_run": s.last_run.map(|t| t.to_rfc3339()),
        "next_run": s.next_run.map(|t| t.to_rfc3339()),
    })
}

pub fn parse_weekday(s: &str) -> Option<Weekday> {
    use Weekday::*;
    Some(match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Mon,
        "tue" | "tuesday" => Tue,
        "wed" | "wednesday" => Wed,
        "thu" | "thursday" => Thu,
        "fri" | "friday" => Fri,
        "sat" | "saturday" => Sat,
        "sun" | "sunday" => Sun,
        _ => return None,
    })
}

pub async fn list_backup_policies_handler() -> impl IntoResponse {
    let mgr = ScheduleManager::load();
    Json(mgr.all().iter().map(policy_json).collect::<Vec<_>>()).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreatePolicyBody {
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
    /// Backs up every VM in the namespace when omitted/empty.
    #[serde(default)]
    pub vm_names: Option<Vec<String>>,
    #[serde(default)]
    pub backup_type: Option<String>,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn create_backup_policy_handler(
    AxumJson(body): AxumJson<CreatePolicyBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "Policy name is required");
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

    let mut mgr = ScheduleManager::load();
    if mgr.get_schedule(&body.name).is_some() {
        let (st, j) = err_json(409, "CONFLICT", "A policy with this name already exists");
        return (st, j).into_response();
    }

    let selector = match body.vm_names.filter(|v| !v.is_empty()) {
        Some(names) => VMSelector::ByName(names),
        None => VMSelector::All,
    };
    let mut schedule = BackupSchedule::new(&body.name, schedule_type)
        .with_selector(selector)
        .with_backup_type(body.backup_type.unwrap_or_else(|| "full".to_string()));
    if let Some(days) = body.retention_days {
        schedule = schedule.with_retention_days(days);
    }
    if body.enabled == Some(false) {
        schedule = schedule.disable();
    }
    schedule.next_run = schedule.calculate_next_run(Utc::now());

    let response = policy_json(&schedule);
    mgr.add_schedule(schedule);
    (StatusCode::CREATED, Json(response)).into_response()
}

pub async fn delete_backup_policy_handler(Path(name): Path<String>) -> impl IntoResponse {
    let mut mgr = ScheduleManager::load();
    if mgr.remove_schedule(&name) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such backup policy");
        (st, j).into_response()
    }
}

async fn set_policy_enabled(name: &str, enabled: bool) -> impl IntoResponse {
    let mut mgr = ScheduleManager::load();
    let Some(existing) = mgr.get_schedule(name).cloned() else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such backup policy");
        return (st, j).into_response();
    };
    let mut updated = existing;
    updated.enabled = enabled;
    updated.next_run = if enabled {
        updated.calculate_next_run(Utc::now())
    } else {
        None
    };
    mgr.remove_schedule(name);
    let response = policy_json(&updated);
    mgr.add_schedule(updated);
    Json(response).into_response()
}

pub async fn enable_backup_policy_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_policy_enabled(&name, true).await
}

pub async fn disable_backup_policy_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_policy_enabled(&name, false).await
}

/// Spawns the loop that actually executes due schedules. Checks every 60s
/// -- coarse, but every schedule granularity this supports (hourly and up)
/// tolerates a bounded delay of that size.
pub fn spawn_backup_scheduler_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if crate::api::leader::skip_if_follower("backup-scheduler") {
                continue;
            }
            let namespace = state.read().await.namespace.clone();
            run_due_schedules(&namespace).await;
        }
    });
}

async fn run_due_schedules(namespace: &str) {
    let mgr = ScheduleManager::load();
    let now = Utc::now();
    let due: Vec<BackupSchedule> = mgr.get_due_schedules(now).into_iter().cloned().collect();
    if due.is_empty() {
        return;
    }

    let client = match crate::kube::KubeClient::new().await {
        Ok(c) => c,
        Err(e) => {
            log::error!("Backup scheduler: failed to connect to cluster: {e}");
            return;
        }
    };

    for schedule in due {
        let vm_names: Vec<String> = match &schedule.vm_selector {
            VMSelector::All => match client.list_vms(namespace).await {
                Ok(vms) => vms.into_iter().filter_map(|vm| vm.metadata.name).collect(),
                Err(e) => {
                    log::error!(
                        "Backup scheduler: failed to list VMs for schedule '{}': {e}",
                        schedule.name
                    );
                    continue;
                }
            },
            VMSelector::ByName(names) => names.clone(),
            VMSelector::ByLabel { .. } => {
                log::warn!(
                    "Backup scheduler: schedule '{}' uses a label selector, which isn't wired to real VM label lookup yet -- skipping this run",
                    schedule.name
                );
                continue;
            }
        };

        // Queue one durable backup per VM. The idempotency key includes the
        // due slot, so if Zorvia dies before the schedule advances below,
        // the next tick re-enqueues the same keys and nothing runs twice.
        let slot = schedule
            .next_run
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| now.to_rfc3339());
        let mut all_queued = true;
        for vm_name in &vm_names {
            let key = format!("sched:{}:{slot}:{vm_name}", schedule.name);
            match crate::backup::op::enqueue(
                namespace,
                vm_name,
                &schedule.backup_type,
                schedule.retention_days,
                Some(format!("scheduled backup ({})", schedule.name)),
                &key,
            ) {
                Ok((op, true)) => log::info!(
                    "Backup scheduler: queued backup {} for VM '{vm_name}' (schedule '{}')",
                    op.id,
                    schedule.name
                ),
                Ok((_, false)) => {}
                Err(e) => {
                    all_queued = false;
                    log::error!(
                        "Backup scheduler: failed to queue backup for VM '{vm_name}' (schedule '{}'): {e}",
                        schedule.name
                    );
                }
            }
        }
        if !all_queued {
            continue;
        }

        let mut mgr = ScheduleManager::load();
        if let Some(current) = mgr.get_schedule(&schedule.name).cloned() {
            let mut updated = current;
            updated.last_run = Some(now);
            updated.next_run = updated.calculate_next_run(now);
            mgr.remove_schedule(&schedule.name);
            mgr.add_schedule(updated);
        }
    }
}
