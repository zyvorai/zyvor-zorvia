// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Draft,
    AwaitingAcceptance,
    Active,
    Expired,
    Cancelled,
}

impl ContractStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AwaitingAcceptance => "awaiting_acceptance",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => Self::Draft,
            "awaiting_acceptance" => Self::AwaitingAcceptance,
            "active" => Self::Active,
            "expired" => Self::Expired,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// `draft → awaiting_acceptance → active → expired/cancelled`, plus
    /// returning a proposal to draft for revision and cancelling before
    /// activation. Expired and cancelled are terminal (renewal is a new
    /// contract).
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::AwaitingAcceptance)
                | (Self::Draft, Self::Cancelled)
                | (Self::AwaitingAcceptance, Self::Draft)
                | (Self::AwaitingAcceptance, Self::Active)
                | (Self::AwaitingAcceptance, Self::Cancelled)
                | (Self::Active, Self::Expired)
                | (Self::Active, Self::Cancelled)
        )
    }
}

/// Payment state is tracked separately from contract/service status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    NotInvoiced,
    Invoiced,
    Paid,
    Overdue,
    Waived,
}

impl PaymentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInvoiced => "not_invoiced",
            Self::Invoiced => "invoiced",
            Self::Paid => "paid",
            Self::Overdue => "overdue",
            Self::Waived => "waived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "not_invoiced" => Self::NotInvoiced,
            "invoiced" => Self::Invoiced,
            "paid" => Self::Paid,
            "overdue" => Self::Overdue,
            "waived" => Self::Waived,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CoverageHours {
    /// True for 24x7; otherwise `days`/`start`/`end` apply.
    #[serde(default)]
    pub always: bool,
    /// ISO weekdays, 1 = Monday .. 7 = Sunday.
    #[serde(default)]
    pub days: Vec<u8>,
    /// "HH:MM" local time in the entitlement timezone.
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupportContact {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Entitlement {
    #[serde(default)]
    pub covered_clusters: Vec<String>,
    /// `supported` or `managed` for coverage contracts; empty for projects.
    #[serde(default)]
    pub support_tier: String,
    #[serde(default)]
    pub coverage_hours: CoverageHours,
    /// IANA timezone name, e.g. `Europe/Berlin`.
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub authorized_contacts: Vec<SupportContact>,
    #[serde(default)]
    pub effective_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    /// Included worker-node allowance across covered clusters.
    #[serde(default)]
    pub node_allowance: Option<u32>,
    /// e.g. `monitoring`, `maintenance`, `upgrades`. Granting one here
    /// records what was purchased; it does not grant cluster access.
    #[serde(default)]
    pub managed_permissions: Vec<String>,
    /// First-response targets in *coverage minutes*, keyed `sev1`..`sev4`.
    /// A severity with no entry has no response timer.
    #[serde(default)]
    pub response_target_minutes: BTreeMap<String, u32>,
    /// Non-binding resolution estimates in coverage minutes, same keys.
    #[serde(default)]
    pub resolution_estimate_minutes: BTreeMap<String, u32>,
    /// Severities covered 24x7 instead of `coverage_hours` (e.g. Supported
    /// Plus: `sev1`, `sev2`).
    #[serde(default)]
    pub always_on_severities: Vec<String>,
    /// Contract holidays as `YYYY-MM-DD` in the entitlement timezone. They
    /// suspend business-hours coverage only, never 24x7 severities.
    #[serde(default)]
    pub holidays: Vec<String>,
}

pub const SEVERITIES: [&str; 4] = ["sev1", "sev2", "sev3", "sev4"];

impl Entitlement {
    /// Structural checks that apply whenever an entitlement is written.
    pub fn validate(&self) -> Result<(), String> {
        if !self.support_tier.is_empty()
            && !matches!(
                self.support_tier.as_str(),
                "supported" | "supported_plus" | "managed"
            )
        {
            return Err("support_tier must be 'supported', 'supported_plus' or 'managed'".into());
        }
        if !self.timezone.is_empty() && !valid_timezone_name(&self.timezone) {
            return Err("timezone must be an IANA name such as 'Europe/Berlin'".into());
        }
        let h = &self.coverage_hours;
        if !h.always && (!h.days.is_empty() || !h.start.is_empty() || !h.end.is_empty()) {
            if h.days.is_empty() || h.days.iter().any(|d| !(1..=7).contains(d)) {
                return Err("coverage_hours.days must be ISO weekdays 1..7".into());
            }
            let (Some(s), Some(e)) = (parse_hhmm(&h.start), parse_hhmm(&h.end)) else {
                return Err("coverage_hours.start/end must be HH:MM".into());
            };
            if s >= e {
                return Err("coverage_hours.start must be before end".into());
            }
        }
        for map in [
            &self.response_target_minutes,
            &self.resolution_estimate_minutes,
        ] {
            for (sev, mins) in map {
                if !SEVERITIES.contains(&sev.as_str()) || *mins == 0 || *mins > 525_600 {
                    return Err("targets must be keyed sev1..sev4 with 1..525600 minutes".into());
                }
            }
        }
        if self
            .always_on_severities
            .iter()
            .any(|s| !SEVERITIES.contains(&s.as_str()))
        {
            return Err("always_on_severities must be from sev1..sev4".into());
        }
        for h in &self.holidays {
            if chrono::NaiveDate::parse_from_str(h, "%Y-%m-%d").is_err() {
                return Err("holidays must be YYYY-MM-DD".into());
            }
        }
        for c in &self.covered_clusters {
            if c.trim().is_empty() || c.len() > 128 {
                return Err("covered cluster identifiers must be 1-128 characters".into());
            }
        }
        for c in &self.authorized_contacts {
            if c.name.trim().is_empty() || !c.email.contains('@') {
                return Err("authorized contacts need a name and an email address".into());
            }
        }
        if let (Some(a), Some(b)) = (self.effective_at, self.expires_at) {
            if b <= a {
                return Err("expires_at must be after effective_at".into());
            }
        }
        Ok(())
    }

    /// Extra requirements before a contract may become active.
    pub fn validate_for_activation(&self, grants_coverage: bool) -> Result<(), String> {
        self.validate()?;
        if self.effective_at.is_none() || self.expires_at.is_none() {
            return Err("effective_at and expires_at are required before activation".into());
        }
        if grants_coverage {
            if self.support_tier.is_empty() {
                return Err("support_tier is required for coverage contracts".into());
            }
            if self.covered_clusters.is_empty() {
                return Err("at least one covered cluster is required".into());
            }
            if self.timezone.is_empty() {
                return Err("timezone is required for coverage contracts".into());
            }
            if !self.coverage_hours.always && self.coverage_hours.days.is_empty() {
                return Err("coverage_hours are required for coverage contracts".into());
            }
        }
        Ok(())
    }
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && s.len() == 5).then_some(h * 60 + m)
}

/// True for any IANA timezone name known to the bundled tz database.
pub fn valid_timezone_name(s: &str) -> bool {
    s.len() <= 64 && s.parse::<chrono_tz::Tz>().is_ok()
}

/// The reviewable quote produced from a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub offering: String,
    pub services_covered: Vec<String>,
    pub infrastructure_covered: String,
    pub included_capacity: String,
    pub pricing_unit: String,
    /// Minor currency units (cents). `None` until a commercial administrator
    /// sets it; nothing is priced by default.
    pub unit_price_minor: Option<i64>,
    pub currency: String,
    pub duration_months: u32,
    pub response_targets: Vec<String>,
    pub support_hours: String,
    pub customer_responsibilities: Vec<String>,
    pub zyvor_responsibilities: Vec<String>,
    pub exclusions: Vec<String>,
    pub required_integrations: Vec<String>,
    /// How control-plane / infra-only / temporary / disconnected nodes are
    /// treated (see docs/BILLING_UNITS.md); recorded per contract.
    #[serde(default)]
    pub node_treatment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contract {
    pub id: String,
    pub org_id: String,
    pub quote_request_id: Option<String>,
    pub offering: String,
    /// Stored status.
    pub status: ContractStatus,
    /// Status after applying expiry to the current time; what callers see.
    pub effective_status: ContractStatus,
    pub payment_status: PaymentStatus,
    pub source: String,
    pub quote: Quote,
    pub entitlement: Entitlement,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Renewal note shown once a coverage contract has expired.
    pub renewal: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractEvent {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub event: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub id: String,
    pub org_id: Option<String>,
    pub requested_by: String,
    pub offering: String,
    pub company: String,
    pub contact_name: String,
    pub contact_email: String,
    pub cluster_count: u32,
    pub worker_node_count: u32,
    pub workload_size: String,
    pub region: String,
    pub desired_coverage: String,
    pub requirements: String,
    /// `submitted` → `quoted` | `declined`.
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Per-cluster coverage answer. `eligible` reflects commercial service
/// eligibility only; VM operation never depends on it.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageEntry {
    pub cluster_id: String,
    pub contract_id: String,
    pub org_id: String,
    pub offering: String,
    pub support_tier: String,
    pub status: ContractStatus,
    pub eligible: bool,
    pub effective_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub coverage_hours: CoverageHours,
    pub node_allowance: Option<u32>,
    pub managed_permissions: Vec<String>,
    pub renewal: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_follow_lifecycle() {
        use ContractStatus::*;
        assert!(Draft.can_transition_to(AwaitingAcceptance));
        assert!(AwaitingAcceptance.can_transition_to(Active));
        assert!(Active.can_transition_to(Expired));
        assert!(Active.can_transition_to(Cancelled));
        assert!(!Draft.can_transition_to(Active));
        assert!(!Expired.can_transition_to(Active));
        assert!(!Cancelled.can_transition_to(Draft));
        assert!(!Active.can_transition_to(Draft));
    }

    #[test]
    fn status_roundtrip() {
        for s in [
            ContractStatus::Draft,
            ContractStatus::AwaitingAcceptance,
            ContractStatus::Active,
            ContractStatus::Expired,
            ContractStatus::Cancelled,
        ] {
            assert_eq!(ContractStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn timezone_shape() {
        assert!(valid_timezone_name("Europe/Berlin"));
        assert!(valid_timezone_name("America/Argentina/Buenos_Aires"));
        assert!(valid_timezone_name("UTC"));
        assert!(!valid_timezone_name("Berlin"));
        assert!(!valid_timezone_name("../etc/passwd"));
        assert!(!valid_timezone_name("Europe//Berlin"));
    }

    #[test]
    fn coverage_hours_validation() {
        let mut e = Entitlement {
            timezone: "Europe/Berlin".into(),
            coverage_hours: CoverageHours {
                always: false,
                days: vec![1, 2, 3, 4, 5],
                start: "09:00".into(),
                end: "17:00".into(),
            },
            ..Default::default()
        };
        assert!(e.validate().is_ok());
        e.coverage_hours.end = "08:00".into();
        assert!(e.validate().is_err());
        e.coverage_hours.end = "17:00".into();
        e.coverage_hours.days = vec![0];
        assert!(e.validate().is_err());
    }

    #[test]
    fn activation_requires_dates_and_coverage_fields() {
        let e = Entitlement::default();
        assert!(e.validate_for_activation(false).is_err());
        let e = Entitlement {
            effective_at: Some(Utc::now()),
            expires_at: Some(Utc::now() + chrono::Duration::days(365)),
            ..Default::default()
        };
        assert!(e.validate_for_activation(false).is_ok());
        assert!(e.validate_for_activation(true).is_err());
    }
}
