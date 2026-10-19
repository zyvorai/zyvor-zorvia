//! AI-assisted VM diagnosis. Gathers real evidence (VM/VMI status, guest
//! insight, Kubernetes events, recent launcher logs), redacts it, and asks the
//! configured model for a diagnosis. Read-only: the model has no tools and the
//! response carries the exact redacted evidence that was sent.
//!
//! Gated like `/vms/{name}/logs` (`vm.power`) because launcher logs are part
//! of the evidence -- a Viewer must not read them through the model.

use super::*;
use serde_json::json;

const LOG_TAIL_LINES: i64 = 80;
const MAX_EVENTS: usize = 40;

pub async fn ai_diagnose_vm_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    use crate::ai::{self, AiError};

    let cfg = match ai::AiConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            let (st, j) = err_json(503, "AI_NOT_CONFIGURED", &e.to_string());
            return (st, j).into_response();
        }
    };

    let (namespace, client, kube_client) = {
        let s = state.read().await;
        (s.namespace.clone(), s.client(), s.kube_client.client())
    };

    let vm = match client.get_vm(&namespace, &name).await {
        Ok(vm) => vm,
        Err(e) => {
            let (st, j) = err_json(404, "VM_NOT_FOUND", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };
    let vmi_status = client
        .get_vmi(&namespace, &name)
        .await
        .ok()
        .and_then(|vmi| vmi.status);
    let insight = crate::guest_insight::GuestInsightReport::from_status(
        name.clone(),
        namespace.clone(),
        vmi_status.as_ref(),
    );

    let events = vm_events(kube_client.clone(), &namespace, &name).await;
    let logs = launcher_logs(kube_client, &namespace, &name).await;

    let evidence = ai::prepare_evidence(json!({
        "vm_status": vm.status,
        "vmi_status": vmi_status,
        "guest_insight": insight,
        "events": events,
        "launcher_logs": logs,
    }));

    let prompt = ai::troubleshoot_prompt(&name, &evidence);
    match ai::complete(&cfg, ai::TROUBLESHOOT_SYSTEM, &prompt).await {
        Ok(diagnosis) => Json(json!({
            "vm": name,
            "model": cfg.model,
            "diagnosis": diagnosis,
            "evidence_sent": evidence,
        }))
        .into_response(),
        Err(AiError::NotConfigured(m)) => {
            let (st, j) = err_json(503, "AI_NOT_CONFIGURED", &m);
            (st, j).into_response()
        }
        Err(AiError::Upstream(m)) => {
            log::warn!("ai diagnose {name}: {m}");
            let (st, j) = err_json(502, "AI_UPSTREAM_ERROR", "the AI provider request failed");
            (st, j).into_response()
        }
    }
}

async fn vm_events(client: kube::Client, namespace: &str, name: &str) -> Vec<serde_json::Value> {
    use k8s_openapi::api::core::v1::Event;
    use kube::api::{Api, ListParams};

    let api: Api<Event> = Api::namespaced(client, namespace);
    let lp = ListParams::default().fields(&format!("involvedObject.name={name}"));
    let mut items = match api.list(&lp).await {
        Ok(l) => l.items,
        Err(e) => {
            log::debug!("events for {name}: {e}");
            return vec![];
        }
    };
    items.sort_by_key(|e| e.last_timestamp.clone().map(|t| t.0));
    items
        .into_iter()
        .rev()
        .take(MAX_EVENTS)
        .map(|e| {
            json!({
                "type": e.type_,
                "reason": e.reason,
                "message": e.message,
                "count": e.count,
                "object": e.involved_object.kind,
            })
        })
        .collect()
}

async fn launcher_logs(client: kube::Client, namespace: &str, name: &str) -> Vec<serde_json::Value> {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{Api, ListParams, LogParams};

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let lp = ListParams::default().labels(&format!("kubevirt.io/vm={name}"));
    let Ok(list) = pods.list(&lp).await else {
        return vec![];
    };
    let mut out = Vec::new();
    for pod in list.items.into_iter().take(2) {
        let Some(pod_name) = pod.metadata.name else {
            continue;
        };
        let params = LogParams {
            tail_lines: Some(LOG_TAIL_LINES),
            ..Default::default()
        };
        if let Ok(text) = pods.logs(&pod_name, &params).await {
            out.push(json!({ "pod": pod_name, "tail": text }));
        }
    }
    out
}
