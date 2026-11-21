// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Autoscaling reconcile loop: decision logic and orchestration.
//!
//! Opt-in (`ZORVIA_AUTOSCALER_LOOP=1` plus `ZORVIA_EXPERIMENTAL=1`), recommendation-only
//! unless `ZORVIA_AUTOSCALER_APPLY=1`. Everything here is I/O-free: load comes from a
//! [`LoadSource`], scaling goes through a [`ScaleExecutor`], and results go to an
//! [`EventSink`], so the decision logic is testable with fakes. The live adapters are in
//! `autoscaler::live`; the audit/operations sink and the leader-gated spawn are in the web
//! layer. See docs/AUTOSCALER_LOOP.md.

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_TARGETS: usize = 100;
pub const MAX_BOUND: u32 = 1000;
pub const MIN_COOLDOWN_SECS: u64 = 30;
/// Minimum distance between the scale-up and scale-down thresholds (hysteresis dead band).
pub const MIN_DEAD_BAND: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Total vCPU count of one VM (CPU hotplug; needs `cpu.maxSockets`).
    VmCpu,
    /// `spec.replicas` of a KubeVirt `VirtualMachinePool`.
    VmPool,
}

/// Which utilisation figure drives a target (default CPU).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadMetric {
    #[default]
    Cpu,
    Memory,
}

/// One scaled resource and its policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopTarget {
    pub id: String,
    pub kind: TargetKind,
    pub namespace: String,
    pub name: String,
    /// VMs whose CPU usage is averaged as the load signal (defaults to `[name]` for `vm_cpu`;
    /// required for `vm_pool`).
    #[serde(default)]
    pub metric_vms: Vec<String>,
    /// Utilisation signal: `cpu` (default) or `memory`.
    #[serde(default)]
    pub load_metric: LoadMetric,
    pub min: u32,
    pub max: u32,
    #[serde(default = "d_step")]
    pub step: u32,
    #[serde(default = "d_up")]
    pub scale_up_threshold: f64,
    #[serde(default = "d_down")]
    pub scale_down_threshold: f64,
    /// Consecutive samples beyond a threshold required before acting (hysteresis).
    #[serde(default = "d_samples")]
    pub required_samples: u32,
    #[serde(default = "d_cooldown")]
    pub cooldown_secs: u64,
}

fn d_step() -> u32 {
    1
}
fn d_up() -> f64 {
    80.0
}
fn d_down() -> f64 {
    30.0
}
fn d_samples() -> u32 {
    3
}
fn d_cooldown() -> u64 {
    300
}

fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

impl LoopTarget {
    pub fn validate(&self) -> Result<()> {
        if !dns_label(&self.id) {
            bail!("target id '{}' must be a DNS-1123 label", self.id);
        }
        if !dns_label(&self.namespace) || !dns_label(&self.name) {
            bail!(
                "target '{}': namespace and name must be DNS-1123 labels",
                self.id
            );
        }
        if self.metric_vms.len() > 50 || self.metric_vms.iter().any(|v| !dns_label(v)) {
            bail!("target '{}': invalid metric_vms", self.id);
        }
        if self.kind == TargetKind::VmPool && self.metric_vms.is_empty() {
            bail!("target '{}': vm_pool needs metric_vms", self.id);
        }
        let floor = if self.kind == TargetKind::VmCpu { 1 } else { 0 };
        if self.min < floor || self.min > self.max || self.max > MAX_BOUND {
            bail!(
                "target '{}': need {floor} <= min <= max <= {MAX_BOUND}",
                self.id
            );
        }
        if self.step == 0 || self.step > MAX_BOUND {
            bail!("target '{}': step must be 1..={MAX_BOUND}", self.id);
        }
        let (up, down) = (self.scale_up_threshold, self.scale_down_threshold);
        if !(up.is_finite() && down.is_finite()) || down <= 0.0 || up > 100.0 {
            bail!("target '{}': thresholds must be within (0, 100]", self.id);
        }
        if up - down < MIN_DEAD_BAND {
            bail!(
                "target '{}': scale_up_threshold must exceed scale_down_threshold by >= {MIN_DEAD_BAND} (hysteresis dead band)",
                self.id
            );
        }
        if self.required_samples == 0 || self.required_samples > 100 {
            bail!("target '{}': required_samples must be 1..=100", self.id);
        }
        if self.cooldown_secs < MIN_COOLDOWN_SECS {
            bail!(
                "target '{}': cooldown_secs must be >= {MIN_COOLDOWN_SECS}",
                self.id
            );
        }
        Ok(())
    }
}

/// Parse and validate a JSON array of targets.
pub fn parse_targets(json: &str) -> Result<Vec<LoopTarget>> {
    let targets: Vec<LoopTarget> = serde_json::from_str(json)?;
    if targets.len() > MAX_TARGETS {
        bail!("at most {MAX_TARGETS} targets");
    }
    let mut seen = std::collections::HashSet::new();
    for t in &targets {
        t.validate()?;
        if !seen.insert(t.id.clone()) {
            bail!("duplicate target id '{}'", t.id);
        }
    }
    Ok(targets)
}

#[derive(Debug, Clone, Copy)]
pub struct LoopConfig {
    pub apply: bool,
    pub interval_secs: u64,
}

impl LoopConfig {
    /// `None` unless `ZORVIA_AUTOSCALER_LOOP` is truthy AND experimental features are on.
    pub fn from_env() -> Option<Self> {
        Self::from_getter(
            |k| std::env::var(k).ok(),
            crate::features::experimental_enabled(),
        )
    }

    pub fn from_getter(get: impl Fn(&str) -> Option<String>, experimental: bool) -> Option<Self> {
        let truthy = |k: &str| {
            get(k).is_some_and(|v| {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            })
        };
        if !experimental || !truthy("ZORVIA_AUTOSCALER_LOOP") {
            return None;
        }
        let interval_secs = get("ZORVIA_AUTOSCALER_INTERVAL_SECS")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(60)
            .clamp(10, 3600);
        Some(Self {
            apply: truthy("ZORVIA_AUTOSCALER_APPLY"),
            interval_secs,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoadSample {
    /// Utilisation percent (0..=100, small overshoot tolerated).
    pub percent: f64,
    /// True when the number is not real telemetry. Simulated load is never acted on.
    pub simulated: bool,
}

#[allow(async_fn_in_trait)]
pub trait LoadSource {
    async fn load(&self, target: &LoopTarget) -> Result<LoadSample>;
}

#[allow(async_fn_in_trait)]
pub trait ScaleExecutor {
    async fn current(&self, target: &LoopTarget) -> Result<u32>;
    async fn scale(&self, target: &LoopTarget, to: u32) -> Result<()>;
}

#[allow(async_fn_in_trait)]
pub trait EventSink {
    async fn record(&self, event: &ScaleEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Recommendation only (dry-run default, or simulated load).
    Recommended,
    Applied,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScaleEvent {
    pub target_id: String,
    pub kind: TargetKind,
    pub namespace: String,
    pub name: String,
    pub direction: Direction,
    pub from: u32,
    pub to: u32,
    pub load_percent: f64,
    pub mode: Mode,
    pub detail: String,
}

/// Per-target hysteresis and cooldown memory.
#[derive(Debug, Default, Clone)]
pub struct TargetState {
    pub up_streak: u32,
    pub down_streak: u32,
    pub last_action: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Hold(&'static str),
    Scale {
        direction: Direction,
        from: u32,
        to: u32,
    },
}

/// Pure decision: updates streaks, applies hysteresis, bounds and cooldown.
pub fn decide(
    t: &LoopTarget,
    st: &mut TargetState,
    current: u32,
    load: f64,
    now: DateTime<Utc>,
) -> Decision {
    if !load.is_finite() || !(0.0..=1000.0).contains(&load) {
        return Decision::Hold("invalid load sample");
    }
    if load >= t.scale_up_threshold {
        st.up_streak = st.up_streak.saturating_add(1);
        st.down_streak = 0;
    } else if load <= t.scale_down_threshold {
        st.down_streak = st.down_streak.saturating_add(1);
        st.up_streak = 0;
    } else {
        // Dead band: any in-between sample resets both streaks, so flapping load never acts.
        st.up_streak = 0;
        st.down_streak = 0;
        return Decision::Hold("within dead band");
    }
    if current < t.min || current > t.max {
        return Decision::Hold("current outside configured bounds");
    }
    let (direction, to) = if st.up_streak >= t.required_samples {
        (Direction::Up, current.saturating_add(t.step).min(t.max))
    } else if st.down_streak >= t.required_samples {
        (Direction::Down, current.saturating_sub(t.step).max(t.min))
    } else {
        return Decision::Hold("waiting for consecutive samples");
    };
    if to == current {
        return Decision::Hold("at bound");
    }
    if let Some(last) = st.last_action {
        if now - last < Duration::seconds(t.cooldown_secs as i64) {
            return Decision::Hold("cooldown");
        }
    }
    Decision::Scale {
        direction,
        from: current,
        to,
    }
}

#[derive(Default)]
pub struct Reconciler {
    states: HashMap<String, TargetState>,
    warned_simulated: bool,
}

impl Reconciler {
    pub fn state(&self, id: &str) -> Option<&TargetState> {
        self.states.get(id)
    }

    /// One pass over all targets. Returns the events produced (also sent to `sink`).
    /// A non-leader does nothing at all (no metrics reads, no state change).
    #[allow(clippy::too_many_arguments)]
    pub async fn tick<M, E, S>(
        &mut self,
        now: DateTime<Utc>,
        is_leader: bool,
        cfg: &LoopConfig,
        targets: &[LoopTarget],
        metrics: &M,
        exec: &E,
        sink: &S,
    ) -> Vec<ScaleEvent>
    where
        M: LoadSource,
        E: ScaleExecutor,
        S: EventSink,
    {
        let mut out = Vec::new();
        if !is_leader {
            return out;
        }
        for t in targets {
            let sample = match metrics.load(t).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("autoscaler: load for '{}' unavailable: {e}", t.id);
                    continue;
                }
            };
            let current = match exec.current(t).await {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("autoscaler: current size of '{}' unavailable: {e}", t.id);
                    continue;
                }
            };
            let st = self.states.entry(t.id.clone()).or_default();
            let Decision::Scale {
                direction,
                from,
                to,
            } = decide(t, st, current, sample.percent, now)
            else {
                continue;
            };
            // Cooldown starts on any recommendation/attempt so a failure is not hammered.
            st.last_action = Some(now);
            st.up_streak = 0;
            st.down_streak = 0;
            let recommend_only = !cfg.apply || sample.simulated;
            if sample.simulated && cfg.apply && !self.warned_simulated {
                self.warned_simulated = true;
                log::warn!("autoscaler: load source is simulated; recommendations only, nothing is applied");
            }
            let (mode, detail) = if recommend_only {
                let why = if sample.simulated {
                    "recommendation only: load source is simulated"
                } else {
                    "recommendation only: dry-run (ZORVIA_AUTOSCALER_APPLY not set)"
                };
                (Mode::Recommended, why.to_string())
            } else {
                match exec.scale(t, to).await {
                    Ok(()) => (Mode::Applied, "applied".to_string()),
                    // Executor error text is sanitised to a short single line.
                    Err(e) => (
                        Mode::Failed,
                        format!(
                            "scale failed: {}",
                            e.to_string().lines().next().unwrap_or("error")
                        )
                        .chars()
                        .take(200)
                        .collect(),
                    ),
                }
            };
            let ev = ScaleEvent {
                target_id: t.id.clone(),
                kind: t.kind,
                namespace: t.namespace.clone(),
                name: t.name.clone(),
                direction,
                from,
                to,
                load_percent: sample.percent,
                mode,
                detail,
            };
            sink.record(&ev).await;
            out.push(ev);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn target() -> LoopTarget {
        LoopTarget {
            id: "web".into(),
            kind: TargetKind::VmPool,
            namespace: "ns".into(),
            name: "pool".into(),
            metric_vms: vec!["vm-a".into()],
            load_metric: LoadMetric::Cpu,
            min: 2,
            max: 5,
            step: 1,
            scale_up_threshold: 80.0,
            scale_down_threshold: 30.0,
            required_samples: 3,
            cooldown_secs: 300,
        }
    }

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn decide_table() {
        // (name, current, loads, expected last decision)
        let up = |from, to| Decision::Scale {
            direction: Direction::Up,
            from,
            to,
        };
        let down = |from, to| Decision::Scale {
            direction: Direction::Down,
            from,
            to,
        };
        let cases: Vec<(&str, u32, Vec<f64>, Decision)> = vec![
            (
                "needs consecutive samples",
                3,
                vec![90.0, 90.0],
                Decision::Hold("waiting for consecutive samples"),
            ),
            ("scales up after 3", 3, vec![90.0, 91.0, 92.0], up(3, 4)),
            ("scales down after 3", 3, vec![10.0, 10.0, 10.0], down(3, 2)),
            (
                "flapping never acts",
                3,
                vec![90.0, 10.0, 90.0, 10.0, 90.0, 10.0],
                Decision::Hold("waiting for consecutive samples"),
            ),
            (
                "dead band resets streak",
                3,
                vec![90.0, 90.0, 50.0, 90.0],
                Decision::Hold("waiting for consecutive samples"),
            ),
            (
                "dead band sample",
                3,
                vec![50.0],
                Decision::Hold("within dead band"),
            ),
            (
                "at max bound",
                5,
                vec![95.0, 95.0, 95.0],
                Decision::Hold("at bound"),
            ),
            (
                "at min bound",
                2,
                vec![1.0, 1.0, 1.0],
                Decision::Hold("at bound"),
            ),
            (
                "step clamps to max",
                5,
                vec![],
                Decision::Hold("waiting for consecutive samples"),
            ),
            (
                "exact up threshold counts",
                3,
                vec![80.0, 80.0, 80.0],
                up(3, 4),
            ),
            (
                "exact down threshold counts",
                3,
                vec![30.0, 30.0, 30.0],
                down(3, 2),
            ),
            (
                "nan rejected",
                3,
                vec![f64::NAN],
                Decision::Hold("invalid load sample"),
            ),
            (
                "negative rejected",
                3,
                vec![-1.0],
                Decision::Hold("invalid load sample"),
            ),
            (
                "outside bounds held",
                9,
                vec![95.0, 95.0, 95.0],
                Decision::Hold("current outside configured bounds"),
            ),
        ];
        for (name, current, loads, want) in cases {
            let t = target();
            let mut st = TargetState::default();
            let mut got = Decision::Hold("waiting for consecutive samples");
            for l in loads {
                got = decide(&t, &mut st, current, l, t0());
            }
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn step_is_clamped_to_bounds() {
        let mut t = target();
        t.step = 10;
        let mut st = TargetState::default();
        let mut d = Decision::Hold("");
        for _ in 0..3 {
            d = decide(&t, &mut st, 3, 99.0, t0());
        }
        assert_eq!(
            d,
            Decision::Scale {
                direction: Direction::Up,
                from: 3,
                to: 5
            }
        );
    }

    #[test]
    fn cooldown_blocks_then_releases() {
        let t = target();
        let mut st = TargetState {
            up_streak: 0,
            down_streak: 0,
            last_action: Some(t0()),
        };
        let mut d = Decision::Hold("");
        for _ in 0..3 {
            d = decide(&t, &mut st, 3, 99.0, t0() + Duration::seconds(299));
        }
        assert_eq!(d, Decision::Hold("cooldown"));
        // streak kept accumulating during cooldown, so it acts as soon as it ends
        let d = decide(&t, &mut st, 3, 99.0, t0() + Duration::seconds(300));
        assert!(matches!(d, Decision::Scale { to: 4, .. }));
    }

    struct FakeMetrics {
        loads: Mutex<Vec<f64>>,
        simulated: bool,
    }
    impl LoadSource for FakeMetrics {
        async fn load(&self, _t: &LoopTarget) -> Result<LoadSample> {
            let mut l = self.loads.lock().unwrap();
            if l.is_empty() {
                bail!("no data");
            }
            Ok(LoadSample {
                percent: l.remove(0),
                simulated: self.simulated,
            })
        }
    }
    struct FakeExec {
        size: Mutex<u32>,
        calls: Mutex<Vec<u32>>,
        fail: bool,
    }
    impl ScaleExecutor for FakeExec {
        async fn current(&self, _t: &LoopTarget) -> Result<u32> {
            Ok(*self.size.lock().unwrap())
        }
        async fn scale(&self, _t: &LoopTarget, to: u32) -> Result<()> {
            self.calls.lock().unwrap().push(to);
            if self.fail {
                bail!("boom\nsecret-ish second line");
            }
            *self.size.lock().unwrap() = to;
            Ok(())
        }
    }
    #[derive(Default)]
    struct FakeSink(Mutex<Vec<ScaleEvent>>);
    impl EventSink for FakeSink {
        async fn record(&self, e: &ScaleEvent) {
            self.0.lock().unwrap().push(e.clone());
        }
    }

    fn fakes(loads: Vec<f64>, simulated: bool, fail: bool) -> (FakeMetrics, FakeExec, FakeSink) {
        (
            FakeMetrics {
                loads: Mutex::new(loads),
                simulated,
            },
            FakeExec {
                size: Mutex::new(3),
                calls: Mutex::new(vec![]),
                fail,
            },
            FakeSink::default(),
        )
    }

    async fn run(
        r: &mut Reconciler,
        cfg: &LoopConfig,
        leader: bool,
        n: usize,
        f: &(FakeMetrics, FakeExec, FakeSink),
    ) -> Vec<ScaleEvent> {
        let ts = [target()];
        let mut all = vec![];
        for i in 0..n {
            all.extend(
                r.tick(
                    t0() + Duration::seconds(i as i64),
                    leader,
                    cfg,
                    &ts,
                    &f.0,
                    &f.1,
                    &f.2,
                )
                .await,
            );
        }
        all
    }

    const APPLY: LoopConfig = LoopConfig {
        apply: true,
        interval_secs: 60,
    };
    const DRY: LoopConfig = LoopConfig {
        apply: false,
        interval_secs: 60,
    };

    #[tokio::test]
    async fn apply_executes_once_then_cooldown_holds() {
        let f = fakes(vec![95.0; 6], false, false);
        let ev = run(&mut Reconciler::default(), &APPLY, true, 6, &f).await;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].mode, Mode::Applied);
        assert_eq!(*f.1.calls.lock().unwrap(), vec![4]);
        assert_eq!(f.2 .0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dry_run_recommends_and_never_executes() {
        let f = fakes(vec![95.0; 3], false, false);
        let ev = run(&mut Reconciler::default(), &DRY, true, 3, &f).await;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].mode, Mode::Recommended);
        assert!(f.1.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn simulated_load_is_never_applied() {
        let f = fakes(vec![95.0; 3], true, false);
        let ev = run(&mut Reconciler::default(), &APPLY, true, 3, &f).await;
        assert_eq!(ev[0].mode, Mode::Recommended);
        assert!(ev[0].detail.contains("simulated"));
        assert!(f.1.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn follower_does_nothing() {
        let f = fakes(vec![95.0; 5], false, false);
        let ev = run(&mut Reconciler::default(), &APPLY, false, 5, &f).await;
        assert!(ev.is_empty());
        assert_eq!(f.0.loads.lock().unwrap().len(), 5, "metrics not even read");
    }

    #[tokio::test]
    async fn failure_is_recorded_sanitised_and_cooled_down() {
        let f = fakes(vec![95.0; 6], false, true);
        let ev = run(&mut Reconciler::default(), &APPLY, true, 6, &f).await;
        assert_eq!(ev.len(), 1, "no retry inside cooldown");
        assert_eq!(ev[0].mode, Mode::Failed);
        assert!(!ev[0].detail.contains("secret-ish"));
    }

    #[tokio::test]
    async fn metrics_error_skips_target() {
        let f = fakes(vec![], false, false);
        let ev = run(&mut Reconciler::default(), &APPLY, true, 3, &f).await;
        assert!(ev.is_empty());
    }

    #[tokio::test]
    async fn flapping_load_produces_no_events() {
        let loads: Vec<f64> = (0..20)
            .map(|i| if i % 2 == 0 { 95.0 } else { 5.0 })
            .collect();
        let f = fakes(loads, false, false);
        let ev = run(&mut Reconciler::default(), &APPLY, true, 20, &f).await;
        assert!(ev.is_empty());
        assert!(f.1.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn config_is_off_by_default_and_needs_experimental() {
        let env = |loop_v: &'static str| {
            move |k: &str| (k == "ZORVIA_AUTOSCALER_LOOP").then(|| loop_v.to_string())
        };
        assert!(LoopConfig::from_getter(|_| None, true).is_none());
        assert!(LoopConfig::from_getter(env("1"), false).is_none());
        let c = LoopConfig::from_getter(env("1"), true).unwrap();
        assert!(!c.apply, "dry-run by default");
        assert_eq!(c.interval_secs, 60);
        let c = LoopConfig::from_getter(
            |k| match k {
                "ZORVIA_AUTOSCALER_LOOP" | "ZORVIA_AUTOSCALER_APPLY" => Some("true".into()),
                "ZORVIA_AUTOSCALER_INTERVAL_SECS" => Some("1".into()),
                _ => None,
            },
            true,
        )
        .unwrap();
        assert!(c.apply);
        assert_eq!(c.interval_secs, 10);
    }

    #[test]
    fn target_validation() {
        assert!(target().validate().is_ok());
        type Mutator = Box<dyn Fn(&mut LoopTarget)>;
        let bad: Vec<Mutator> = vec![
            Box::new(|t| t.min = 6),
            Box::new(|t| t.max = MAX_BOUND + 1),
            Box::new(|t| t.step = 0),
            Box::new(|t| t.scale_down_threshold = 78.0),
            Box::new(|t| t.scale_up_threshold = 101.0),
            Box::new(|t| t.required_samples = 0),
            Box::new(|t| t.cooldown_secs = 1),
            Box::new(|t| t.name = "Bad_Name".into()),
            Box::new(|t| t.metric_vms.clear()),
            Box::new(|t| {
                t.kind = TargetKind::VmCpu;
                t.min = 0;
            }),
        ];
        for (i, f) in bad.iter().enumerate() {
            let mut t = target();
            f(&mut t);
            assert!(t.validate().is_err(), "case {i}");
        }
    }

    #[test]
    fn parse_targets_rejects_duplicates_and_bad_json() {
        let one = r#"{"id":"a","kind":"vm_cpu","namespace":"n","name":"v","min":1,"max":8}"#;
        assert_eq!(parse_targets(&format!("[{one}]")).unwrap().len(), 1);
        assert!(parse_targets(&format!("[{one},{one}]")).is_err());
        assert!(parse_targets("nope").is_err());
    }
}
