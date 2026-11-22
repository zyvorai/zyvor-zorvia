// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Expiry monitoring: alert once per threshold per certificate. The decision logic is
//! pure ([`evaluate`]); [`check_once`] adds persistence and the notification list.
//! The scheduler loop that calls it (leader-only) lives next to the API handlers.

use super::{CertInfo, CertSource, FileSpec};
use crate::notifications::{Notification, NotificationType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const DEFAULT_THRESHOLDS: [u32; 4] = [30, 14, 7, 1];
pub const MAX_THRESHOLDS: usize = 10;
pub const MAX_THRESHOLD_DAYS: u32 = 3650;
pub const DEFAULT_INTERVAL_SECS: u64 = 3600;
pub const MIN_INTERVAL_SECS: u64 = 60;
const ALERT_RETENTION_SECS: u64 = 30 * 24 * 3600;
/// State for a certificate not seen for this long is dropped.
const FORGET_AFTER_DAYS: i64 = 7;
pub const ALERT_SOURCE: &str = "cert-lifecycle";

/// Parse `30,14,7,1`: positive whole days, de-duplicated, largest first.
pub fn parse_thresholds(s: &str) -> Result<Vec<u32>, String> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let d: u32 = part
            .parse()
            .map_err(|_| format!("'{part}' is not a whole number of days"))?;
        if d == 0 || d > MAX_THRESHOLD_DAYS {
            return Err(format!(
                "threshold {d} must be between 1 and {MAX_THRESHOLD_DAYS}"
            ));
        }
        v.push(d);
    }
    if v.is_empty() {
        return Err("no thresholds given".into());
    }
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    if v.len() > MAX_THRESHOLDS {
        return Err(format!("at most {MAX_THRESHOLDS} thresholds"));
    }
    Ok(v)
}

/// `ZORVIA_CERT_ALERT_DAYS`, falling back (with a warning) to 30/14/7/1.
pub fn thresholds_from_env() -> Vec<u32> {
    match std::env::var("ZORVIA_CERT_ALERT_DAYS") {
        Ok(s) if !s.trim().is_empty() => parse_thresholds(&s).unwrap_or_else(|e| {
            log::warn!("ZORVIA_CERT_ALERT_DAYS ignored ({e}); using the default 30,14,7,1");
            DEFAULT_THRESHOLDS.to_vec()
        }),
        _ => DEFAULT_THRESHOLDS.to_vec(),
    }
}

/// `ZORVIA_CERT_CHECK_INTERVAL_SECS` (default 3600, minimum 60).
pub fn interval_from_env() -> u64 {
    std::env::var("ZORVIA_CERT_CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|s| s.max(MIN_INTERVAL_SECS))
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntryState {
    pub fingerprint: String,
    /// Thresholds (days) already crossed and reported for this certificate.
    pub fired: Vec<u32>,
    pub expired_alerted: bool,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExpiryState {
    pub entries: BTreeMap<String, EntryState>,
    pub last_check: Option<DateTime<Utc>>,
    pub last_certificates: usize,
    pub last_alerts: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CertAlert {
    pub level: NotificationType,
    pub message: String,
}

fn label(c: &CertInfo) -> String {
    match &c.namespace {
        Some(ns) => format!("{ns}/{} ({})", c.name, c.key),
        None => format!("{} ({})", c.name, c.key),
    }
}

/// Decide which alerts are due. At most one alert per certificate per call: when several
/// thresholds were crossed at once (first sight of a certificate that is already close to
/// expiry) all are marked fired and the tightest is reported.
pub fn evaluate(
    certs: &[CertInfo],
    state: &mut ExpiryState,
    thresholds: &[u32],
    now: DateTime<Utc>,
) -> Vec<CertAlert> {
    let mut alerts = Vec::new();
    for c in certs.iter().filter(|c| c.monitored()) {
        let id = c.id();
        let fp = &c.meta.fingerprint_sha256;
        let replaced = match state.entries.get(&id) {
            Some(e) if &e.fingerprint != fp => Some(!e.fired.is_empty() || e.expired_alerted),
            _ => None,
        };
        if let Some(had_alerts) = replaced {
            state.entries.remove(&id);
            if had_alerts && !c.meta.expired {
                alerts.push(CertAlert {
                    level: NotificationType::Success,
                    message: format!(
                        "Certificate {} was replaced; new expiry {}",
                        label(c),
                        c.meta.not_after.format("%Y-%m-%d")
                    ),
                });
            }
        }
        let entry = state.entries.entry(id).or_insert_with(|| EntryState {
            fingerprint: fp.clone(),
            fired: Vec::new(),
            expired_alerted: false,
            last_seen: now,
        });
        entry.last_seen = now;

        if c.meta.expired {
            if !entry.expired_alerted {
                entry.expired_alerted = true;
                entry.fired = thresholds.to_vec();
                alerts.push(CertAlert {
                    level: NotificationType::Error,
                    message: format!(
                        "Certificate {} has expired ({}, expired {})",
                        label(c),
                        c.meta.subject,
                        c.meta.not_after.format("%Y-%m-%d")
                    ),
                });
            }
            continue;
        }
        if c.meta.not_yet_valid {
            continue;
        }
        let remaining = c.meta.not_after.timestamp() - now.timestamp();
        let crossed: Vec<u32> = thresholds
            .iter()
            .copied()
            .filter(|t| remaining <= i64::from(*t) * 86_400)
            .collect();
        let tightest_new = crossed
            .iter()
            .copied()
            .filter(|t| !entry.fired.contains(t))
            .min();
        if let Some(t) = tightest_new {
            for x in crossed {
                if !entry.fired.contains(&x) {
                    entry.fired.push(x);
                }
            }
            let days = c.meta.days_remaining;
            alerts.push(CertAlert {
                level: if t <= 7 {
                    NotificationType::Error
                } else {
                    NotificationType::Warning
                },
                message: format!(
                    "Certificate {} expires in {days} day{} on {} (threshold {t} day{}; {})",
                    label(c),
                    if days == 1 { "" } else { "s" },
                    c.meta.not_after.format("%Y-%m-%d"),
                    if t == 1 { "" } else { "s" },
                    c.meta.subject
                ),
            });
        }
    }
    state
        .entries
        .retain(|_, e| now.signed_duration_since(e.last_seen).num_days() < FORGET_AFTER_DAYS);
    alerts
}

static STATE_LOCK: Mutex<()> = Mutex::new(());

pub fn state_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zorvia")
        .join("cert_expiry.json")
}

pub fn alerts_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zorvia")
        .join("cert_alerts.json")
}

pub fn load_state(path: &Path) -> ExpiryState {
    crate::state_docs::read(path)
        .and_then(|c| match serde_json::from_str(&c) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("Failed to parse certificate expiry state: {e}");
                None
            }
        })
        .unwrap_or_default()
}

fn save_state(path: &Path, s: &ExpiryState) -> anyhow::Result<()> {
    crate::state_docs::write(path, &serde_json::to_string_pretty(s)?)
}

/// Recent certificate alerts (persisted like the DR alerts), oldest first.
pub fn recent_alerts(path: &Path) -> Vec<Notification> {
    crate::dr::pairs::recent_alerts_from(path)
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckSummary {
    pub certificates: usize,
    pub alerts: usize,
}

/// One monitoring pass: collect, evaluate, persist state, record alerts.
pub async fn check_once(
    source: &impl CertSource,
    files: &[FileSpec],
    thresholds: &[u32],
    state_file: &Path,
    alerts_file: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<CheckSummary> {
    let inv = match super::collect(source, files, None, now).await {
        Ok(i) => i,
        Err(e) => {
            // Do not touch alert state on a failed scan (that would look like "all gone").
            let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut st = load_state(state_file);
            st.last_check = Some(now);
            st.last_error = Some(e.to_string());
            let _ = save_state(state_file, &st);
            return Err(e);
        }
    };
    let alerts = {
        let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = load_state(state_file);
        let alerts = evaluate(&inv.certificates, &mut st, thresholds, now);
        st.last_check = Some(now);
        st.last_certificates = inv.certificates.len();
        st.last_alerts = alerts.len();
        st.last_error = None;
        save_state(state_file, &st)?;
        alerts
    };
    for a in &alerts {
        match a.level {
            NotificationType::Error => log::error!("{}", a.message),
            NotificationType::Warning => log::warn!("{}", a.message),
            _ => log::info!("{}", a.message),
        }
        crate::dr::pairs::record_alert(
            alerts_file,
            Notification::new(&a.message, a.level.clone())
                .with_source(ALERT_SOURCE)
                .with_duration(ALERT_RETENTION_SECS),
        );
    }
    Ok(CheckSummary {
        certificates: inv.certificates.len(),
        alerts: alerts.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeSource;
    use super::super::inspect::testutil::*;
    use super::super::{build_inventory, RawSecret, SecretScan};
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    /// A certificate expiring `secs` after t0 (fixed key reuse not needed).
    fn secret(name: &str, expires_in_secs: i64) -> RawSecret {
        let k = key(&Key::EcP256);
        let p = pem(&cert(
            &Spec {
                cn: name,
                dns: &[],
                ip: None,
                not_before: t0().timestamp() - 86_400 * 100,
                not_after: t0().timestamp() + expires_in_secs,
                digest: None,
            },
            &k,
            None,
        ));
        RawSecret {
            namespace: "ns".into(),
            name: name.into(),
            tls_crt: Some(p),
            ca_crt: None,
        }
    }

    fn certs_at(raw: &RawSecret, now: DateTime<Utc>) -> Vec<CertInfo> {
        build_inventory(
            SecretScan {
                secrets: vec![raw.clone()],
                skipped: vec![],
            },
            &[],
            vec![],
            None,
            now,
        )
        .certificates
    }

    const D: i64 = 86_400;

    #[test]
    fn thresholds_parse_validate_sort_dedup() {
        assert_eq!(
            parse_thresholds("1, 30,14,7,7").unwrap(),
            vec![30, 14, 7, 1]
        );
        assert!(parse_thresholds("").is_err());
        assert!(parse_thresholds("0").is_err());
        assert!(parse_thresholds("abc").is_err());
        assert!(parse_thresholds("-5").is_err());
        assert!(parse_thresholds("99999").is_err());
        assert!(parse_thresholds(
            &(1..=11)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
        .is_err());
    }

    #[test]
    fn alerts_once_per_threshold_as_time_passes() {
        let raw = secret("web", 40 * D);
        let th = DEFAULT_THRESHOLDS;
        let mut st = ExpiryState::default();
        // 40 days left: nothing.
        assert!(evaluate(&certs_at(&raw, t0()), &mut st, &th, t0()).is_empty());
        // Crossing 30 days.
        let n = t0() + Duration::days(11);
        let a = evaluate(&certs_at(&raw, n), &mut st, &th, n);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].level, NotificationType::Warning);
        assert!(a[0].message.contains("threshold 30 days"));
        // Same state again and a bit later: no repeat.
        assert!(evaluate(&certs_at(&raw, n), &mut st, &th, n).is_empty());
        let n2 = n + Duration::days(2);
        assert!(evaluate(&certs_at(&raw, n2), &mut st, &th, n2).is_empty());
        // 14, then 7 (error level), then 1.
        let mut seen = vec![];
        for days in [27, 33, 39, 40] {
            let now = t0() + Duration::days(days) - Duration::hours(1);
            for al in evaluate(&certs_at(&raw, now), &mut st, &th, now) {
                seen.push(al);
            }
        }
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[0].message.contains("threshold 14 days"));
        assert!(
            seen[1].message.contains("threshold 7 days")
                && seen[1].level == NotificationType::Error
        );
        assert!(
            seen[2].message.contains("threshold 1 day)")
                || seen[2].message.contains("threshold 1 day;")
        );
        // Then expiry, once.
        let after = t0() + Duration::days(41);
        let a = evaluate(&certs_at(&raw, after), &mut st, &th, after);
        assert_eq!(a.len(), 1);
        assert!(a[0].message.contains("has expired"));
        assert!(evaluate(&certs_at(&raw, after), &mut st, &th, after).is_empty());
    }

    #[test]
    fn first_sight_close_to_expiry_gives_one_alert_for_the_tightest_threshold() {
        let raw = secret("late", 5 * D + 3600);
        let mut st = ExpiryState::default();
        let a = evaluate(&certs_at(&raw, t0()), &mut st, &DEFAULT_THRESHOLDS, t0());
        assert_eq!(a.len(), 1);
        assert!(a[0].message.contains("threshold 7 days"));
        assert!(evaluate(&certs_at(&raw, t0()), &mut st, &DEFAULT_THRESHOLDS, t0()).is_empty());
    }

    #[test]
    fn already_expired_alerts_once_and_replacement_resets() {
        let old = secret("rot", -2 * D);
        let mut st = ExpiryState::default();
        let a = evaluate(&certs_at(&old, t0()), &mut st, &DEFAULT_THRESHOLDS, t0());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].level, NotificationType::Error);
        assert!(evaluate(&certs_at(&old, t0()), &mut st, &DEFAULT_THRESHOLDS, t0()).is_empty());
        // The Secret is renewed (new certificate, same identity): an info alert, then fresh state.
        let renewed = secret("rot", 90 * D);
        let a = evaluate(
            &certs_at(&renewed, t0()),
            &mut st,
            &DEFAULT_THRESHOLDS,
            t0(),
        );
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].level, NotificationType::Success);
        let soon = t0() + Duration::days(65);
        let a = evaluate(
            &certs_at(&renewed, soon),
            &mut st,
            &DEFAULT_THRESHOLDS,
            soon,
        );
        assert_eq!(a.len(), 1);
        assert!(a[0].message.contains("threshold 30 days"));
    }

    #[test]
    fn unseen_entries_are_forgotten_after_a_week() {
        let raw = secret("gone", 3 * D);
        let mut st = ExpiryState::default();
        evaluate(&certs_at(&raw, t0()), &mut st, &DEFAULT_THRESHOLDS, t0());
        assert_eq!(st.entries.len(), 1);
        evaluate(&[], &mut st, &DEFAULT_THRESHOLDS, t0() + Duration::days(2));
        assert_eq!(st.entries.len(), 1);
        evaluate(&[], &mut st, &DEFAULT_THRESHOLDS, t0() + Duration::days(8));
        assert!(st.entries.is_empty());
    }

    #[test]
    fn custom_thresholds_are_honoured() {
        let raw = secret("c", 43 * D);
        let mut st = ExpiryState::default();
        assert!(evaluate(&certs_at(&raw, t0()), &mut st, &[45], t0()).len() == 1);
        let mut st2 = ExpiryState::default();
        assert!(evaluate(&certs_at(&raw, t0()), &mut st2, &[40], t0()).is_empty());
    }

    #[tokio::test]
    async fn check_once_persists_state_and_alerts_without_repeating() {
        let dir = tempfile::tempdir().unwrap();
        let (sf, af) = (
            dir.path().join("state.json"),
            dir.path().join("alerts.json"),
        );
        let fake = FakeSource {
            scan: SecretScan {
                secrets: vec![secret("db", 6 * D)],
                skipped: vec![],
            },
            ..Default::default()
        };
        let s = check_once(&fake, &[], &DEFAULT_THRESHOLDS, &sf, &af, t0())
            .await
            .unwrap();
        assert_eq!((s.certificates, s.alerts), (1, 1));
        let s = check_once(
            &fake,
            &[],
            &DEFAULT_THRESHOLDS,
            &sf,
            &af,
            t0() + Duration::hours(1),
        )
        .await
        .unwrap();
        assert_eq!(s.alerts, 0);
        let alerts = recent_alerts(&af);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].source, ALERT_SOURCE);
        assert!(alerts[0].message.contains("ns/db"));
        assert!(!alerts[0].message.contains("BEGIN"));
        let st = load_state(&sf);
        assert_eq!(st.last_certificates, 1);
        assert!(st.last_error.is_none());
    }

    #[tokio::test]
    async fn failed_scan_keeps_alert_state_and_records_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let (sf, af) = (
            dir.path().join("state.json"),
            dir.path().join("alerts.json"),
        );
        let ok = FakeSource {
            scan: SecretScan {
                secrets: vec![secret("db", 6 * D)],
                skipped: vec![],
            },
            ..Default::default()
        };
        check_once(&ok, &[], &DEFAULT_THRESHOLDS, &sf, &af, t0())
            .await
            .unwrap();
        let bad = FakeSource {
            scan_error: Some("api down".into()),
            ..Default::default()
        };
        assert!(check_once(&bad, &[], &DEFAULT_THRESHOLDS, &sf, &af, t0())
            .await
            .is_err());
        let st = load_state(&sf);
        assert_eq!(st.entries.len(), 1);
        assert_eq!(st.last_error.as_deref(), Some("api down"));
        // Recovery does not re-alert.
        let s = check_once(&ok, &[], &DEFAULT_THRESHOLDS, &sf, &af, t0())
            .await
            .unwrap();
        assert_eq!(s.alerts, 0);
    }
}
