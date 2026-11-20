# Support scope

Prices are in [PRICING.md](PRICING.md). Support is a paid service contract around the source-available software. It is not a software licence, and nothing in the product enforces it.

The tier structure (business-hours and 24×7 tiers, severity-based response targets, named contacts, a published lifecycle) follows the familiar enterprise-support model. It is Zyvor's own policy. It does not claim the same terms, certifications or coverage as any other vendor's subscription.

> **Product status:** contract terms (tier, coverage hours, timezone, holidays, per-severity response targets, authorized contacts, covered clusters) are recorded on the contract. The support case portal that measures them is being built ([COMMERCIAL_OFFERINGS.md](COMMERCIAL_OFFERINGS.md)).

Which Kubernetes, KubeVirt, CDI and storage combinations have actually been run is recorded, with dates and measured results, in [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md). A combination not listed there has not been validated.

## Tiers

| | Supported | Supported Plus | Managed |
|---|---|---|---|
| Channel | Case portal, email | Case portal, email, phone for Sev 1 | As Supported Plus, plus the operations channel in the contract |
| Hours | Business hours | 24×7 for Sev 1 and 2; business hours for Sev 3 and 4 | 24×7 for Sev 1 and 2 by default; per contract |
| Response targets | Yes | Yes, faster | Per contract (defaults match Supported Plus) |
| Zorvia troubleshooting and upgrade guidance | Yes | Yes, priority handling | Yes |
| Planned Zorvia upgrade assistance | Guidance | Assistance | Per contract |
| Quarterly health review | No | Yes | Yes |
| Named support contacts | Yes | Yes | Yes |
| Escalation to engineering lead | Sev 1 and 2 | Sev 1 and 2, with manager notification | Per contract |
| Monitoring, maintenance, operational tasks | No | No | Per contract |

**Business hours** are Monday to Friday, 09:00 to 18:00 in the timezone named in the contract, excluding the holidays listed in the contract. **24×7** applies only to the severities marked above, and only for covered clusters.

## Severity definitions

| Severity | Meaning |
|---|---|
| 1 – Critical | Production Zorvia function unavailable or unusable, no workaround |
| 2 – High | Major Zorvia function badly degraded, or Sev 1 with a workaround |
| 3 – Medium | Partial loss of a non-critical function, with a workaround |
| 4 – Low | Question, documentation issue or feature request |

Severity is about impact on Zorvia functionality. Problems in Kubernetes, storage, hardware or guests that Zorvia merely reveals are handled as diagnostic guidance unless separately contracted.

## Response targets

Targets are for **first response by an engineer**, counted in the coverage hours that apply to that severity. They are **not resolution deadlines**.

| Severity | Supported | Supported Plus |
|---|---|---|
| 1 – Critical | 1 business hour | 1 hour, 24×7 |
| 2 – High | 4 business hours | 2 hours, 24×7 |
| 3 – Medium | 1 business day | 4 business hours |
| 4 – Low | 2 business days | 1 business day |

Managed response targets are written into the contract. Resolution times are estimates, never commitments. The timer pauses while a case is waiting on the customer.

## What is covered

| Component | Supported / Supported Plus | Managed |
|---|---|---|
| Zorvia API, CLI, TUI and web console | Covered | Covered |
| Zorvia configuration and documented integrations | Covered within the compatibility matrix below | Covered |
| Kubernetes and KubeVirt operations | Diagnostic guidance; operational ownership priced separately | Covered only when explicitly contracted |
| Storage and networking | Zorvia integration troubleshooting; backend support priced separately | Defined per contract |
| Guest operating systems and applications | Separately scoped | Separately scoped |
| Backups and recovery drills | Separately scoped | Included only where contracted and validated |
| Hardware and cloud infrastructure | Customer / provider responsibility | Responsibility specified in the contract |

Experimental features ([FEATURE_MATURITY.md](FEATURE_MATURITY.md)) are outside support unless the contract says otherwise in writing.

## Supported releases and compatibility

- Fixes and security updates are provided for the **latest Zorvia minor release and the one before it**. Older releases get upgrade guidance only.
- Support applies to Zorvia running against a **supported Kubernetes and KubeVirt version pair** listed in the release notes for that Zorvia version. Unlisted combinations receive best-effort guidance.
- Security advisories are published through [GitHub Security Advisories](https://github.com/zyvorai/zorvia/security/advisories) ([SECURITY.md](../SECURITY.md)).

## Customer responsibilities

Keep named contacts current, provide a diagnostic bundle when asked ([DIAGNOSTIC_PRIVACY.md](DIAGNOSTIC_PRIVACY.md)), run a supported release, and give access only through the scoped, revocable mechanisms in [MANAGED_OPERATIONS.md](MANAGED_OPERATIONS.md). A contract grants no cluster access.

## Contact channels

Case portal (once available) and email. Until a dedicated support address is published, use sales@zyvor.dev. Phone for Sev 1 is provided to Supported Plus and Managed customers with the onboarding pack.

## Support case portal

Case lifecycle: `open → triaged → investigating → waiting_on_customer → resolved → closed`, with reopening, escalation history, a snapshot of the contract coverage at creation, and organization-isolated access. Timers run in the contract's coverage hours and timezone, skip contract holidays, and pause while a case waits on the customer.
