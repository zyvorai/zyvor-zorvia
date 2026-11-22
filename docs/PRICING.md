# Pricing

> **Note:** Zyvor is moving to an enterprise subscription model for new quotes: an annual subscription that includes updates and the stated support level. See `docs/SUBSCRIPTION-MODEL.md` in this repository. Rates below apply to existing agreements; contact [sales@zyvor.dev](mailto:sales@zyvor.dev) for current rates.
>
> **Prices: INR incl. 18% GST in India; US dollars excl. tax elsewhere (VAT, GST or sales tax added where applicable). Pay online by UPI, card or netbanking where a buy page exists; international cards work (charged in rupees); or request a US-dollar invoice by bank transfer. Custom pricing: contact [sales@zyvor.dev](mailto:sales@zyvor.dev).** Buy / get a quote: <https://zyvor.dev/buy>.


> **Approved prices.** The India and International tables below are the approved prices for Zorvia support, and the "Relationship to Zyvor Platform editions" section carries the approved Zyvor price book (2026-11-20). Final terms are in your signed order form. No billing, payment collection or entitlement enforcement exists in the product itself.

## What you pay for

Zorvia is source-available under the Zyvor Production Licence v2.0. Production Use requires a commercial license (sales@zyvor.dev). Customers pay for the commercial license and **services around the software**: production support, deployment, migration, training and managed operations. Support is a paid service contract, not a software licence, and nothing in the product checks for one.

An expired or missing contract never stops VMs or disables existing functionality ([COMMERCIAL_OFFERINGS.md](COMMERCIAL_OFFERINGS.md)).

## India (INR, including 18% GST)

| Offering | Price | Billing period | Included services |
|---|---:|---|---|
| Supported | ₹59,999 per worker node including GST; minimum 5 nodes (₹299,995) | Per year | Business-hours Zorvia troubleshooting and upgrade guidance |
| Supported Plus | ₹119,999 per worker node including GST; minimum 5 nodes (₹599,995) | Per year | Supported, plus 24×7 response for Severity 1 and 2, priority handling, quarterly health reviews, planned Zorvia upgrade assistance |
| Managed | From ₹148,680 per cluster (USD 1,500 x 99.12) | Per month | Support, monitoring, maintenance and operational tasks defined in the contract |
| Deployment | Custom quote: [sales@zyvor.dev](mailto:sales@zyvor.dev) | One-time | Installation, agreed integrations, validation, handover |
| Migration | Per VM, one ladder: ₹7,434 (1–99 VMs), ₹5,947 (100–499), ₹4,956 (500+); project minimum ₹247,800 | One-time | Assessment, pilot, migration waves, validation |
| Training | Custom quote: [sales@zyvor.dev](mailto:sales@zyvor.dev) | Per workshop | One remote administrator workshop and operational runbooks |

## International (USD, excluding taxes)

Supported and Supported Plus are the GST-free equivalent of the India price (INR / 1.18 / 84, rounded). Managed and Migration are USD-native; their India figures above are USD x 99.12 including 18% GST. Pay online or by US-dollar invoice (bank transfer); VAT, GST or sales tax is added where applicable.

| Offering | Starting price | Billing period |
|---|---:|---|
| Supported | $600 per worker node; $3,000 annual minimum | Per year |
| Supported Plus | $1,200 per worker node; $6,000 annual minimum | Per year |
| Managed | From $1,500 per cluster | Per month |
| Deployment | Custom quote | One-time |
| Migration | $75 / $60 / $50 per VM (1–99 / 100–499 / 500+ VMs); $2,500 project minimum | One-time |
| Training | Custom quote | n/a |

## Relationship to Zyvor Platform editions

Zorvia's per-node support prices above are the **Zyvor Support meter** in the Zyvor price book and apply to every Zyvor infrastructure software product on the node.

- **Zyvor Platform Enterprise and above** ($8,500 per node per year, about ₹842,520 including GST in India; founding price $7,350 / ₹728,532 in year 1 for the first 25 customers) include Zorvia **Supported Plus** on the covered hosts. Do not add a second Zorvia subscription for those nodes.
- **Zyvor Platform Professional** ($4,500 per node per year, about ₹446,040 including GST in India) does not include Zorvia support; Zorvia **Supported** (₹59,999 / $600 per worker node) can be added.
- One support subscription per covered node covers all Zyvor infrastructure software on that node, not one subscription per product.
- A node is one physical server with up to 2 CPU sockets and 64 cores. The minimum for every platform edition is one node. The Supported Plus minimum is $6,000 (₹599,995 in India).
- Volume bands (256–499 nodes −10%, 500–999 −15%, 1,000+ −20%) and term discounts (2-year −5%, 3-year −10%, combined cap −25%) apply to per-node list prices; annual minimums apply after discounts.
- VM migration (Transiva) is priced per VM on one ladder: $75 / ₹7,434 (1–99 VMs), $60 / ₹5,947 (100–499), $50 / ₹4,956 (500+), project minimum $2,500 / ₹247,800 (India figures are USD x 99.12 including 18% GST). The h2kvm engine is included at no extra charge in a Transiva project.
- **Getting started:** a commercial license is required to run Zorvia in production. A free two-week migration assessment, a 30-day proof of concept and 30-day trial key for production licences (extendable to 60 days by agreement), a capped 90-day migration promotion and a startup program are available by agreement. Edu, nonprofit and design-partner discounts are a percentage, never stack and total at most 50%.
- The founding price cannot be combined with the proof-of-concept or any program discount; volume and term bands are capped at 25% in total, measured against list price.
- Renewal increases are capped at 7%. Counts use a fixed measurement date; true-ups are prospective only, with a 90-day grace period.

Rates are binding only through a written quotation or order form. Buy / get a quote: <https://zyvor.dev/buy>. Custom pricing: [sales@zyvor.dev](mailto:sales@zyvor.dev).

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

## Example: three-node cluster (India)

| Item | Amount |
|---|---:|
| Supported annual subscription (3 × ₹59,999 = ₹179,997, below the minimum) | ₹299,995 |
| **First-year total** | **₹299,995** |
| Annual support renewal | ₹299,995 |

Deployment and other separately scoped services are quoted on top (sales@zyvor.dev). Renewal assumes unchanged scope and pricing.

## Maturity

Only **GA** and documented **Beta** features are sold as production services. Deployment and migration engagements do not commit to Experimental capabilities (for example the plan-only Transiva hook). See [FEATURE_MATURITY.md](FEATURE_MATURITY.md).
