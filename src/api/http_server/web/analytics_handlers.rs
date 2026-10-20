//! Real fleet-wide usage ranking, built from the same real per-VM usage
//! numbers `fabric_vm_metrics` already reports for one VM at a time
//! (`quick_vm_usage`, shared with `fabric_vm_handlers.rs`) -- no new metrics
//! source, just aggregated across every VM instead of one.

use super::*;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct TopVmsQuery {
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize {
    10
}

/// Row fields the ranking can sort on. Anything else used to sort as 0.0 silently.
const TOP_VMS_METRICS: [&str; 3] = ["cpu_usage", "memory_usage", "disk_usage"];

fn validate_metric(metric: Option<&str>) -> Result<&str, &'static str> {
    let m = metric.unwrap_or("cpu_usage");
    if TOP_VMS_METRICS.contains(&m) {
        Ok(m)
    } else {
        Err("metric must be one of: cpu_usage, memory_usage, disk_usage")
    }
}

pub async fn analytics_top_vms_handler(
    State(state): State<SharedState>,
    Query(q): Query<TopVmsQuery>,
) -> impl IntoResponse {
    let metric = match validate_metric(q.metric.as_deref()) {
        Ok(m) => m,
        Err(msg) => {
            let (st, j) = err_json(400, "INVALID_METRIC", msg);
            return (st, j).into_response();
        }
    };

    let s = state.read().await;
    let namespace = s.namespace.clone();
    let client = s.client();
    drop(s);

    let vms = match client.list_vms(&namespace).await {
        Ok(v) => v,
        Err(e) => {
            let (st, j) = err_json(500, "ANALYTICS_FAILED", &sanitize_error(&e));
            return (st, j).into_response();
        }
    };

    let mut rows = Vec::with_capacity(vms.len());
    for vm in &vms {
        let name = vm.metadata.name.clone().unwrap_or_default();
        let (cpu_usage, memory_usage, disk_usage, source, _gm) =
            super::fabric_vm_handlers::quick_vm_usage(&client, &namespace, &name).await;
        rows.push(json!({
            "vm_name": name,
            "cpu_usage": cpu_usage,
            "memory_usage": memory_usage,
            "disk_usage": disk_usage,
            "source": source,
        }));
    }

    rows.sort_by(|a, b| {
        let av = a.get(metric).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let bv = b.get(metric).and_then(|v| v.as_f64()).unwrap_or(0.0);
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(q.limit.max(1));

    Json(rows).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_validation() {
        assert_eq!(validate_metric(None), Ok("cpu_usage"));
        assert_eq!(validate_metric(Some("memory_usage")), Ok("memory_usage"));
        assert!(validate_metric(Some("bogus")).is_err());
    }
}
