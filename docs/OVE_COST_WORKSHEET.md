# OpenShift Virtualization cost worksheet

Red Hat does **not** publish a list price for OpenShift Virtualization Engine
(OVE is licensed per bare-metal node). This worksheet never invents an OVE
price. You paste **your** quote; Zorvia’s side uses published launch prices from
[PRICING.md](PRICING.md).

See also [OPENSHIFT_COMPARISON.md](OPENSHIFT_COMPARISON.md) (how to compare) and
[leave-openshift.md](leave-openshift.md).

## Inputs (fill in)

| Input | Your value |
|-------|------------|
| Access date of Red Hat product page (re-check each quote) | |
| Billable bare-metal / worker nodes (same set for both sides) | |
| **Your** OVE / OpenShift Virtualization quote (currency, year 1) | |
| Your OVE support / subscription renewals (year 2+) | |
| Extra Red Hat / partner line items in scope (storage, ACS, …) | |
| Zorvia region for estimate | India (INR) / International (USD) |
| Zorvia support tier | Supported / Supported Plus / Managed |
| One-time Deployment / Migration / Training needed? | |

## Zorvia estimate (from PRICING.md)

Annual support is the **greater of** (nodes × per-node) and the annual minimum.

| Tier | Per worker node / year | Annual minimum |
|------|------------------------:|---------------:|
| Supported (INR) | ₹30,000 | ₹1.5 lakh |
| Supported Plus (INR) | ₹60,000 | ₹4 lakh |
| Supported (USD) | $600 | $3,000 |
| Supported Plus (USD) | $1,200 | $8,000 |

Optional one-time (not support): Deployment ₹1–3 lakh / from $3,000;
Migration from ₹2 lakh / $5,000. Managed is quoted separately.

### Quick compute

```bash
# Usage: ./scripts/ove-cost-worksheet.sh <nodes> <inr|usd> <supported|supported-plus> [ove_year1_amount]
./scripts/ove-cost-worksheet.sh 8 usd supported 48000
```

The script prints the Zorvia annual support estimate and, if you pass an OVE
year-1 amount, a side-by-side **difference** (your quote minus Zorvia). It does
not claim savings percentages or that Zorvia is cheaper.

## Assumptions to keep aligned

- Same node set and hardware class on both sides.
- Zorvia runs on **KubeVirt you operate**; Kubernetes / KubeVirt / storage /
  networking ownership is outside the support line unless Managed covers it
  ([SUPPORT_SCOPE.md](SUPPORT_SCOPE.md)).
- Do not sell Experimental features as validated services.
- Taxes, travel, and third-party subscriptions are additional on both sides.
