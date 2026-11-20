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
| Supported (INR, incl. 18% GST) | ₹59,999 | ₹299,995 (5 nodes) |
| Supported Plus (INR, incl. 18% GST) | ₹119,999 | ₹599,995 (5 nodes) |
| Supported (USD) | $600 | $3,000 |
| Supported Plus (USD) | $1,200 | $6,000 |

Optional one-time (not support): Deployment is a custom quote; Migration (Transiva)
is $75 / $60 / $50 per VM (1–99 / 100–499 / 500+ VMs), minimum $2,500 (₹7,434 / ₹5,947 /
₹4,956 per VM, minimum ₹247,800, in India). Managed is from $1,500 per cluster per month
(₹148,680 in India). Prices: INR incl. 18% GST in India; US dollars excl. tax elsewhere (VAT, GST or sales tax added where applicable). Pay online by UPI, card or netbanking where a buy page exists; international cards work (charged in rupees); or request a US-dollar invoice by bank transfer. Custom pricing: contact sales@zyvor.dev.
Buy / get a quote: <https://zyvor.dev/buy>.

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
