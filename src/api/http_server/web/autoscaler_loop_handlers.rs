// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Autoscaler reconcile loop wiring: leader-gated spawn, audit + operations sink.
//! Off unless `ZORVIA_AUTOSCALER_LOOP=1` and `ZORVIA_EXPERIMENTAL=1`; see docs/AUTOSCALER_LOOP.md.

use super::*;
use crate::autoscaler::decisions::{self, DecisionLog};
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
    decisions::global().start(&targets, cfg.apply, cfg.interval_secs);
    tokio::spawn(async move {
        let (kube, audit) = {
            let s = state.read().await;
            (s.kube_client.clone(), s.audit.clone())
        };
        let load = TelemetryLoadSource::from_env(&kube);
        let exec = KubeExecutor { kube };
        let sink = AuditOpsSink { audit };
        let mut rec = Reconciler::with_log(decisions::global().clone());
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

// ---------------------------------------------------------------- read-only API

const DISABLED_MSG: &str =
    "The autoscaler loop view is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable";

/// Why the loop is not running, for the disabled state (no secrets: only variable names).
fn not_running_hint(experimental: bool) -> &'static str {
    if !experimental {
        "ZORVIA_EXPERIMENTAL=1 is not set"
    } else {
        "the loop is not running: set ZORVIA_AUTOSCALER_LOOP=1 and ZORVIA_AUTOSCALER_TARGETS and restart"
    }
}

/// `GET /v1/autoscaler/status`: loop settings, targets and their hysteresis state.
pub(crate) fn autoscaler_status_view(
    experimental: bool,
    log: &DecisionLog,
) -> (u16, serde_json::Value) {
    if !experimental {
        return (403, serde_json::json!({"gate": true}));
    }
    let (status, targets) = log.status();
    let hint = if status.running {
        serde_json::Value::Null
    } else {
        not_running_hint(experimental).into()
    };
    (
        200,
        serde_json::json!({
            "success": true,
            "data": {
                "experimental": true,
                "enabled": status.running,
                "disabled_reason": hint,
                "status": status,
                "targets": targets,
            }
        }),
    )
}

#[derive(Debug, Deserialize)]
pub struct DecisionsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub target: Option<String>,
}

/// `GET /v1/autoscaler/decisions?limit=&target=`: newest first.
pub(crate) fn autoscaler_decisions_view(
    experimental: bool,
    log: &DecisionLog,
    q: &DecisionsQuery,
) -> (u16, serde_json::Value) {
    if !experimental {
        return (403, serde_json::json!({"gate": true}));
    }
    let limit = match q.limit {
        None => decisions::DEFAULT_LIMIT,
        Some(l) if (1..=decisions::MAX_LIMIT).contains(&l) => l,
        Some(_) => {
            return (
                400,
                serde_json::json!({"code": "INVALID_LIMIT",
                    "message": format!("limit must be 1..={}", decisions::MAX_LIMIT)}),
            )
        }
    };
    if let Some(t) = &q.target {
        let ok = !t.is_empty()
            && t.len() <= 63
            && t.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return (
                400,
                serde_json::json!({"code": "INVALID_TARGET", "message": "target must be a DNS-1123 label"}),
            );
        }
        if !log.knows_target(t) {
            return (
                404,
                serde_json::json!({"code": "NOT_FOUND", "message": "No such autoscaler target"}),
            );
        }
    }
    let records = log.recent(limit, q.target.as_deref());
    let (status, _) = log.status();
    (
        200,
        serde_json::json!({
            "success": true,
            "data": {
                "experimental": true,
                "enabled": status.running,
                "disabled_reason": if status.running { serde_json::Value::Null } else { not_running_hint(true).into() },
                "decisions": records,
            }
        }),
    )
}

fn render_view(v: (u16, serde_json::Value)) -> axum::response::Response {
    let (status, body) = v;
    match status {
        200 => Json(body).into_response(),
        403 => {
            let (st, j) = err_json(403, "EXPERIMENTAL_DISABLED", DISABLED_MSG);
            (st, j).into_response()
        }
        code => {
            let c = body["code"].as_str().unwrap_or("ERROR").to_string();
            let m = body["message"].as_str().unwrap_or("error").to_string();
            let (st, j) = err_json(code, &c, &m);
            (st, j).into_response()
        }
    }
}

pub async fn autoscaler_status_handler() -> impl IntoResponse {
    render_view(autoscaler_status_view(
        crate::features::experimental_enabled(),
        decisions::global(),
    ))
}

pub async fn autoscaler_decisions_handler(
    axum::extract::Query(q): axum::extract::Query<DecisionsQuery>,
) -> impl IntoResponse {
    render_view(autoscaler_decisions_view(
        crate::features::experimental_enabled(),
        decisions::global(),
        &q,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoscaler::decisions::{DecisionKind, Observation, Outcome};
    use crate::autoscaler::reconcile::{LoadMetric, LoopTarget, TargetKind};

    fn target() -> LoopTarget {
        LoopTarget {
            id: "web".into(),
            kind: TargetKind::VmPool,
            namespace: "prod".into(),
            name: "web-pool".into(),
            metric_vms: vec!["web-0".into()],
            load_metric: LoadMetric::Cpu,
            min: 1,
            max: 5,
            step: 1,
            scale_up_threshold: 80.0,
            scale_down_threshold: 30.0,
            required_samples: 3,
            cooldown_secs: 300,
        }
    }

    fn q(limit: Option<usize>, target: Option<&str>) -> DecisionsQuery {
        DecisionsQuery {
            limit,
            target: target.map(String::from),
        }
    }

    fn seeded() -> DecisionLog {
        let log = DecisionLog::default();
        let t = target();
        log.start(std::slice::from_ref(&t), false, 60);
        for i in 0..3 {
            log.push(Observation {
                now: chrono::Utc::now(),
                target: &t,
                load_percent: Some(90.0 + i as f64),
                source: "metrics_api",
                decision: DecisionKind::ScaleUp,
                outcome: Outcome::Recommended,
                from: Some(2),
                to: Some(3),
                reason: &format!("r{i}"),
            });
        }
        log
    }

    #[test]
    fn gated_without_experimental() {
        let log = seeded();
        assert_eq!(autoscaler_status_view(false, &log).0, 403);
        assert_eq!(
            autoscaler_decisions_view(false, &log, &q(None, None)).0,
            403
        );
    }

    #[test]
    fn status_lists_targets_and_mode() {
        let (code, v) = autoscaler_status_view(true, &seeded());
        assert_eq!(code, 200);
        assert_eq!(v["data"]["enabled"], true);
        assert_eq!(v["data"]["status"]["mode"], "recommend_only");
        assert_eq!(v["data"]["targets"][0]["id"], "web");
        // no metric_vms or other target internals beyond the policy
        assert!(v["data"]["targets"][0].get("metric_vms").is_none());
    }

    #[test]
    fn status_reports_disabled_loop() {
        let (code, v) = autoscaler_status_view(true, &DecisionLog::default());
        assert_eq!(code, 200);
        assert_eq!(v["data"]["enabled"], false);
        assert!(v["data"]["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("ZORVIA_AUTOSCALER_LOOP"));
    }

    #[test]
    fn decisions_limit_order_and_filter() {
        let log = seeded();
        let (code, v) = autoscaler_decisions_view(true, &log, &q(Some(2), Some("web")));
        assert_eq!(code, 200);
        let d = v["data"]["decisions"].as_array().unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["reason"], "r2");
        assert_eq!(d[0]["outcome"], "recommended");
        assert_eq!(d[0]["decision"], "scale_up");
        assert_eq!(d[0]["source"], "metrics_api");
        assert_eq!(autoscaler_decisions_view(true, &log, &q(None, None)).0, 200);
    }

    #[test]
    fn decisions_validates_input() {
        let log = seeded();
        for bad in [q(Some(0), None), q(Some(501), None)] {
            assert_eq!(autoscaler_decisions_view(true, &log, &bad).0, 400);
        }
        for bad in ["Web", "a/b", "", "../x"] {
            assert_eq!(
                autoscaler_decisions_view(true, &log, &q(None, Some(bad))).0,
                400,
                "{bad}"
            );
        }
        assert_eq!(
            autoscaler_decisions_view(true, &log, &q(None, Some("other"))).0,
            404
        );
    }

    #[tokio::test]
    async fn http_handlers_answer_403_when_experimental_is_off() {
        if crate::features::experimental_enabled() {
            return;
        }
        use axum::{body::Body, http::Request, routing::get, Router};
        use tower::ServiceExt;
        let app = Router::new()
            .route("/s", get(autoscaler_status_handler))
            .route("/d", get(autoscaler_decisions_handler));
        for p in ["/s", "/d?limit=5"] {
            let r = app
                .clone()
                .oneshot(Request::builder().uri(p).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "{p}");
            let b = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
            assert!(String::from_utf8_lossy(&b).contains("EXPERIMENTAL_DISABLED"));
        }
    }
}
