// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real webhook registration + delivery, backed by
//! `crate::api::webhooks::WebhookManager` -- a real, well-built module
//! (SSRF-guarded URL validation, retry/backoff, delivery-stats tracking)
//! that had persistence and actual HTTP delivery added alongside this file
//! (it previously only tracked config in memory with no way to send
//! anything). `dispatch_webhook_event` is called from the same VM
//! lifecycle handlers `record_audit` already instruments.

use super::*;
use crate::api::webhooks::{WebhookConfig, WebhookEvent, WebhookManager, WebhookPayload};
use axum::extract::Json as AxumJson;
use serde_json::json;

fn webhook_json(w: &WebhookConfig) -> serde_json::Value {
    json!({
        "id": w.id,
        "name": w.name,
        "url": w.url,
        "events": w.events.iter().map(|e| e.to_string()).collect::<Vec<_>>(),
        "enabled": w.enabled,
        "retry_count": w.retry_count,
        "retry_delay_secs": w.retry_delay_secs,
        "timeout_secs": w.timeout_secs,
        "created_at": w.created_at.to_rfc3339(),
        "last_triggered": w.last_triggered.map(|t| t.to_rfc3339()),
        "delivery_count": w.delivery_count,
        "failure_count": w.failure_count,
        "success_rate": w.success_rate(),
    })
}

fn parse_event(s: &str) -> WebhookEvent {
    match s {
        "vm.created" => WebhookEvent::VMCreated,
        "vm.deleted" => WebhookEvent::VMDeleted,
        "vm.started" => WebhookEvent::VMStarted,
        "vm.stopped" => WebhookEvent::VMStopped,
        "vm.restarted" => WebhookEvent::VMRestarted,
        "vm.failed" => WebhookEvent::VMFailed,
        "vm.health_changed" => WebhookEvent::VMHealthChanged,
        "snapshot.created" => WebhookEvent::SnapshotCreated,
        "snapshot.restored" => WebhookEvent::SnapshotRestored,
        "backup.completed" => WebhookEvent::BackupCompleted,
        "backup.failed" => WebhookEvent::BackupFailed,
        "migration.started" => WebhookEvent::MigrationStarted,
        "migration.completed" => WebhookEvent::MigrationCompleted,
        "alert.triggered" => WebhookEvent::AlertTriggered,
        "alert.resolved" => WebhookEvent::AlertResolved,
        other => WebhookEvent::Custom(other.trim_start_matches("custom.").to_string()),
    }
}

pub async fn list_webhooks_handler() -> impl IntoResponse {
    let mgr = WebhookManager::load();
    Json(
        mgr.list()
            .iter()
            .map(|w| webhook_json(w))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookBody {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub retry_count: Option<u32>,
    #[serde(default)]
    pub retry_delay_secs: Option<u64>,
}

pub async fn create_webhook_handler(
    AxumJson(body): AxumJson<CreateWebhookBody>,
) -> impl IntoResponse {
    let mut config = match WebhookConfig::new(&body.name, &body.url) {
        Ok(c) => c,
        Err(e) => {
            let (st, j) = err_json(400, "INVALID_WEBHOOK", &e.to_string());
            return (st, j).into_response();
        }
    };
    for event in &body.events {
        config.add_event(parse_event(event));
    }
    if let Some(secret) = body.secret {
        config = config.with_secret(secret);
    }
    if let (Some(count), Some(delay)) = (body.retry_count, body.retry_delay_secs) {
        config = config.with_retry(count, delay);
    }

    let mut mgr = WebhookManager::load();
    mgr.register(config.clone());
    if let Err(e) = mgr.save() {
        let (st, j) = err_json(500, "PERSIST_FAILED", &e.to_string());
        return (st, j).into_response();
    }
    (StatusCode::CREATED, Json(webhook_json(&config))).into_response()
}

pub async fn delete_webhook_handler(Path(id): Path<String>) -> impl IntoResponse {
    let mut mgr = WebhookManager::load();
    if mgr.unregister(&id) {
        let _ = mgr.save();
        StatusCode::NO_CONTENT.into_response()
    } else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such webhook");
        (st, j).into_response()
    }
}

pub async fn test_webhook_handler(Path(id): Path<String>) -> impl IntoResponse {
    let mut mgr = WebhookManager::load();
    let Some(config) = mgr.get(&id).cloned() else {
        let (st, j) = err_json(404, "NOT_FOUND", "No such webhook");
        return (st, j).into_response();
    };

    let mut payload = WebhookPayload::new(&WebhookEvent::Custom("test".to_string()));
    payload.add_data("message", "This is a test delivery from Zorvia");
    // Deliver directly regardless of the webhook's subscribed events -- a
    // test button should always fire.
    let success = crate::api::webhooks::deliver_once(&config, &payload)
        .await
        .is_ok();
    if let Some(w) = mgr.get_mut(&id) {
        w.record_delivery(success);
    }
    let _ = mgr.save();

    Json(json!({ "delivered": true, "success": success })).into_response()
}

/// Fires a real webhook delivery for `event` -- reads and re-saves the
/// persisted `WebhookManager` (delivery stats change), same pattern as
/// `record_audit`. Only mutating VM lifecycle actions call this today.
pub(crate) async fn dispatch_webhook_event(event: WebhookEvent, vm_name: &str, success: bool) {
    let mut mgr = WebhookManager::load();
    let actual_event = if success {
        event
    } else {
        WebhookEvent::VMFailed
    };
    let mut payload = WebhookPayload::new(&actual_event);
    payload.add_data("vm_name", vm_name);
    mgr.dispatch(&actual_event, &payload).await;
}
