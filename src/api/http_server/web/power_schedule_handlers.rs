// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Real, disk-persisted VM power scheduling (start/stop/restart on a
//! recurring schedule), reusing `crate::backup::schedule::ScheduleType`'s
//! already-tested next-run math and the same `ScheduleManager`-shaped
//! persistence pattern Phase 5's `BackupScheduler` established --
//! `spawn_power_schedule_loop` actually runs due schedules against the
//! real VM start/stop/restart routes.

use super::*;
use crate::backup::schedule::ScheduleType;
use crate::power_schedule::{PowerAction, PowerSchedule, PowerScheduleManager};
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

fn action_label(a: PowerAction) -> &'static str {
    match a {
        PowerAction::Start => "start",
        PowerAction::Stop => "stop",
        PowerAction::Restart => "restart",
    }
}

fn schedule_json(s: &PowerSchedule) -> serde_json::Value {
    json!({
        "name": s.name,
        "vm_name": s.vm_name,
        "action": action_label(s.action),
        "schedule_type": schedule_type_label(&s.schedule_type),
        "enabled": s.enabled,
        "last_run": s.last_run.map(|t| t.to_rfc3339()),
        "next_run": s.next_run.map(|t| t.to_rfc3339()),
    })
}

fn parse_weekday(s: &str) -> Option<Weekday> {
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

pub async fn list_power_schedules_handler() -> impl IntoResponse {
    let mgr = PowerScheduleManager::load();
    Json(mgr.all().iter().map(schedule_json).collect::<Vec<_>>()).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreatePowerScheduleBody {
    pub name: String,
    pub vm_name: String,
    /// "start" | "stop" | "restart"
    pub action: String,
    /// "hourly" | "daily" | "weekly" | "monthly"
    pub schedule_type: String,
    #[serde(default)]
    pub hour: Option<u32>,
    #[serde(default)]
    pub minute: Option<u32>,
    #[serde(default)]
    pub weekday: Option<String>,
    #[serde(default)]
    pub day_of_month: Option<u32>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn create_power_schedule_handler(
    AxumJson(body): AxumJson<CreatePowerScheduleBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() || body.vm_name.trim().is_empty() {
        let (st, j) = err_json(400, "INVALID_NAME", "name and vm_name are required");
        return (st, j).into_response();
    }
    let action = match body.action.as_str() {
        "start" => PowerAction::Start,
        "stop" => PowerAction::Stop,
        "restart" => PowerAction::Restart,
        other => {
            let (st, j) = err_json(
                400,
                "INVALID_ACTION",
                &format!("Unknown action '{other}' (use start, stop, or restart)"),
            );
            return (st, j).into_response();
        }
    };

    let hour = body.hour.unwrap_or(8).min(23);
    let minute = body.minute.unwrap_or(0).min(59);
    let schedule_type = match body.schedule_type.as_str() {
        "hourly" => ScheduleType::hourly(minute),
        "daily" => ScheduleType::daily(hour, minute),
        "weekly" => {
            let Some(weekday) = body.weekday.as_deref().and_then(parse_weekday) else {
                let (st, j) = err_json(
                    400,
                    "INVALID_WEEKDAY",
                    "weekly schedules require a valid `weekday` (e.g. \"monday\")",
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

    let mut mgr = PowerScheduleManager::load();
    if mgr.get_schedule(&body.name).is_some() {
        let (st, j) = err_json(409, "CONFLICT", "A schedule with this name already exists");
        return (st, j).into_response();
    }

    let mut schedule = PowerSchedule::new(&body.name, &body.vm_name, action, schedule_type);
    if body.enabled == Some(false) {
        schedule.enabled = false;
    }
    schedule.next_run = schedule.calculate_next_run(Utc::now());

    let response = schedule_json(&schedule);
    mgr.add_schedule(schedule);
    (StatusCode::CREATED, Json(response)).into_response()
}

pub async fn delete_power_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    let mut mgr = PowerScheduleManager::load();
    if mgr.remove_schedule(&name) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such power schedule");
        (st, j).into_response()
    }
}

async fn set_schedule_enabled(name: &str, enabled: bool) -> impl IntoResponse {
    let mut mgr = PowerScheduleManager::load();
    let Some(existing) = mgr.get_schedule(name).cloned() else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such power schedule");
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
    let response = schedule_json(&updated);
    mgr.add_schedule(updated);
    Json(response).into_response()
}

pub async fn enable_power_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_schedule_enabled(&name, true).await
}

pub async fn disable_power_schedule_handler(Path(name): Path<String>) -> impl IntoResponse {
    set_schedule_enabled(&name, false).await
}

/// Spawns the loop that actually executes due power schedules. Checks
/// every 60s, same cadence and rationale as `spawn_backup_scheduler_loop`.
pub fn spawn_power_schedule_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if crate::api::leader::skip_if_follower("power-scheduler") {
                continue;
            }
            let (namespace, client) = {
                let s = state.read().await;
                (s.namespace.clone(), s.client())
            };
            run_due_power_schedules(&namespace, &client).await;
        }
    });
}

async fn run_due_power_schedules(namespace: &str, client: &crate::kube::KubeClient) {
    let mgr = PowerScheduleManager::load();
    let now = Utc::now();
    let due: Vec<PowerSchedule> = mgr.get_due_schedules(now).into_iter().cloned().collect();
    if due.is_empty() {
        return;
    }

    for schedule in due {
        let result = match schedule.action {
            PowerAction::Start => client.start_vm(namespace, &schedule.vm_name).await,
            PowerAction::Stop => client.stop_vm(namespace, &schedule.vm_name).await,
            PowerAction::Restart => client.restart_vm(namespace, &schedule.vm_name).await,
        };
        match result {
            Ok(_) => log::info!(
                "Power scheduler: {} VM '{}' (schedule '{}')",
                action_label(schedule.action),
                schedule.vm_name,
                schedule.name
            ),
            Err(e) => log::error!(
                "Power scheduler: failed to {} VM '{}' (schedule '{}'): {e}",
                action_label(schedule.action),
                schedule.vm_name,
                schedule.name
            ),
        }

        let mut mgr = PowerScheduleManager::load();
        if let Some(current) = mgr.get_schedule(&schedule.name).cloned() {
            let mut updated = current;
            updated.last_run = Some(now);
            updated.next_run = updated.calculate_next_run(now);
            mgr.remove_schedule(&schedule.name);
            mgr.add_schedule(updated);
        }
    }
}
