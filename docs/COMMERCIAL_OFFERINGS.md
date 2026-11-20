# Commercial offerings

Zorvia is source-available under the Zyvor Production License v1.0 and every existing capability works without a subscription. Contracts describe **purchased services and support coverage**. They never gate VM features.

> **Expiry safety rule.** An expired, cancelled or missing contract changes commercial service eligibility only. It never stops VMs, disables VM APIs or blocks access to your data.

## Offerings

| Offering | Included scope | Billing model |
|---|---|---|
| Community | Software, documentation, community issues | Free |
| Supported | Technical assistance, upgrade guidance, agreed response times | Annual contract |
| Supported Plus | Supported plus 24x7 response for Severity 1 and 2, priority handling, quarterly health reviews, planned upgrade assistance | Annual contract |
| Managed | Supported plus agreed monitoring, maintenance, upgrades, recovery testing | Monthly or annual contract |
| Deployment | Installation, integrations, configuration, acceptance testing | One-time project |
| Migration | Assessment, execution, guest validation, handover | Scoped project |
| Training | Administrator onboarding, operational workshops | Workshop fee |

The full per-offering detail (prerequisites, responsibilities, exclusions, required integrations) is served, without prices, by the public `GET /api/v1/commercial/catalog`. Prices are set per quote by authorized commercial administrators. Nothing is priced by default and nothing is charged automatically.

## Flow

1. **Quote request.** A signed-in user submits company, contact, cluster count, worker-node count, workload size, region, desired coverage and requirements. This creates a record. No notification is sent unless a notification integration is configured (none exists yet).
2. **Reviewable quote.** An administrator turns the request into a *draft contract*. The quote lists services and infrastructure covered, included capacity, pricing unit, currency, duration, response targets, support hours, both parties' responsibilities, exclusions and required integrations.
3. **Entitlement.** While the contract is a draft the administrator sets covered clusters, tier, coverage hours and timezone, authorized contacts, effective and expiration dates, node allowance and managed-operation permissions.
4. **Acceptance.** `draft → awaiting_acceptance`; a member of the customer organization accepts, giving `active`. Accepted terms cannot be edited; to revise, return the contract to `draft`.
5. **End of life.** `active → expired` (by date) or `cancelled`. Both are terminal; renewal is a new contract. Payment status (`not_invoiced`, `invoiced`, `paid`, `overdue`, `waived`) is tracked separately from contract status.

Activation requires effective and expiry dates, plus for Supported/Managed at least one covered cluster, a timezone and coverage hours.

## Organizations and isolation

Customers are organizations with named members (usernames). Non-administrators see only quote requests they filed or that belong to their organizations, and only their organizations' contracts, history and coverage. Anything else answers `404`, indistinguishable from a missing record. Administrators (`users.admin`) see everything.

## API

| Endpoint | Access |
|---|---|
| `GET /api/v1/commercial/catalog` | Public |
| `GET/POST /api/v1/commercial/quote-requests` | Signed in |
| `GET /api/v1/commercial/contracts`, `/{id}`, `/{id}/history` | Signed in, own organizations |
| `POST /api/v1/commercial/contracts/{id}/accept` | Member of the contract's organization |
| `GET /api/v1/commercial/coverage[?cluster=]` | Signed in, own organizations |
| `GET/POST /api/v1/support/cases`, `GET /api/v1/support/cases/{id}` | Signed in, own organizations |
| `POST /api/v1/support/cases/{id}/messages` (text plus base64 attachments) | Signed in, own organizations; `internal` notes support desk only |
| `POST /api/v1/support/cases/{id}/status`, `/escalate` | Signed in, own organizations (customers may only close or reopen a resolved case) |
| `GET /api/v1/support/cases/{id}/attachments/{aid}` | Signed in, own organizations; always an opaque download |
| `POST /api/v1/support/diagnostics` | `cluster.admin`; preview by default, `{"preview": false}` downloads. Never uploads |
| `POST /api/v1/support/cases/{id}/diagnostics/upload` | `cluster.admin` and case access; needs `{"confirm": true}` |
| `POST /api/v1/support/admin/cases/{id}/assign` | `users.admin` (support desk) |
| `GET /api/v1/services/engagements`, `…/{id}`, `POST …/{id}/milestones/{key}/evidence`, `POST …/{id}/accept` | Signed in, own organizations |
| `/api/v1/services/admin/*` | `users.admin` (service desk): create engagements, update milestones, request acceptance, cancel |
| `/api/v1/managed/enrollments/*` | Signed in, own organizations; see [MANAGED_OPERATIONS.md](MANAGED_OPERATIONS.md) |
| `/api/v1/managed/admin/*` | `users.admin` (service desk) |
| `GET /api/v1/billing/invoices`, `…/{id}`, `…/{id}/document`; `GET /api/v1/billing/capacity/reconcile` | Signed in, own organizations (drafts hidden) |
| `POST /api/v1/billing/capacity/observe` | `cluster.admin` and `users.admin`: records the local cluster's node counts |
| `/api/v1/billing/admin/*` | `users.admin` (service desk): manual observations, invoice create/issue/mark-paid/void |
| `/api/v1/commercial/admin/*` | `users.admin` only: organizations and members, quote generation, entitlement edits, transitions, payment status, offline import |

An unconfigured integration shows as unavailable in the catalog response (`integrations.*.available = false`).

### Offline import

`POST /api/v1/commercial/admin/contracts/import` loads a contract bundle for disconnected deployments. It is administrator-only, validated like any contract, recorded in history with `source = offline_import`. **Bundle authenticity is not cryptographically verified in this phase.**

## Storage, backup and rollback

- Commercial data lives in its own SQLite file: `ZORVIA_COMMERCIAL_DB` (default `<data dir>/zorvia/commercial.db`), separate from VM state and the auth DB.
- **Single instance only.** SQLite in one file is safe for exactly one commercial API replica. A shared transactional store is required before running several replicas; that store is not implemented yet.
- **Migrations** are versioned and append-only (`schema_migrations`), applied automatically at first use, each in a transaction.
- **Backup:** before upgrading, stop the server (or use `sqlite3 commercial.db ".backup 'commercial.pre-upgrade.db'"`) and keep the copy.
- **Rollback:** there are no down-migrations. Stop the server, restore the pre-upgrade copy, start the older version. A build refuses to open a database whose schema is *newer* than it understands rather than run against it.
- Every contract change (quote, entitlement edits with before/after, transitions, acceptance, payment status, import, expiry) is appended to the contract's history with actor and time.

## Feature maturity

| Area | Status |
|---|---|
| Catalog, quote requests, contracts, coverage, offline import | **Beta** (records only) |
| Support cases, timers, diagnostic bundles | **Beta** (single-instance; see below) |
| Service engagements, managed-operation records | **Beta** (records only; no cluster access) |
| Capacity reconciliation, invoice records | **Beta** (records only; see [BILLING_UNITS.md](BILLING_UNITS.md)) |
| Payment provider integration | Not implemented; waits on provider and terms |
| Notifications | Not implemented; requests are recorded only |

Existing experimental migration APIs remain experimental (see [FEATURE_MATURITY.md](FEATURE_MATURITY.md)).

## Support cases and timers

Entitlements carry the machine-readable terms cases are measured against: `response_target_minutes` and `resolution_estimate_minutes` (keyed `sev1`..`sev4`, in *coverage minutes*), `always_on_severities` (24x7 severities such as `sev1`, `sev2` for Supported Plus), `holidays` (`YYYY-MM-DD`), `coverage_hours` and an IANA `timezone` (validated against the bundled tz database).

- A case snapshots the contract's coverage when it is created. Later contract edits do not move an open case's targets. A cluster with no active coverage still gets a case, recorded without response commitments.
- The **first-response clock** counts only covered time, skips holidays, is DST-correct, and stops at the first customer-visible reply from the support desk. Internal notes and customer-initiated uploads do not stop it.
- The **resolution estimate** is never a commitment. It pauses while a case is `waiting_on_customer` and resumes from the covered time that remained. A case cannot enter `waiting_on_customer` before the desk has responded.
- States: `open → triaged → investigating ⇄ waiting_on_customer → resolved → closed`. Customers can close a resolved case or reopen it; the desk can reopen a closed one. Owner assignment and every escalation are recorded in the case timeline.
- Attachments: base64 in the message, at most 5 per message and `ZORVIA_SUPPORT_ATTACHMENT_MAX_BYTES` each (default 5 MiB, capped at 6 MiB). Filenames are sanitized, downloads are always `application/octet-stream` with `nosniff`. Bytes older than `ZORVIA_SUPPORT_ATTACHMENT_RETENTION_DAYS` (default 90) are purged; their metadata stays.

**Who is "support"?** A caller with `users.admin` acts as the support desk on this instance. The case portal lives in your own Zorvia instance's database, so a remote vendor support team needs a `users.admin` account on it, or the customer relays. Central multi-tenant case handling is not built.

Migration `v2` adds the support tables; rollback is the same restore-from-backup procedure as above.

## Service engagements

Deployment (assessment, install/integration, functional and recovery testing, handover, acceptance), migration (inventory/assessment, network and storage mapping, pilot, approved waves, guest and application validation, handover) and training engagements are tracked as milestones with an owner, due date and **evidence**. A milestone cannot be marked done without evidence. The desk requests acceptance once every other milestone is done; only a member of the customer's organization can accept, which completes the acceptance milestone. This tracks purchased delivery only; it runs nothing, and experimental migration APIs stay experimental.

Migration `v3` adds these tables; rollback is the same restore-from-backup procedure as above.

Migration `v4` adds capacity and invoice tables; rollback is the same restore-from-backup procedure as above.
