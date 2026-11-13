# Comparison with OpenShift Virtualization Engine

> Zorvia prices are in [PRICING.md](PRICING.md). This page publishes **no OpenShift prices and no percentage savings**. Cost comparisons are made only against a comparable, customer-specific quote.

## Positioning

> Zorvia provides a focused VM operations platform for compatible KubeVirt infrastructure, with free Apache-2.0 software and optional commercial services.

The primary comparator is OpenShift Virtualization Engine. Zorvia does not imply equivalent certification, feature maturity or support coverage. Where OpenShift is the better fit, [leave-openshift.md](leave-openshift.md) says so.

## Worksheet

Fill [OVE_COST_WORKSHEET.md](OVE_COST_WORKSHEET.md) or run
`./scripts/ove-cost-worksheet.sh <nodes> <inr|usd> <supported|supported-plus> [ove_year1]`.
It estimates Zorvia support from [PRICING.md](PRICING.md) and optionally subtracts
that from **your** OVE year-1 figure. It does not publish OpenShift prices.

## How to compare: use a customer-specific proposal

Put both proposals side by side on the same assumptions:

| Dimension | Ask of each vendor |
|---|---|
| Covered worker nodes and hardware | Which nodes, which hardware, which are billable |
| Platform components supported | Exactly which components are inside the support boundary |
| Support hours and response targets | Hours, timezone, severities, targets (and whether they are commitments) |
| Installation and upgrade responsibilities | Who installs, who upgrades, who validates |
| Storage, networking, backup and DR | What is covered versus separately scoped |
| Migration services | What is included, what is a separate project |
| Third-party subscriptions | What must be bought elsewhere (OS, storage, hypervisor tooling) |
| Total first-year and renewal cost | Both years, with taxes stated separately |

Zorvia's answers come from [SUPPORT_SCOPE.md](SUPPORT_SCOPE.md) and the order form. Note that Zorvia runs on KubeVirt infrastructure you operate; Kubernetes and KubeVirt operational ownership is priced separately unless a Managed contract covers it, so compare like with like.

## Sources and assumptions

| Claim | Source | Status |
|---|---|---|
| OpenShift Virtualization Engine is licensed per bare-metal node; no list price is published | [Red Hat product page](https://www.redhat.com/en/technologies/cloud-computing/openshift/virtualization-engine), as cited in [leave-openshift.md](leave-openshift.md) | **Access date must be recorded, and the page re-read, each time a comparison is issued** |
| Zorvia feature maturity | [FEATURE_MATURITY.md](FEATURE_MATURITY.md) at the release being quoted | Current release |

Any competitive claim added to this page must carry a source and the date it was checked. Claims without a dated source do not belong here.

## What this page does not say

- It does not state that Zorvia costs less. That depends on your quote.
- It does not claim Red Hat certification, operator-catalogue parity or telco-grade networking. Those are called out as gaps in [leave-openshift.md](leave-openshift.md).
- It does not sell Experimental features (for example plan-only migration or multi-cluster inventory) as validated services.
