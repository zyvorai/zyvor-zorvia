// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Autoscaler reconcile loop wiring: leader-gated spawn, audit + operations sink.
//! Off unless `ZORVIA_AUTOSCALER_LOOP=1` and `ZORVIA_EXPERIMENTAL=1`; see docs/AUTOSCALER_LOOP.md.

use super::*;
use crate::autoscaler::live::KubeExecutor;
use crate::autoscaler::reconcile::{
    parse_targets, Direction, EventSink, LoopConfig, Mode, Reconciler, ScaleEvent,
};
use crate::autoscaler::telemetry::TelemetryLoadSource;
use crate::operations::{NewOperation, OperationsDb};

struct AuditOpsSink {
    audit: SharedAuditTrail,
}

impl EventSink for AuditOpsSink {
    async fn record(&self, ev: &ScaleEvent) {
        let details = serde_json::json!({
            "target": ev.target_id, "from": ev.from, "to": ev.to,
            "load_percent": ev.load_percent, "mode": ev.mode, "detail": ev.detail,
        });
        // Operations record only for real attempts; recommendations are audit-only.
        if matches!(ev.mode, Mode::Applied | Mode::Failed) {
            let mut new = NewOperation::new("autoscale", &ev.name, &ev.namespace);
            new.params = details.clone();
            new.max_attempts = 1;
            new.owner = Some("system:autoscaler".into());
            let db = OperationsDb::global();
            if let Ok((op, true)) = db.create(new) {
                let _ = db.start(&op.id);
                let _ = if ev.mode == Mode::Applied {
                    db.succeed(&op.id, Some(&details))
                } else {
                    db.fail(&op.id, &ev.detail, false).map(|_| ())
                };
            }
        }
        let entry = crate::audit_trail::AuditEntry {
            id: crate::utils::generate_id("audit", &ev.name),
            timestamp: chrono::Utc::now(),
            user: "system:autoscaler".into(),
            action: match ev.direction {
                Direction::Up => crate::audit_trail::AuditAction::ScaleUp,
                Direction::Down => crate::audit_trail::AuditAction::ScaleDown,
            },
            resource_type: "autoscale".into(),
            resource_name: ev.name.clone(),
            namespace: ev.namespace.clone(),
            details,
            ip_address: String::new(),
            success: ev.mode != Mode::Failed,
            severity: crate::audit_trail::AuditSeverity::Medium,
        };
        self.audit.write().await.record(entry);
    }
}

pub fn spawn_autoscaler_loop(state: SharedState) {
    let Some(cfg) = LoopConfig::from_env() else {
        return;
    };
    let Ok(path) = std::env::var("ZORVIA_AUTOSCALER_TARGETS") else {
        log::warn!(
            "autoscaler loop enabled but ZORVIA_AUTOSCALER_TARGETS is not set; not starting"
        );
        return;
    };
    let targets = match std::fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|s| parse_targets(&s))
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("autoscaler targets invalid ({e}); loop not started");
            return;
        }
    };
    log::info!(
        "autoscaler loop starting: {} target(s), {}",
        targets.len(),
        if cfg.apply {
            "APPLY mode"
        } else {
            "recommendation-only"
        }
    );
    tokio::spawn(async move {
        let (kube, audit) = {
            let s = state.read().await;
            (s.kube_client.clone(), s.audit.clone())
        };
        let load = TelemetryLoadSource::from_env(&kube);
        let exec = KubeExecutor { kube };
        let sink = AuditOpsSink { audit };
        let mut rec = Reconciler::default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(cfg.interval_secs)).await;
            let leader = !crate::api::leader::skip_if_follower("autoscaler");
            rec.tick(
                chrono::Utc::now(),
                leader,
                &cfg,
                &targets,
                &load,
                &exec,
                &sink,
            )
            .await;
        }
    });
}
