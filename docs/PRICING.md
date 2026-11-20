# Pricing

> **Note:** Zyvor is moving to an enterprise subscription model for new quotes: an annual subscription that includes updates and the stated support level, with open-source and non-production terms unchanged. See `docs/SUBSCRIPTION-MODEL.md` in this repository. Rates below apply to existing agreements; contact [sales@zyvor.dev](mailto:sales@zyvor.dev) for current rates.


> **Approved launch prices.** The India and International tables below are the approved launch prices for Zorvia services. Final terms are in your signed order form.
>
> **Draft, not approved:** the section "Relationship to Zyvor Platform editions (proposal v1.2)" is a proposal pending commercial approval. It does not change the approved prices above it. No billing, payment collection or entitlement enforcement exists in the product yet.

## The software stays free

Zorvia is Apache-2.0. You may use, modify and run it in production, commercially, at no charge. Paying customers pay for **services around the software**: production support, deployment, migration, training and managed operations. Support is a paid service contract, not a software licence, and nothing in the product checks for one.

An expired or missing contract never stops VMs or disables existing community functionality ([COMMERCIAL_OFFERINGS.md](COMMERCIAL_OFFERINGS.md)).

## India (INR; Supported and Supported Plus include 18% GST, other rows exclude taxes)

| Offering | Price | Billing period | Included services |
|---|---:|---|---|
| Community | Free | n/a | Software, documentation, community support |
| Supported | ₹59,999 per worker node including GST; minimum 5 nodes (₹299,995) | Per year | Business-hours Zorvia troubleshooting and upgrade guidance |
| Supported Plus | ₹119,999 per worker node including GST; minimum 5 nodes (₹599,995) | Per year | Supported, plus 24×7 response for Severity 1 and 2, priority handling, quarterly health reviews, planned Zorvia upgrade assistance |
| Managed | From ₹50,000 per cluster | Per month | Support, monitoring, maintenance and operational tasks defined in the contract |
| Deployment | ₹1–3 lakh per project | One-time | Installation, agreed integrations, validation, handover |
| Migration | From ₹2 lakh per project | Per project | Assessment, pilot, migration waves, validation |
| Training | ₹40,000 per workshop | Per workshop | One remote administrator workshop and operational runbooks |

## International (USD, excluding taxes)

Supported and Supported Plus are the GST-free equivalent of the India price (INR / 1.18 / 84, rounded). The other rows are independently set regional prices, **not currency conversions** of the India table.

| Offering | Starting price | Billing period |
|---|---:|---|
| Supported | $600 per worker node; $3,000 annual minimum | Per year |
| Supported Plus | $1,200 per worker node; $6,000 annual minimum | Per year |
| Managed | $1,500 per cluster | Per month |
| Deployment | $3,000 per project | One-time |
| Migration | $5,000 per project | Per project |
| Training | Custom quote | n/a |

## Relationship to Zyvor Platform editions (proposal v1.2)

Zorvia's per-node support prices above are the **open-source support meter** in the Zyvor pricing proposal v1.2 and apply to Zorvia alone.

- **Zyvor Platform Enterprise and above** ($8,500 per node per year; founding price $7,350 in year 1 for the first 25 customers) include Zorvia **Supported Plus** on the covered hosts. Do not add a second Zorvia subscription for those nodes.
- **Zyvor Platform Professional** ($4,500 per node per year) does not include Zorvia support; Zorvia **Supported** ($600 / worker node) can be added.
- One support subscription per covered node covers all Apache-2.0 Zyvor infrastructure products on that node, not one subscription per product.
- A node is one physical server with up to 2 CPU sockets and 64 cores. The minimum for every platform edition is one node. The USD Supported Plus minimum is $6,000 (it was $8,000).
- Volume bands (4–7 nodes −10%, 8–15 −15%, 16+ −20%) and term discounts (2-year −5%, 3-year −10%, combined cap −25%) need commercial approval; annual minimums apply after discounts.
- VM migration is priced per VM on one ladder: $75 (1–99 VMs), $60 (100–499), $50 (500+), project minimum $2,500. This replaces the $5,000 per-project starting price only where a per-VM quote is used.
- **Getting started:** the software is free (Community: up to 4 nodes with 64 cores in total). A free two-week migration assessment, a 30-day proof of concept (extendable by agreement), a capped 90-day migration promotion and a startup program are available by agreement. Program discounts never stack and total at most 50%.
- The founding price cannot be combined with the proof-of-concept or any program discount; volume and term bands are capped at 25% in total, measured against list price.
- Renewal increases are capped at 7%. Counts use a fixed measurement date; true-ups are prospective only, with a 90-day grace period.

Draft status: the v1.2 platform and bundle rates are proposals pending approval and are not binding until a written quotation or order form. Contact [sales@zyvor.dev](mailto:sales@zyvor.dev).

## What is not included

Hardware, infrastructure hosting, third-party subscriptions, travel and applicable taxes are additional. So is any service listed as "separately scoped" in [SUPPORT_SCOPE.md](SUPPORT_SCOPE.md).

## Billing rules

- **Billable node:** a Kubernetes worker node covered by the contract.
- **Order form:** lists the covered clusters and nodes.
- **Node treatment:** the order form states explicitly how mixed-role nodes, temporary nodes and capacity changes are treated. Nothing is assumed.
- **Annual support price:** the **greater of** (worker nodes × per-node price) and the annual minimum.
- **Mid-term additions:** follow the proration policy written in the order form. No default policy is assumed on this page.
- **Managed:** quoted by node count, VM count, supported components and operational workload. Managed pricing **includes** the contracted support for the same coverage, so do not add a second Supported subscription for it.
- **Activation:** a paid service is activated only after explicit customer acceptance.
- **Expiry:** ends the service; never stops VMs.

### Minimum-contract check

| Worker nodes | Supported (India) | Supported Plus (India) | Supported (USD) | Supported Plus (USD) |
|---:|---:|---:|---:|---:|
| 3 | ₹299,995 (minimum) | ₹599,995 (minimum) | $3,000 (minimum) | $6,000 (minimum) |
| 5 | ₹299,995 (= 5 × ₹59,999) | ₹599,995 (= 5 × ₹119,999) | $3,000 (= 5 × $600) | $6,000 (= 5 × $1,200) |
| 8 | ₹479,992 | ₹959,992 | $4,800 | $9,600 |
| 12 | ₹719,988 | ₹1,439,988 | $7,200 | $14,400 |

The minimum is 5 nodes, so the node-based price exceeds the minimum above 5 nodes for both tiers.

## Example: three-node deployment (India)

| Item | Amount |
|---|---:|
| Deployment | ₹1 lakh |
| Supported annual subscription (3 × ₹59,999 = ₹179,997, below the minimum) | ₹299,995 |
| **First-year total** | **₹399,995** |
| Annual support renewal | ₹299,995 |

Applicable taxes and separately scoped services are additional. Renewal assumes unchanged scope and pricing.

## Maturity

Only **GA** and documented **Beta** features are sold as production services. Deployment and migration engagements do not commit to Experimental capabilities (for example the plan-only Transiva hook). See [FEATURE_MATURITY.md](FEATURE_MATURITY.md).
