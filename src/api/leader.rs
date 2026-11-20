// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Kubernetes Lease-based leader election for background schedulers.
//!
//! Enabled when `ZORVIA_LEADER_ELECTION=1`. Enabled election fails closed:
//! API/configuration errors never grant leadership. Disabled election is for
//! single-replica deployments only.

use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::Client;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static LEADERSHIP: Mutex<(bool, Option<Instant>)> = Mutex::new((false, None));

pub fn is_leader() -> bool {
    let (held, deadline) = *LEADERSHIP.lock().unwrap_or_else(|e| e.into_inner());
    leadership_valid(held, deadline, Instant::now())
}

fn leadership_valid(held: bool, deadline: Option<Instant>, now: Instant) -> bool {
    held && deadline.is_none_or(|until| now < until)
}

fn set_leader(held: bool, deadline: Option<Instant>) -> bool {
    let mut leadership = LEADERSHIP.lock().unwrap_or_else(|e| e.into_inner());
    let previous = leadership.0;
    *leadership = (held, deadline);
    previous
}

fn env_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

/// Spawn a background renew loop when leader election is enabled.
pub fn spawn_leader_election() {
    if !env_truthy("ZORVIA_LEADER_ELECTION") {
        log::info!("Leader election disabled; this process runs schedulers");
        set_leader(true, None);
        return;
    }
    set_leader(false, None);
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_election_loop().await {
                set_leader(false, None);
                log::error!("Leader election failed ({e}); schedulers remain paused; retrying");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn run_election_loop() -> anyhow::Result<()> {
    let client = Client::try_default().await?;
    let ns = std::env::var("ZORVIA_LEADER_NAMESPACE")
        .or_else(|_| std::env::var("POD_NAMESPACE"))
        .unwrap_or_else(|_| "zorvia-system".into());
    let lease_name =
        std::env::var("ZORVIA_LEADER_LEASE").unwrap_or_else(|_| "zorvia-schedulers".into());
    let identity =
        std::env::var("POD_NAME").unwrap_or_else(|_| format!("zorvia-{}", uuid::Uuid::new_v4()));
    let api: Api<Lease> = Api::namespaced(client, &ns);
    let lease_duration = Duration::from_secs(15);
    let renew_period = Duration::from_secs(5);

    log::info!(
        "Leader election: lease={}/{} identity={}",
        ns,
        lease_name,
        identity
    );

    loop {
        // Bound API waits and expire local authority even if the renew task
        // stalls. The deadline begins before the request, with a safety margin.
        let valid_until = Instant::now() + lease_duration - Duration::from_secs(1);
        let renewed = match tokio::time::timeout(
            renew_period,
            try_acquire_or_renew(&api, &lease_name, &identity, lease_duration),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("scheduler Lease request timed out")),
        };
        match renewed {
            Ok(true) => {
                if !set_leader(true, Some(valid_until)) {
                    log::info!("Acquired scheduler leadership ({identity})");
                }
            }
            Ok(false) => {
                if set_leader(false, None) {
                    log::info!("Lost scheduler leadership ({identity})");
                }
            }
            Err(e) => {
                log::warn!("Lease renew error: {e}");
                set_leader(false, None);
            }
        }
        tokio::time::sleep(renew_period).await;
    }
}

fn format_k8s_micro_time(ts: k8s_openapi::jiff::Timestamp) -> String {
    // Kubernetes MicroTime parsing expects RFC3339 with *exactly* 6 fractional
    // digits (`.000000`). jiff's Display omits trailing zeros, which the
    // apiserver rejects.
    let s = ts.to_string();
    let body = s.trim_end_matches('Z');
    if let Some((date, frac)) = body.split_once('.') {
        let mut digits: String = frac
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(6)
            .collect();
        while digits.len() < 6 {
            digits.push('0');
        }
        format!("{date}.{digits}Z")
    } else {
        format!("{body}.000000Z")
    }
}

fn micro_now() -> k8s_openapi::jiff::Timestamp {
    let t = k8s_openapi::jiff::Timestamp::now();
    k8s_openapi::jiff::Timestamp::from_microsecond(t.as_microsecond()).unwrap_or(t)
}

async fn try_acquire_or_renew(
    api: &Api<Lease>,
    name: &str,
    identity: &str,
    lease_duration: Duration,
) -> anyhow::Result<bool> {
    let now = micro_now();
    match api.get(name).await {
        Ok(existing) => {
            let holder = existing
                .spec
                .as_ref()
                .and_then(|s| s.holder_identity.as_deref())
                .unwrap_or("");
            if may_acquire(&existing, identity, now) {
                let resource_version = existing
                    .metadata
                    .resource_version
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("scheduler Lease has no resourceVersion"))?;
                let renew = format_k8s_micro_time(now);
                let acquire = existing
                    .spec
                    .as_ref()
                    .and_then(|s| s.acquire_time.as_ref())
                    .filter(|_| holder == identity)
                    .map(|t| format_k8s_micro_time(t.0))
                    .unwrap_or_else(|| renew.clone());
                let patch = serde_json::json!({
                    "metadata": { "resourceVersion": resource_version },
                    "spec": {
                        "holderIdentity": identity,
                        "leaseDurationSeconds": lease_duration.as_secs() as i32,
                        "renewTime": renew,
                        "acquireTime": acquire,
                    }
                });
                api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            let lease = Lease {
                metadata: kube::api::ObjectMeta {
                    name: Some(name.to_string()),
                    ..Default::default()
                },
                spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                    holder_identity: Some(identity.to_string()),
                    lease_duration_seconds: Some(lease_duration.as_secs() as i32),
                    acquire_time: Some(MicroTime(now)),
                    renew_time: Some(MicroTime(now)),
                    ..Default::default()
                }),
            };
            api.create(&PostParams::default(), &lease).await?;
            Ok(true)
        }
        Err(e) => Err(e.into()),
    }
}

/// Respect the current holder's declared duration, not our desired duration.
/// A foreign holder with incomplete timing data cannot be safely displaced.
fn may_acquire(lease: &Lease, identity: &str, now: k8s_openapi::jiff::Timestamp) -> bool {
    let Some(spec) = &lease.spec else {
        return true;
    };
    let holder = spec.holder_identity.as_deref().unwrap_or("");
    if holder.is_empty() || holder == identity {
        return true;
    }
    match (&spec.renew_time, spec.lease_duration_seconds) {
        (Some(renew), Some(seconds)) if seconds > 0 => {
            now.as_microsecond()
                > renew
                    .0
                    .as_microsecond()
                    .saturating_add(i64::from(seconds) * 1_000_000)
        }
        _ => false,
    }
}

/// Helper used by scheduler loops: skip a tick when not leader.
pub fn skip_if_follower(task: &str) -> bool {
    if is_leader() {
        return false;
    }
    log::trace!("Skipping {task}: not lease leader");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(seconds: Option<i32>, renew: Option<i64>) -> Lease {
        Lease {
            spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                holder_identity: Some("other-pod".into()),
                lease_duration_seconds: seconds,
                renew_time: renew
                    .map(|t| MicroTime(k8s_openapi::jiff::Timestamp::from_microsecond(t).unwrap())),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn honors_foreign_duration_and_exact_expiry_boundary() {
        let l = lease(Some(60), Some(1_000_000));
        let at = |t| k8s_openapi::jiff::Timestamp::from_microsecond(t).unwrap();
        assert!(!may_acquire(&l, "new-pod", at(20_000_000)));
        assert!(!may_acquire(&l, "new-pod", at(61_000_000)));
        assert!(may_acquire(&l, "new-pod", at(61_000_001)));
        assert!(may_acquire(&l, "other-pod", at(2_000_000)));
    }

    #[test]
    fn malformed_foreign_timing_does_not_grant_leadership() {
        let now = k8s_openapi::jiff::Timestamp::from_microsecond(100_000_000).unwrap();
        for l in [
            lease(None, Some(0)),
            lease(Some(0), Some(0)),
            lease(Some(-1), Some(0)),
            lease(Some(15), None),
        ] {
            assert!(!may_acquire(&l, "new-pod", now));
        }
        assert!(may_acquire(&Lease::default(), "new-pod", now));
    }

    #[test]
    fn local_authority_expires_even_if_renew_loop_stalls() {
        let now = Instant::now();
        assert!(leadership_valid(
            true,
            Some(now + Duration::from_secs(1)),
            now
        ));
        assert!(!leadership_valid(true, Some(now), now));
        assert!(!leadership_valid(false, None, now));
        assert!(leadership_valid(true, None, now)); // explicitly disabled election
    }
}
