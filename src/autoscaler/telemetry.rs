// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Real load sources for the autoscaler reconcile loop.
//!
//! 1. **Kubernetes metrics API** (`metrics.k8s.io/v1beta1` `PodMetrics`) for each VM's
//!    virt-launcher pod, mapped to the VM's CPU/memory size.
//! 2. **Prometheus** (optional, `ZORVIA_AUTOSCALER_PROMETHEUS_URL`) with KubeVirt `kubevirt_vmi_*`
//!    metrics, used only when the metrics API gave no usable answer.
//!
//!    If the Prometheus endpoint needs authentication, `ZORVIA_AUTOSCALER_PROMETHEUS_TOKEN_FILE`
//!    names a file holding a bearer token. It is read on every request (so rotation needs no
//!    restart), only ever sent over https, never logged, and never accepted from a plain env var.
//!
//! A sample from here is always `simulated: false`. Missing, stale, ambiguous or malformed
//! data is an `Err`, which the reconciler treats as "no decision" for that tick (it never
//! scales on it). Not verified against a live cluster; see docs/AUTOSCALER_LOOP.md.

use super::reconcile::{LoadMetric, LoadSample, LoadSource, LoopTarget};
use crate::kube::KubeClient;
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration, TimeZone, Utc};
use kube::api::{Api, DynamicObject, ListParams};
use kube::core::{ApiResource, GroupVersionKind};
use serde_json::Value;

pub const DEFAULT_MAX_AGE_SECS: i64 = 120;
const MIN_MAX_AGE_SECS: i64 = 30;
const MAX_MAX_AGE_SECS: i64 = 900;
/// Prometheus response body cap.
pub const PROM_MAX_BODY: usize = 1 << 20;
const PROM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const PROM_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// A sample stamped this far in the future is treated as malformed (clock skew guard).
const MAX_FUTURE_SKEW_SECS: i64 = 60;
/// Anything above this is not a plausible utilisation percent.
const MAX_PERCENT: f64 = 1000.0;

/// Size of a VM, used to turn absolute usage into a percentage.
#[derive(Debug, Clone, Copy)]
pub struct VmSize {
    pub vcpus: f64,
    pub memory_bytes: f64,
}

#[allow(async_fn_in_trait)]
pub trait VmSizing {
    async fn size(&self, namespace: &str, vm: &str) -> Result<VmSize>;
}

/// Reads sizes from the VM spec (CPU topology, guest memory, or requests).
pub struct KubeSizing(pub KubeClient);

impl VmSizing for KubeSizing {
    async fn size(&self, namespace: &str, vm: &str) -> Result<VmSize> {
        let vm = self.0.get_vm(namespace, vm).await?;
        let d = &vm.spec.template.spec.domain;
        let req = d.resources.requests.as_ref();
        let vcpus = match d.cpu.as_ref() {
            Some(c) => {
                (c.sockets.unwrap_or(1).max(1)
                    * c.cores.unwrap_or(1).max(1)
                    * c.threads.unwrap_or(1).max(1)) as f64
            }
            None => req
                .and_then(|r| r.get("cpu"))
                .and_then(|q| parse_cpu_cores(q))
                .ok_or_else(|| anyhow!("VM has no CPU size"))?,
        };
        let mem = d
            .memory
            .as_ref()
            .and_then(|m| m.guest.as_deref())
            .or_else(|| req.and_then(|r| r.get("memory")).map(String::as_str))
            .and_then(parse_bytes)
            .ok_or_else(|| anyhow!("VM has no memory size"))?;
        Ok(VmSize {
            vcpus,
            memory_bytes: mem,
        })
    }
}

/// CPU quantity (`123456n`, `250u`, `12m`, `2`) in cores.
pub fn parse_cpu_cores(q: &str) -> Option<f64> {
    let q = q.trim();
    let (num, div) = if let Some(n) = q.strip_suffix('n') {
        (n, 1e9)
    } else if let Some(n) = q.strip_suffix('u') {
        (n, 1e6)
    } else if let Some(n) = q.strip_suffix('m') {
        (n, 1e3)
    } else {
        (q, 1.0)
    };
    let v = num.parse::<f64>().ok()? / div;
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// Memory quantity (`1234Ki`, `12Mi`, `2G`, plain bytes) in bytes.
pub fn parse_bytes(q: &str) -> Option<f64> {
    let q = q.trim();
    const SUFFIXES: [(&str, f64); 10] = [
        ("Ki", 1024.0),
        ("Mi", 1048576.0),
        ("Gi", 1073741824.0),
        ("Ti", 1099511627776.0),
        ("Pi", 1125899906842624.0),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
    ];
    let (num, mult) = SUFFIXES
        .iter()
        .find_map(|(s, m)| q.strip_suffix(s).map(|n| (n, *m)))
        .unwrap_or((q, 1.0));
    let v = num.parse::<f64>().ok()? * mult;
    (v.is_finite() && v >= 0.0).then_some(v)
}

fn percent(used: f64, capacity: f64) -> Result<f64> {
    if !capacity.is_finite() || capacity <= 0.0 {
        bail!("VM size is zero or invalid");
    }
    check_percent(used / capacity * 100.0)
}

fn check_percent(p: f64) -> Result<f64> {
    if !p.is_finite() || !(0.0..=MAX_PERCENT).contains(&p) {
        bail!("utilisation out of range");
    }
    Ok(p.min(100.0))
}

fn check_age(ts: DateTime<Utc>, now: DateTime<Utc>, max_age: Duration) -> Result<()> {
    let age = now - ts;
    if age > max_age {
        bail!("stale sample ({}s old)", age.num_seconds());
    }
    if -age > Duration::seconds(MAX_FUTURE_SKEW_SECS) {
        bail!("sample timestamp is in the future");
    }
    Ok(())
}

// ---------------------------------------------------------------- metrics.k8s.io

/// Utilisation percent from one `PodMetrics` object (the `compute` container is the
/// virt-launcher process that runs the guest).
pub fn pod_metrics_percent(
    item: &Value,
    metric: LoadMetric,
    size: VmSize,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Result<f64> {
    let ts = item
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .ok_or_else(|| anyhow!("PodMetrics has no valid timestamp"))?
        .with_timezone(&Utc);
    check_age(ts, now, max_age)?;
    let usage = item
        .get("containers")
        .and_then(Value::as_array)
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("name").and_then(Value::as_str) == Some("compute"))
        })
        .and_then(|c| c.get("usage"))
        .ok_or_else(|| anyhow!("PodMetrics has no compute container usage"))?;
    let field = match metric {
        LoadMetric::Cpu => "cpu",
        LoadMetric::Memory => "memory",
    };
    let q = usage
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("PodMetrics usage has no {field}"))?;
    match metric {
        LoadMetric::Cpu => percent(
            parse_cpu_cores(q).ok_or_else(|| anyhow!("bad cpu quantity"))?,
            size.vcpus,
        ),
        LoadMetric::Memory => percent(
            parse_bytes(q).ok_or_else(|| anyhow!("bad memory quantity"))?,
            size.memory_bytes,
        ),
    }
}

async fn launcher_metrics(client: &kube::Client, ns: &str, vm: &str) -> Result<Vec<Value>> {
    let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
    let ar = ApiResource::from_gvk_with_plural(&gvk, "pods");
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar);
    let lp = ListParams::default()
        .labels(&format!(
            "kubevirt.io=virt-launcher,vm.kubevirt.io/name={vm}"
        ))
        .limit(10);
    let list = api
        .list(&lp)
        .await
        .map_err(|e| anyhow!("metrics API unavailable: {}", first_line(&e.to_string())))?;
    list.items
        .iter()
        .map(|o| serde_json::to_value(o).map_err(Into::into))
        .collect()
}

fn first_line(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or("error")
        .chars()
        .take(200)
        .collect()
}

fn vms_of(t: &LoopTarget) -> Vec<&str> {
    if t.metric_vms.is_empty() {
        vec![t.name.as_str()]
    } else {
        t.metric_vms.iter().map(String::as_str).collect()
    }
}

async fn metrics_api_load<Z: VmSizing>(
    client: &kube::Client,
    sizing: &Z,
    t: &LoopTarget,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Result<f64> {
    let vms = vms_of(t);
    let mut sum = 0.0;
    for vm in &vms {
        let size = sizing.size(&t.namespace, vm).await?;
        let items = launcher_metrics(client, &t.namespace, vm).await?;
        let item = match items.as_slice() {
            [] => bail!("no PodMetrics for VM '{vm}'"),
            [one] => one,
            _ => bail!("ambiguous: several virt-launcher pods for VM '{vm}'"),
        };
        sum += pod_metrics_percent(item, t.load_metric, size, now, max_age)
            .map_err(|e| anyhow!("VM '{vm}': {e}"))?;
    }
    Ok(sum / vms.len() as f64)
}

// ------------------------------------------------------------------- Prometheus

fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `https` is required, except `http` to a loopback host. No credentials in the URL.
pub fn validate_prometheus_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| anyhow!("invalid Prometheus URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("Prometheus URL has no host"))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(host) => {}
        _ => bail!("Prometheus URL must be https (http only for loopback)"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Prometheus URL must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("Prometheus URL must not contain a query or fragment");
    }
    Ok(url)
}

fn safe_label_value(s: &str) -> Result<&str> {
    if s.is_empty()
        || s.len() > 253
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
    {
        bail!("invalid namespace or VM name for a query");
    }
    Ok(s)
}

/// PromQL for one VM's utilisation percent (KubeVirt `kubevirt_vmi_*` metrics).
pub fn prom_query(metric: LoadMetric, ns: &str, vm: &str, size: VmSize) -> Result<String> {
    let sel = format!(
        "namespace=\"{}\",name=\"{}\"",
        safe_label_value(ns)?,
        safe_label_value(vm)?
    );
    Ok(match metric {
        LoadMetric::Cpu => {
            if !size.vcpus.is_finite() || size.vcpus <= 0.0 {
                bail!("VM size is zero or invalid");
            }
            format!(
                "100 * sum(rate(kubevirt_vmi_cpu_usage_seconds_total{{{sel}}}[2m])) / {}",
                size.vcpus
            )
        }
        LoadMetric::Memory => format!(
            "100 * (1 - sum(kubevirt_vmi_memory_usable_bytes{{{sel}}}) / sum(kubevirt_vmi_memory_available_bytes{{{sel}}}))"
        ),
    })
}

/// Parse a Prometheus instant-query response holding exactly one fresh, finite sample.
pub fn parse_prom_response(body: &[u8], now: DateTime<Utc>, max_age: Duration) -> Result<f64> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| anyhow!("malformed Prometheus JSON"))?;
    if v.get("status").and_then(Value::as_str) != Some("success") {
        bail!("Prometheus query did not succeed");
    }
    if v.pointer("/data/resultType").and_then(Value::as_str) != Some("vector") {
        bail!("Prometheus result is not a vector");
    }
    let series = v
        .pointer("/data/result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("malformed Prometheus result"))?;
    let sample = match series.as_slice() {
        [] => bail!("no data in Prometheus"),
        [one] => one,
        _ => bail!("ambiguous: Prometheus returned several series"),
    };
    let pair = sample
        .get("value")
        .and_then(Value::as_array)
        .filter(|p| p.len() == 2)
        .ok_or_else(|| anyhow!("malformed Prometheus sample"))?;
    let secs = pair[0]
        .as_f64()
        .filter(|s| s.is_finite())
        .ok_or_else(|| anyhow!("malformed Prometheus timestamp"))?;
    let ts = Utc
        .timestamp_millis_opt((secs * 1000.0) as i64)
        .single()
        .ok_or_else(|| anyhow!("malformed Prometheus timestamp"))?;
    check_age(ts, now, max_age)?;
    let val = pair[1]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| anyhow!("malformed Prometheus value"))?;
    check_percent(val)
}

/// Largest token file accepted.
const MAX_TOKEN_BYTES: u64 = 8 * 1024;

/// Reads the bearer token from `path` and builds a sensitive `Authorization` header.
/// Refuses to do so unless `base` is https. Errors never contain the token.
pub fn bearer_header(
    base: &reqwest::Url,
    path: &std::path::Path,
) -> Result<reqwest::header::HeaderValue> {
    if base.scheme() != "https" {
        bail!(
            "refusing to send the Prometheus token over {}",
            base.scheme()
        );
    }
    let meta = std::fs::metadata(path).map_err(|e| {
        anyhow!(
            "cannot read Prometheus token file: {}",
            e.kind().to_string().to_lowercase()
        )
    })?;
    if !meta.is_file() || meta.len() > MAX_TOKEN_BYTES {
        bail!("Prometheus token file is not a regular file of at most 8 KiB");
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|_| anyhow!("Prometheus token file is unreadable or not UTF-8"))?;
    let token = raw.trim();
    if token.is_empty() {
        bail!("Prometheus token file is empty");
    }
    if token.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("Prometheus token file must contain a single token");
    }
    let mut v = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| anyhow!("Prometheus token is not a valid header value"))?;
    v.set_sensitive(true);
    Ok(v)
}

struct PromSource {
    http: reqwest::Client,
    base: reqwest::Url,
    token_file: Option<std::path::PathBuf>,
}

impl PromSource {
    fn new(base: reqwest::Url, token_file: Option<std::path::PathBuf>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(PROM_TIMEOUT)
            .connect_timeout(PROM_CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        Ok(Self {
            http,
            base,
            token_file,
        })
    }

    async fn query(&self, q: &str) -> Result<Vec<u8>> {
        let mut url = self.base.clone();
        let path = format!("{}/api/v1/query", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let mut rb = self.http.get(url).query(&[("query", q)]);
        if let Some(tf) = &self.token_file {
            // Re-read per request so a rotated token is picked up without a restart.
            rb = rb.header(
                reqwest::header::AUTHORIZATION,
                bearer_header(&self.base, tf)?,
            );
        }
        let mut resp = rb
            .send()
            .await
            .map_err(|e| anyhow!("prometheus request failed: {}", e.without_url()))?;
        if !resp.status().is_success() {
            bail!("prometheus returned HTTP {}", resp.status().as_u16());
        }
        if resp
            .content_length()
            .is_some_and(|l| l > PROM_MAX_BODY as u64)
        {
            bail!("prometheus response too large");
        }
        let mut buf = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| anyhow!("prometheus read failed: {}", e.without_url()))?
        {
            if buf.len() + chunk.len() > PROM_MAX_BODY {
                bail!("prometheus response too large");
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    async fn load<Z: VmSizing>(
        &self,
        sizing: &Z,
        t: &LoopTarget,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<f64> {
        let vms = vms_of(t);
        let mut sum = 0.0;
        for vm in &vms {
            let size = sizing.size(&t.namespace, vm).await?;
            let q = prom_query(t.load_metric, &t.namespace, vm, size)?;
            let body = self.query(&q).await?;
            sum +=
                parse_prom_response(&body, now, max_age).map_err(|e| anyhow!("VM '{vm}': {e}"))?;
        }
        Ok(sum / vms.len() as f64)
    }
}

// -------------------------------------------------------------------- composite

/// Telemetry settings (all optional).
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub max_age: Duration,
    pub prometheus: Option<reqwest::Url>,
    /// File holding a bearer token for the Prometheus endpoint (never the token itself).
    pub prometheus_token_file: Option<std::path::PathBuf>,
}

impl TelemetryConfig {
    /// `ZORVIA_AUTOSCALER_METRICS_MAX_AGE_SECS` (default 120, clamped 30..900) and
    /// `ZORVIA_AUTOSCALER_PROMETHEUS_URL`. An invalid Prometheus URL is an error.
    pub fn from_getter(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let secs = get("ZORVIA_AUTOSCALER_METRICS_MAX_AGE_SECS")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(DEFAULT_MAX_AGE_SECS)
            .clamp(MIN_MAX_AGE_SECS, MAX_MAX_AGE_SECS);
        let prometheus =
            match get("ZORVIA_AUTOSCALER_PROMETHEUS_URL").filter(|v| !v.trim().is_empty()) {
                Some(raw) => Some(validate_prometheus_url(&raw)?),
                None => None,
            };
        let prometheus_token_file = get("ZORVIA_AUTOSCALER_PROMETHEUS_TOKEN_FILE")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from);
        if let (Some(u), Some(_)) = (&prometheus, &prometheus_token_file) {
            if u.scheme() != "https" {
                bail!("ZORVIA_AUTOSCALER_PROMETHEUS_TOKEN_FILE needs an https Prometheus URL");
            }
        }
        Ok(Self {
            max_age: Duration::seconds(secs),
            prometheus,
            prometheus_token_file,
        })
    }
}

/// Metrics API first, then Prometheus (if configured). Never simulated.
pub struct TelemetryLoadSource<Z: VmSizing> {
    client: kube::Client,
    sizing: Z,
    max_age: Duration,
    prom: Option<PromSource>,
}

impl TelemetryLoadSource<KubeSizing> {
    /// Build from the environment. A bad Prometheus URL disables only the Prometheus
    /// fallback (logged); the metrics API source still works.
    pub fn from_env(kube: &KubeClient) -> Self {
        let cfg = TelemetryConfig::from_getter(|k| std::env::var(k).ok()).unwrap_or_else(|e| {
            log::error!("autoscaler telemetry: {e}; Prometheus source disabled");
            TelemetryConfig {
                max_age: Duration::seconds(DEFAULT_MAX_AGE_SECS),
                prometheus: None,
                prometheus_token_file: None,
            }
        });
        Self::new(kube.client(), KubeSizing(kube.clone()), cfg)
    }
}

impl<Z: VmSizing> TelemetryLoadSource<Z> {
    pub fn new(client: kube::Client, sizing: Z, cfg: TelemetryConfig) -> Self {
        let token_file = cfg.prometheus_token_file;
        let prom = cfg
            .prometheus
            .and_then(|u| match PromSource::new(u, token_file) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::error!("autoscaler telemetry: cannot build Prometheus client: {e}");
                    None
                }
            });
        Self {
            client,
            sizing,
            max_age: cfg.max_age,
            prom,
        }
    }

    async fn load_at(&self, t: &LoopTarget, now: DateTime<Utc>) -> Result<LoadSample> {
        let first = metrics_api_load(&self.client, &self.sizing, t, now, self.max_age).await;
        let (pct, source) = match (first, &self.prom) {
            (Ok(p), _) => (p, "metrics_api"),
            (Err(e), None) => return Err(e),
            (Err(e1), Some(prom)) => (
                prom.load(&self.sizing, t, now, self.max_age)
                    .await
                    .map_err(|e2| {
                        anyhow!("no real telemetry: metrics API: {e1}; prometheus: {e2}")
                    })?,
                "prometheus",
            ),
        };
        Ok(LoadSample {
            percent: pct,
            simulated: false,
            source,
        })
    }
}

impl<Z: VmSizing> LoadSource for TelemetryLoadSource<Z> {
    async fn load(&self, t: &LoopTarget) -> Result<LoadSample> {
        self.load_at(t, Utc::now()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoscaler::reconcile::TargetKind;
    use axum::{extract::Query, http::StatusCode, routing::get, Router};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const PM_OK: &str = include_str!("../../tests/fixtures/autoscaler/podmetrics_ok.json");
    const PM_NO_COMPUTE: &str =
        include_str!("../../tests/fixtures/autoscaler/podmetrics_no_compute.json");
    const PM_BAD_QTY: &str =
        include_str!("../../tests/fixtures/autoscaler/podmetrics_bad_qty.json");
    const PROM_OK: &str = include_str!("../../tests/fixtures/autoscaler/prom_ok.json");
    const PROM_EMPTY: &str = include_str!("../../tests/fixtures/autoscaler/prom_empty.json");
    const PROM_NAN: &str = include_str!("../../tests/fixtures/autoscaler/prom_nan.json");
    const PROM_MULTI: &str = include_str!("../../tests/fixtures/autoscaler/prom_multi.json");
    const PROM_ERR: &str = include_str!("../../tests/fixtures/autoscaler/prom_error.json");

    struct FakeSizing;
    impl VmSizing for FakeSizing {
        async fn size(&self, _ns: &str, _vm: &str) -> Result<VmSize> {
            Ok(VmSize {
                vcpus: 4.0,
                memory_bytes: 8.0 * 1073741824.0,
            })
        }
    }

    /// Fixture with `__TS__` replaced by an RFC3339 time `age_secs` ago.
    fn pm(fixture: &str, age_secs: i64) -> String {
        let ts = (Utc::now() - Duration::seconds(age_secs)).to_rfc3339();
        fixture.replace("__TS__", &ts)
    }

    /// Fixture with `__EPOCH__` replaced by a unix time `age_secs` ago.
    fn prom(fixture: &str, age_secs: i64) -> String {
        let t = (Utc::now() - Duration::seconds(age_secs)).timestamp();
        fixture.replace("__EPOCH__", &t.to_string())
    }

    #[derive(Clone)]
    struct Mock {
        pm_status: u16,
        pm_body: String,
        prom_status: u16,
        prom_body: String,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Mock {
        fn new(pm_body: String, prom_body: String) -> Self {
            Self {
                pm_status: 200,
                pm_body,
                prom_status: 200,
                prom_body,
                seen: Arc::default(),
            }
        }
    }

    async fn serve(m: Mock) -> (std::net::SocketAddr, Arc<Mutex<Vec<String>>>) {
        let seen = m.seen.clone();
        let (a, b) = (m.clone(), m);
        let app = Router::new()
            .route(
                "/apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let m = a.clone();
                    async move {
                        m.seen.lock().unwrap().push(format!(
                            "pods {}",
                            q.get("labelSelector").cloned().unwrap_or_default()
                        ));
                        (
                            StatusCode::from_u16(m.pm_status).unwrap(),
                            [("content-type", "application/json")],
                            m.pm_body,
                        )
                    }
                }),
            )
            .route(
                "/api/v1/query",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let m = b.clone();
                    async move {
                        m.seen.lock().unwrap().push(format!(
                            "query {}",
                            q.get("query").cloned().unwrap_or_default()
                        ));
                        (
                            StatusCode::from_u16(m.prom_status).unwrap(),
                            [("content-type", "application/json")],
                            m.prom_body,
                        )
                    }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (addr, seen)
    }

    fn kube_client(addr: std::net::SocketAddr) -> kube::Client {
        let cfg = kube::Config::new(format!("http://{addr}").parse().unwrap());
        kube::Client::try_from(cfg).unwrap()
    }

    fn target(metric: LoadMetric) -> LoopTarget {
        LoopTarget {
            id: "t".into(),
            kind: TargetKind::VmCpu,
            namespace: "prod".into(),
            name: "vm-a".into(),
            metric_vms: vec![],
            load_metric: metric,
            min: 1,
            max: 8,
            step: 1,
            scale_up_threshold: 80.0,
            scale_down_threshold: 30.0,
            required_samples: 3,
            cooldown_secs: 300,
        }
    }

    fn source(addr: std::net::SocketAddr, with_prom: bool) -> TelemetryLoadSource<FakeSizing> {
        TelemetryLoadSource::new(
            kube_client(addr),
            FakeSizing,
            TelemetryConfig {
                max_age: Duration::seconds(120),
                prometheus: with_prom.then(|| format!("http://{addr}").parse().unwrap()),
                prometheus_token_file: None,
            },
        )
    }

    #[test]
    fn token_file_config_and_header() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tok");
        let fs = f.to_str().unwrap().to_string();
        let https: reqwest::Url = "https://prom.example.com".parse().unwrap();
        let http: reqwest::Url = "http://127.0.0.1:9090".parse().unwrap();

        // https is required, even to loopback
        std::fs::write(&f, "s3cret\n").unwrap();
        let e = bearer_header(&http, &f).unwrap_err().to_string();
        assert!(e.contains("refusing") && !e.contains("s3cret"));

        let h = bearer_header(&https, &f).unwrap();
        assert_eq!(h.to_str().unwrap(), "Bearer s3cret");
        assert!(h.is_sensitive());
        assert!(!format!("{h:?}").contains("s3cret"));

        // rotation: the next call sees the new token
        std::fs::write(&f, "rotated").unwrap();
        assert_eq!(
            bearer_header(&https, &f).unwrap().to_str().unwrap(),
            "Bearer rotated"
        );

        // bad contents / missing file never echo the contents
        for bad in ["", "  \n", "two words", "a\r\nb"] {
            std::fs::write(&f, bad).unwrap();
            assert!(bearer_header(&https, &f).is_err(), "{bad:?}");
        }
        assert!(bearer_header(&https, &dir.path().join("nope")).is_err());

        // config: the path comes from the env getter; non-https is rejected up front
        let get = |url: &'static str| {
            let fs = fs.clone();
            move |k: &str| match k {
                "ZORVIA_AUTOSCALER_PROMETHEUS_URL" => Some(url.to_string()),
                "ZORVIA_AUTOSCALER_PROMETHEUS_TOKEN_FILE" => Some(fs.clone()),
                _ => None,
            }
        };
        let c = TelemetryConfig::from_getter(get("https://prom.example.com")).unwrap();
        assert_eq!(c.prometheus_token_file.as_deref(), Some(f.as_path()));
        assert!(TelemetryConfig::from_getter(get("http://127.0.0.1:9090")).is_err());
        let none = TelemetryConfig::from_getter(|_| None).unwrap();
        assert!(none.prometheus_token_file.is_none());
    }

    #[tokio::test]
    async fn token_never_sent_over_http() {
        let (addr, seen) = serve(Mock::new(pm("2000000000n", 10), prom("40", 10))).await;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tok");
        std::fs::write(&f, "s3cret").unwrap();
        let p = PromSource::new(format!("http://{addr}").parse().unwrap(), Some(f)).unwrap();
        let e = p.query("up").await.unwrap_err().to_string();
        assert!(e.contains("refusing") && !e.contains("s3cret"));
        assert!(seen.lock().unwrap().is_empty(), "no request must be made");
    }

    #[test]
    fn quantity_parsing() {
        assert_eq!(parse_cpu_cores("2000000000n"), Some(2.0));
        assert_eq!(parse_cpu_cores("500m"), Some(0.5));
        assert_eq!(parse_cpu_cores("250000u"), Some(0.25));
        assert_eq!(parse_cpu_cores("3"), Some(3.0));
        for bad in ["", "x", "-1", "NaN", "infm", "1.2.3"] {
            assert_eq!(parse_cpu_cores(bad), None, "{bad}");
        }
        assert_eq!(parse_bytes("1Ki"), Some(1024.0));
        assert_eq!(parse_bytes("2Gi"), Some(2.0 * 1073741824.0));
        assert_eq!(parse_bytes("3M"), Some(3e6));
        assert_eq!(parse_bytes("123"), Some(123.0));
        for bad in ["", "Gi", "-5Mi", "inf", "12Xi"] {
            assert_eq!(parse_bytes(bad), None, "{bad}");
        }
    }

    #[test]
    fn prometheus_url_rules() {
        for ok in [
            "https://prom.example.com",
            "https://prom.example.com:9091/prefix",
            "http://127.0.0.1:9090",
            "http://localhost:9090",
            "http://[::1]:9090",
        ] {
            assert!(validate_prometheus_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://prom.example.com",
            "http://10.0.0.5:9090",
            "http://127.0.0.1.evil.com",
            "ftp://127.0.0.1",
            "https://user:pw@prom.example.com",
            "https://prom.example.com/?q=1",
            "not a url",
            "",
        ] {
            assert!(validate_prometheus_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn config_from_env_clamps_and_validates() {
        let get =
            |m: HashMap<&'static str, &'static str>| move |k: &str| m.get(k).map(|v| v.to_string());
        let c = TelemetryConfig::from_getter(get(HashMap::new())).unwrap();
        assert_eq!(c.max_age, Duration::seconds(120));
        assert!(c.prometheus.is_none());
        let c = TelemetryConfig::from_getter(get(HashMap::from([(
            "ZORVIA_AUTOSCALER_METRICS_MAX_AGE_SECS",
            "5",
        )])))
        .unwrap();
        assert_eq!(c.max_age, Duration::seconds(30));
        assert!(TelemetryConfig::from_getter(get(HashMap::from([(
            "ZORVIA_AUTOSCALER_PROMETHEUS_URL",
            "http://prom.example.com",
        )])))
        .is_err());
    }

    #[test]
    fn query_rejects_injection() {
        let s = VmSize {
            vcpus: 2.0,
            memory_bytes: 1.0,
        };
        assert!(prom_query(LoadMetric::Cpu, "ns", "vm", s).is_ok());
        assert!(prom_query(LoadMetric::Cpu, "ns\"}", "vm", s).is_err());
        assert!(prom_query(LoadMetric::Memory, "ns", "a b", s).is_err());
    }

    #[tokio::test]
    async fn metrics_api_cpu_and_memory_are_real() {
        let (addr, seen) = serve(Mock::new(pm(PM_OK, 10), String::new())).await;
        let src = source(addr, false);
        let cpu = src.load(&target(LoadMetric::Cpu)).await.unwrap();
        assert!(!cpu.simulated);
        assert!((cpu.percent - 50.0).abs() < 1e-6, "{}", cpu.percent); // 2 of 4 vCPU
        let mem = src.load(&target(LoadMetric::Memory)).await.unwrap();
        assert!((mem.percent - 25.0).abs() < 1e-6, "{}", mem.percent); // 2Gi of 8Gi
        assert_eq!(
            seen.lock().unwrap()[0],
            "pods kubevirt.io=virt-launcher,vm.kubevirt.io/name=vm-a"
        );
    }

    #[tokio::test]
    async fn pool_load_is_the_mean_of_members() {
        let (addr, seen) = serve(Mock::new(pm(PM_OK, 10), String::new())).await;
        let mut t = target(LoadMetric::Cpu);
        t.kind = TargetKind::VmPool;
        t.metric_vms = vec!["p-0".into(), "p-1".into()];
        let s = source(addr, false).load(&t).await.unwrap();
        assert!((s.percent - 50.0).abs() < 1e-6);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn stale_missing_and_malformed_metrics_give_no_sample() {
        let cases = [
            (pm(PM_OK, 3600), "stale"),
            (pm(PM_OK, -3600), "future"),
            (r#"{"items":[]}"#.to_string(), "no PodMetrics"),
            (pm(PM_NO_COMPUTE, 5), "compute"),
            (pm(PM_BAD_QTY, 5), "bad cpu"),
            ("not json".to_string(), "unavailable"),
            (pm(PM_OK, 5).replace("timestamp", "ts"), "timestamp"),
            (
                // two launcher pods (mid-migration) are ambiguous
                {
                    let one = pm(PM_OK, 5);
                    let v: Value = serde_json::from_str(&one).unwrap();
                    let item = v["items"][0].clone();
                    serde_json::json!({"items": [item.clone(), item]}).to_string()
                },
                "ambiguous",
            ),
        ];
        for (body, needle) in cases {
            let (addr, _) = serve(Mock::new(body, String::new())).await;
            let err = source(addr, false)
                .load(&target(LoadMetric::Cpu))
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    #[tokio::test]
    async fn metrics_api_http_error_gives_no_sample() {
        let mut m = Mock::new(r#"{"kind":"Status"}"#.into(), String::new());
        m.pm_status = 500; // 503, 504 and 429 are retried by the kube client
        let (addr, _) = serve(m).await;
        assert!(source(addr, false)
            .load(&target(LoadMetric::Cpu))
            .await
            .is_err());
    }

    #[test]
    fn prometheus_parsing_table() {
        let now = Utc::now();
        let age = Duration::seconds(120);
        let ok = parse_prom_response(prom(PROM_OK, 5).as_bytes(), now, age).unwrap();
        assert!((ok - 42.5).abs() < 1e-9);
        for (body, needle) in [
            (prom(PROM_OK, 3600), "stale"),
            (prom(PROM_EMPTY, 5), "no data"),
            (prom(PROM_NAN, 5), "out of range"),
            (prom(PROM_MULTI, 5), "ambiguous"),
            (prom(PROM_ERR, 5), "did not succeed"),
            ("<html>".to_string(), "malformed"),
            (prom(PROM_OK, 5).replace("\"42.5\"", "\"abc\""), "value"),
            (
                prom(PROM_OK, 5).replace("\"42.5\"", "\"-3\""),
                "out of range",
            ),
            (
                prom(PROM_OK, 5).replace("\"42.5\"", "\"5000\""),
                "out of range",
            ),
            (prom(PROM_OK, 5).replace("vector", "matrix"), "not a vector"),
        ] {
            let err = parse_prom_response(body.as_bytes(), now, age)
                .unwrap_err()
                .to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    #[tokio::test]
    async fn prometheus_is_used_when_metrics_api_has_nothing() {
        let (addr, seen) = serve(Mock::new(r#"{"items":[]}"#.into(), prom(PROM_OK, 5))).await;
        let s = source(addr, true)
            .load(&target(LoadMetric::Cpu))
            .await
            .unwrap();
        assert!(!s.simulated);
        assert!((s.percent - 42.5).abs() < 1e-9);
        let q = seen.lock().unwrap().last().unwrap().clone();
        assert!(q.contains(r#"namespace="prod",name="vm-a""#), "{q}");
        assert!(q.ends_with("/ 4"), "{q}");
    }

    #[tokio::test]
    async fn metrics_api_wins_over_prometheus() {
        let (addr, seen) = serve(Mock::new(pm(PM_OK, 5), prom(PROM_OK, 5))).await;
        let s = source(addr, true)
            .load(&target(LoadMetric::Cpu))
            .await
            .unwrap();
        assert!((s.percent - 50.0).abs() < 1e-6);
        assert!(seen.lock().unwrap().iter().all(|l| l.starts_with("pods")));
    }

    #[tokio::test]
    async fn both_sources_bad_gives_no_sample() {
        for prom_body in [prom(PROM_OK, 3600), prom(PROM_EMPTY, 5), "garbage".into()] {
            let (addr, _) = serve(Mock::new(pm(PM_OK, 3600), prom_body)).await;
            let err = source(addr, true)
                .load(&target(LoadMetric::Cpu))
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("no real telemetry"), "{err}");
        }
    }

    #[tokio::test]
    async fn prometheus_http_error_and_oversize_are_rejected() {
        let mut m = Mock::new(String::new(), prom(PROM_OK, 5));
        m.prom_status = 500;
        let (addr, _) = serve(m).await;
        let p = PromSource::new(format!("http://{addr}").parse().unwrap(), None).unwrap();
        assert!(p
            .query("up")
            .await
            .unwrap_err()
            .to_string()
            .contains("HTTP 500"));

        let big = format!("{{\"pad\":\"{}\"}}", "a".repeat(PROM_MAX_BODY + 10));
        let (addr, _) = serve(Mock::new(String::new(), big)).await;
        let p = PromSource::new(format!("http://{addr}").parse().unwrap(), None).unwrap();
        assert!(p
            .query("up")
            .await
            .unwrap_err()
            .to_string()
            .contains("too large"));
    }

    #[tokio::test]
    async fn prometheus_does_not_follow_redirects() {
        let app = Router::new().route(
            "/api/v1/query",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "http://169.254.169.254/")],
                    "",
                )
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let p = PromSource::new(format!("http://{addr}").parse().unwrap(), None).unwrap();
        assert!(p
            .query("up")
            .await
            .unwrap_err()
            .to_string()
            .contains("HTTP 302"));
    }
}
