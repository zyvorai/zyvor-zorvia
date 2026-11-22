// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Static service catalog. Deliberately carries no prices: pricing is set by
//! authorized commercial administrators on each quote.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Offering {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// Free / annual / monthly-or-annual / one-time / scoped / workshop.
    pub billing_model: &'static str,
    /// Can be requested through a quote request.
    pub requestable: bool,
    /// Whether the offering results in a support/managed entitlement
    /// (covered clusters, coverage hours) rather than a one-off project.
    pub grants_coverage: bool,
    pub included_scope: &'static [&'static str],
    pub prerequisites: &'static [&'static str],
    pub support_hours: &'static str,
    pub customer_responsibilities: &'static [&'static str],
    pub zyvor_responsibilities: &'static [&'static str],
    pub exclusions: &'static [&'static str],
    pub required_integrations: &'static [&'static str],
    pub maturity: &'static str,
}

pub static OFFERINGS: &[Offering] = &[
    Offering {
        id: "supported",
        name: "Supported",
        summary: "Technical assistance, upgrade guidance and agreed response times.",
        billing_model: "Annual contract",
        requestable: true,
        grants_coverage: true,
        included_scope: &[
            "Technical assistance for covered clusters",
            "Upgrade guidance",
            "Agreed response times",
        ],
        prerequisites: &[
            "A supported Kubernetes and KubeVirt version",
            "Named authorized support contacts",
        ],
        support_hours: "As stated in the contract (hours, timezone, holidays)",
        customer_responsibilities: &[
            "Operate the covered clusters",
            "Provide diagnostics when requested",
            "Keep authorized contacts current",
        ],
        zyvor_responsibilities: &[
            "Respond within the agreed targets",
            "Provide upgrade and troubleshooting guidance",
        ],
        exclusions: &[
            "Remote access to customer clusters",
            "Third-party software not shipped by Zyvor",
        ],
        required_integrations: &[],
        maturity: "Beta",
    },
    Offering {
        id: "supported_plus",
        name: "Supported Plus",
        summary: "Supported plus 24x7 response for Severity 1 and 2, priority handling, quarterly health reviews and planned upgrade assistance.",
        billing_model: "Annual contract",
        requestable: true,
        grants_coverage: true,
        included_scope: &[
            "Everything in Supported",
            "24x7 response for Severity 1 and 2",
            "Priority handling",
            "Quarterly health reviews",
            "Planned Zorvia upgrade assistance",
        ],
        prerequisites: &[
            "A supported Kubernetes and KubeVirt version",
            "Named authorized support contacts, including an out-of-hours contact",
        ],
        support_hours: "24x7 for Severity 1 and 2; business hours for Severity 3 and 4",
        customer_responsibilities: &[
            "Operate the covered clusters",
            "Provide diagnostics when requested",
            "Keep authorized contacts reachable out of hours",
        ],
        zyvor_responsibilities: &[
            "Respond within the agreed targets",
            "Run quarterly health reviews",
            "Assist with planned Zorvia upgrades",
        ],
        exclusions: &[
            "Remote access to customer clusters",
            "Third-party software not shipped by Zyvor",
            "Resolution deadlines",
        ],
        required_integrations: &[],
        maturity: "Beta",
    },
    Offering {
        id: "managed",
        name: "Managed",
        summary: "Supported plus agreed monitoring, maintenance, upgrades and recovery testing.",
        billing_model: "Monthly or annual contract",
        requestable: true,
        grants_coverage: true,
        included_scope: &[
            "Everything in Supported",
            "Agreed monitoring and maintenance windows",
            "Agreed upgrades",
            "Recovery testing",
        ],
        prerequisites: &[
            "Explicit enrollment of each covered cluster",
            "Separately enabled, scoped and revocable remote-operation credentials",
        ],
        support_hours: "As stated in the contract (hours, timezone, holidays)",
        customer_responsibilities: &[
            "Enroll clusters explicitly",
            "Grant and maintain scoped access",
            "Approve maintenance windows and remediation policies",
        ],
        zyvor_responsibilities: &[
            "Perform the agreed maintenance tasks",
            "Report incidents and service outcomes",
        ],
        exclusions: &[
            "Access to a cluster that has not been separately enabled",
            "Automated remediation without a documented policy and customer authorization",
        ],
        required_integrations: &["Remote-operation credentials (customer-issued)"],
        maturity: "Model only",
    },
    Offering {
        id: "deployment",
        name: "Deployment",
        summary: "Installation, integrations, configuration and acceptance testing.",
        billing_model: "One-time project",
        requestable: true,
        grants_coverage: false,
        included_scope: &[
            "Environment assessment",
            "Installation and integration",
            "Functional and recovery testing",
            "Administrator handover",
            "Acceptance",
        ],
        prerequisites: &["Target environment available for assessment"],
        support_hours: "Project schedule agreed in the quote",
        customer_responsibilities: &["Provide environment access and decisions on schedule"],
        zyvor_responsibilities: &["Deliver the milestones in the quote"],
        exclusions: &["Ongoing operations after acceptance"],
        required_integrations: &[],
        maturity: "Beta",
    },
    Offering {
        id: "migration",
        name: "Migration",
        summary: "Assessment, migration execution, guest validation and handover.",
        billing_model: "Scoped project",
        requestable: true,
        grants_coverage: false,
        included_scope: &[
            "Source inventory and assessment",
            "Network and storage mapping",
            "Pilot migration",
            "Approved migration waves",
            "Guest and application validation",
            "Handover",
        ],
        prerequisites: &["Read access to the source environment"],
        support_hours: "Project schedule agreed in the quote",
        customer_responsibilities: &[
            "Approve each migration wave",
            "Validate applications after cutover",
        ],
        zyvor_responsibilities: &["Deliver the milestones in the quote"],
        exclusions: &[
            "Guarantees about experimental migration APIs, which remain labelled experimental until verified",
        ],
        required_integrations: &["Source platform access"],
        maturity: "Beta",
    },
    Offering {
        id: "training",
        name: "Training",
        summary: "Administrator onboarding and operational workshops.",
        billing_model: "Workshop fee",
        requestable: true,
        grants_coverage: false,
        included_scope: &["Administrator onboarding", "Operational workshops"],
        prerequisites: &[],
        support_hours: "Sessions scheduled in the quote",
        customer_responsibilities: &["Provide attendees and a lab or test environment"],
        zyvor_responsibilities: &["Deliver the agreed sessions"],
        exclusions: &["Production changes made during workshops"],
        required_integrations: &[],
        maturity: "Beta",
    },
];

pub fn find(id: &str) -> Option<&'static Offering> {
    OFFERINGS.iter().find(|o| o.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_and_no_prices() {
        let mut ids: Vec<_> = OFFERINGS.iter().map(|o| o.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), OFFERINGS.len());
        let json = serde_json::to_string(&OFFERINGS).unwrap();
        assert!(!json.contains("price"));
    }

    #[test]
    fn supported_grants_coverage() {
        assert!(find("community").is_none());
        assert!(find("supported").unwrap().grants_coverage);
    }
}
