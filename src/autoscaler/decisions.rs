// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Bounded in-memory record of the autoscaler loop's recent decisions, for the read-only
//! status/decisions API, CLI and web panel. See docs/AUTOSCALER_LOOP.md.
//!
//! Memory only: it is lost on restart (not persisted) and is per process (a follower replica
//! has an empty log because only the leader ticks). Records carry no secrets: target ids,
//! numbers, a metric label, a load-source label and a short reason string.

use super::reconcile::{LoadMetric, LoopTarget, TargetKind, MAX_TARGETS};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

/// Records kept per target; the oldest is dropped first.
pub const MAX_PER_TARGET: usize = 100;
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 500;
const MAX_REASON: usize = 200;

/// What the loop decided on one tick for one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    ScaleUp,
    ScaleDown,
    /// Evaluated and deliberately did nothing (dead band, cooldown, bound, ...).
    Hold,
    /// Could not evaluate (no usable load or size).
    Skipped,
}

/// What became of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    Recommended,
    Failed,
    Held,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionRecord {
    /// Monotonic across all targets; defines the order.
    pub seq: u64,
    /// Time of the latest tick this record covers.
    pub timestamp: DateTime<Utc>,
    pub target: String,
    pub metric: &'static str,
    pub load_percent: Option<f64>,
    /// `metrics_api`, `prometheus`, `simulated`, or `unknown` when no sample was read.
    pub source: &'static str,
    pub decision: DecisionKind,
    pub outcome: Outcome,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub reason: String,
    /// Consecutive identical hold/skip ticks folded into this record (1 = a single tick).
    pub count: u32,
}

/// Static policy and live hysteresis state of one target, for the status view.
#[derive(Debug, Clone, Serialize)]
pub struct TargetStatus {
    pub id: String,
    pub kind: TargetKind,
    pub namespace: String,
    pub name: String,
    pub load_metric: LoadMetric,
    pub min: u32,
    pub max: u32,
    pub step: u32,
    pub scale_up_threshold: f64,
    pub scale_down_threshold: f64,
    pub required_samples: u32,
    pub cooldown_secs: u64,
    pub up_streak: u32,
    pub down_streak: u32,
    pub last_action: Option<DateTime<Utc>>,
}

impl TargetStatus {
    fn from_target(t: &LoopTarget) -> Self {
        Self {
            id: t.id.clone(),
            kind: t.kind,
            namespace: t.namespace.clone(),
            name: t.name.clone(),
            load_metric: t.load_metric,
            min: t.min,
            max: t.max,
            step: t.step,
            scale_up_threshold: t.scale_up_threshold,
            scale_down_threshold: t.scale_down_threshold,
            required_samples: t.required_samples,
            cooldown_secs: t.cooldown_secs,
            up_streak: 0,
            down_streak: 0,
            last_action: None,
        }
    }
}

/// Loop-level status.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LoopStatus {
    /// True once the loop task has started in this process.
    pub running: bool,
    /// `apply` or `recommend_only`; absent until the loop starts.
    pub mode: Option<&'static str>,
    pub interval_secs: Option<u64>,
    /// Whether this replica held the lease on the last tick.
    pub leader: Option<bool>,
    pub last_tick: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    status: LoopStatus,
    targets: Vec<TargetStatus>,
    rings: HashMap<String, VecDeque<DecisionRecord>>,
}

#[derive(Default)]
pub struct DecisionLog {
    inner: Mutex<Inner>,
}

/// The process-wide log shared by the loop and the API.
pub fn global() -> &'static Arc<DecisionLog> {
    static LOG: OnceLock<Arc<DecisionLog>> = OnceLock::new();
    LOG.get_or_init(|| Arc::new(DecisionLog::default()))
}

/// One line, bounded, with URLs removed, so a load-source error cannot leak an endpoint.
pub fn sanitize_reason(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    let cleaned: Vec<&str> = line
        .split_whitespace()
        .map(|w| if w.contains("://") { "<url>" } else { w })
        .collect();
    cleaned.join(" ").chars().take(MAX_REASON).collect()
}

/// One tick's observation for one target (input to [`DecisionLog::push`]).
pub struct Observation<'a> {
    pub now: DateTime<Utc>,
    pub target: &'a LoopTarget,
    pub load_percent: Option<f64>,
    pub source: &'static str,
    pub decision: DecisionKind,
    pub outcome: Outcome,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub reason: &'a str,
}

impl DecisionLog {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock only means a panic elsewhere; the data is still plain values.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called once when the loop starts: registers the targets and loop settings.
    pub fn start(&self, targets: &[LoopTarget], apply: bool, interval_secs: u64) {
        let mut g = self.lock();
        g.targets = targets
            .iter()
            .take(MAX_TARGETS)
            .map(TargetStatus::from_target)
            .collect();
        g.status = LoopStatus {
            running: true,
            mode: Some(if apply { "apply" } else { "recommend_only" }),
            interval_secs: Some(interval_secs),
            leader: None,
            last_tick: None,
        };
    }

    pub fn note_tick(&self, now: DateTime<Utc>, leader: bool) {
        let mut g = self.lock();
        g.status.leader = Some(leader);
        g.status.last_tick = Some(now);
    }

    /// Copies a target's hysteresis counters for the status view.
    pub fn note_state(&self, id: &str, up: u32, down: u32, last_action: Option<DateTime<Utc>>) {
        let mut g = self.lock();
        if let Some(t) = g.targets.iter_mut().find(|t| t.id == id) {
            t.up_streak = up;
            t.down_streak = down;
            t.last_action = last_action;
        }
    }

    /// Appends a record. Consecutive identical hold/skip observations are folded into the
    /// previous record (timestamp, load and count updated) so steady-state ticks cannot push
    /// real scale decisions out of the ring.
    pub fn push(&self, o: Observation<'_>) {
        let mut g = self.lock();
        if !g.rings.contains_key(&o.target.id) && g.rings.len() >= MAX_TARGETS {
            return;
        }
        g.seq += 1;
        let seq = g.seq;
        let reason = sanitize_reason(o.reason);
        let ring = g.rings.entry(o.target.id.clone()).or_default();
        let foldable = matches!(o.outcome, Outcome::Held | Outcome::Skipped);
        if foldable {
            if let Some(last) = ring.back_mut() {
                if last.outcome == o.outcome && last.reason == reason {
                    last.seq = seq;
                    last.timestamp = o.now;
                    last.load_percent = o.load_percent;
                    last.source = o.source;
                    last.count = last.count.saturating_add(1);
                    return;
                }
            }
        }
        if ring.len() >= MAX_PER_TARGET {
            ring.pop_front();
        }
        ring.push_back(DecisionRecord {
            seq,
            timestamp: o.now,
            target: o.target.id.clone(),
            metric: match o.target.load_metric {
                LoadMetric::Cpu => "cpu",
                LoadMetric::Memory => "memory",
            },
            load_percent: o.load_percent,
            source: o.source,
            decision: o.decision,
            outcome: o.outcome,
            from: o.from,
            to: o.to,
            reason,
            count: 1,
        });
    }

    /// Newest first across all targets (or one `target`), at most `limit` (clamped to
    /// `1..=MAX_LIMIT`).
    pub fn recent(&self, limit: usize, target: Option<&str>) -> Vec<DecisionRecord> {
        let limit = limit.clamp(1, MAX_LIMIT);
        let g = self.lock();
        let mut all: Vec<DecisionRecord> = g
            .rings
            .iter()
            .filter(|(id, _)| target.is_none_or(|t| t == id.as_str()))
            .flat_map(|(_, r)| r.iter().cloned())
            .collect();
        all.sort_by_key(|r| std::cmp::Reverse(r.seq));
        all.truncate(limit);
        all
    }

    pub fn status(&self) -> (LoopStatus, Vec<TargetStatus>) {
        let g = self.lock();
        (g.status.clone(), g.targets.clone())
    }

    pub fn knows_target(&self, id: &str) -> bool {
        self.lock().targets.iter().any(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str) -> LoopTarget {
        LoopTarget {
            id: id.into(),
            kind: TargetKind::VmPool,
            namespace: "ns".into(),
            name: "pool".into(),
            metric_vms: vec!["vm-a".into()],
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

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn obs<'a>(t: &'a LoopTarget, i: i64, outcome: Outcome, reason: &'a str) -> Observation<'a> {
        Observation {
            now: t0() + chrono::Duration::seconds(i),
            target: t,
            load_percent: Some(i as f64),
            source: "metrics_api",
            decision: DecisionKind::ScaleUp,
            outcome,
            from: Some(1),
            to: Some(2),
            reason,
        }
    }

    #[test]
    fn ring_is_bounded_per_target_and_drops_oldest() {
        let log = DecisionLog::default();
        let t = target("a");
        for i in 0..(MAX_PER_TARGET as i64 + 25) {
            log.push(obs(&t, i, Outcome::Recommended, "r"));
        }
        let all = log.recent(MAX_LIMIT, Some("a"));
        assert_eq!(all.len(), MAX_PER_TARGET);
        // newest kept, oldest dropped
        assert_eq!(all[0].load_percent, Some((MAX_PER_TARGET + 24) as f64));
        assert_eq!(all.last().unwrap().load_percent, Some(25.0));
    }

    #[test]
    fn one_noisy_target_does_not_evict_another() {
        let log = DecisionLog::default();
        let (a, b) = (target("a"), target("b"));
        log.push(obs(&b, 0, Outcome::Applied, "ok"));
        for i in 0..300 {
            log.push(obs(&a, i, Outcome::Recommended, "r"));
        }
        assert_eq!(log.recent(MAX_LIMIT, Some("b")).len(), 1);
    }

    #[test]
    fn recent_is_newest_first_across_targets_and_filters() {
        let log = DecisionLog::default();
        let (a, b) = (target("a"), target("b"));
        log.push(obs(&a, 1, Outcome::Recommended, "x"));
        log.push(obs(&b, 2, Outcome::Applied, "y"));
        log.push(obs(&a, 3, Outcome::Failed, "z"));
        let all = log.recent(10, None);
        let seqs: Vec<u64> = all.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![3, 2, 1]);
        assert_eq!(log.recent(2, None).len(), 2);
        let only_a = log.recent(10, Some("a"));
        assert!(only_a.iter().all(|r| r.target == "a") && only_a.len() == 2);
        assert!(log.recent(10, Some("nope")).is_empty());
        // limit is clamped to at least 1 and at most MAX_LIMIT
        assert_eq!(log.recent(0, None).len(), 1);
    }

    #[test]
    fn identical_holds_fold_and_real_decisions_survive() {
        let log = DecisionLog::default();
        let t = target("a");
        log.push(obs(&t, 0, Outcome::Applied, "applied"));
        for i in 1..=500 {
            log.push(obs(&t, i, Outcome::Held, "cooldown"));
        }
        let all = log.recent(MAX_LIMIT, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].outcome, Outcome::Held);
        assert_eq!(all[0].count, 500);
        assert_eq!(all[0].timestamp, t0() + chrono::Duration::seconds(500));
        assert_eq!(all[1].outcome, Outcome::Applied);
        // a different hold reason starts a new record
        log.push(obs(&t, 501, Outcome::Held, "at bound"));
        assert_eq!(log.recent(MAX_LIMIT, None).len(), 3);
    }

    #[test]
    fn target_count_is_bounded() {
        let log = DecisionLog::default();
        for i in 0..(MAX_TARGETS + 10) {
            let t = target(&format!("t{i}"));
            log.push(obs(&t, 0, Outcome::Held, "r"));
        }
        assert_eq!(
            log.recent(MAX_LIMIT, None).len(),
            MAX_LIMIT.min(MAX_TARGETS)
        );
    }

    #[test]
    fn reasons_are_single_line_bounded_and_url_free() {
        let s = sanitize_reason(
            "load unavailable: GET https://user:pw@prom.example/api?q=1 failed\nsecond line",
        );
        assert_eq!(s, "load unavailable: GET <url> failed");
        assert!(sanitize_reason(&"x".repeat(1000)).len() <= MAX_REASON);
    }

    #[test]
    fn status_tracks_loop_and_target_state() {
        let log = DecisionLog::default();
        assert!(!log.status().0.running);
        let t = target("a");
        log.start(std::slice::from_ref(&t), false, 60);
        log.note_tick(t0(), true);
        log.note_state("a", 2, 0, Some(t0()));
        let (s, ts) = log.status();
        assert!(s.running && s.leader == Some(true));
        assert_eq!(s.mode, Some("recommend_only"));
        assert_eq!(ts[0].up_streak, 2);
        assert!(log.knows_target("a") && !log.knows_target("b"));
    }
}
