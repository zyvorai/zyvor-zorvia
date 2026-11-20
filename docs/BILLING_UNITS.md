# Billing units

> **Status: Beta, records only.** Capacity observations, reconciliation and invoice *records* exist. **No payment provider is integrated, nothing is charged, and no card data is handled.** Tax and legal invoice issuance stay with your billing system; a Zorvia invoice is a record that can carry that system's number.

## Unit

One **registered Kubernetes worker node** within an agreed covered cluster. The contract's `node_treatment` states how control-plane nodes, infrastructure-only nodes, temporary nodes and disconnected clusters are treated. That statement is copied onto every reconciliation so the person reading it applies the same rule.

## Capacity observations

`POST /api/v1/billing/capacity/observe` (`cluster.admin`) reads node **labels** from the local cluster and records: a stable cluster identity, total / control-plane / worker counts, timestamp, source and, on failure, the error. Nothing else is read: no workload names, no guest contents, no node names.

- **Cluster identity** is `ZORVIA_CLUSTER_ID` if set, otherwise the UID of the `kube-system` namespace. The contract's covered-cluster identifier must equal it.
- A node with a control-plane label counts as control plane even if it also runs workloads; the contract's `node_treatment` decides how such mixed nodes are billed.
- Disconnected clusters use `POST /api/v1/billing/admin/capacity/observations` (`source: offline_import` or `manual`).

## Unknown is never zero

`GET /api/v1/billing/capacity/reconcile?contract_id=…` classifies each covered cluster:

| State | Meaning | Usable for billing |
|---|---|---|
| `ok` | Latest observation succeeded and is newer than `ZORVIA_CAPACITY_STALE_DAYS` (default 7) | Yes |
| `stale` | Latest successful observation is too old (last value shown for information) | **No** |
| `error` | Latest observation failed (discovery error) | **No** |
| `missing` | No observation at all | **No** |

`billable_worker_nodes` is present **only** when every covered cluster is `ok`; otherwise it is `null` and `unknown_clusters` lists what is missing. It is never a partial sum, and a failed or empty discovery is stored as an error, not as a zero-node observation. A successful count of zero total nodes is rejected for the same reason. The result also reports `over_allowance` against the contract's node allowance when it can be determined.

## Invoice records

Line items are integer minor units (`quantity × unit_price_minor`, overflow-checked); the currency is taken from the contract's quote.

- `draft → issued → paid`, or `void` from draft/issued. A paid invoice cannot be voided. Drafts are invisible to customers.
- Marking an invoice paid takes a `payment_reference`. It is **idempotent**: repeating the call with the same reference changes nothing and records no second event. A different reference on a paid invoice, or reusing a reference on another invoice, is a conflict. `external_reference` (your billing system's invoice number) is unique per organization.
- The contract's payment status follows its invoices (`invoiced` while any is issued and unpaid, `paid` once settled), without overriding `overdue` or `waived` set by an administrator.
- `GET /api/v1/billing/invoices/{id}/document` downloads a plain-text rendering.

Annual pricing (the greater of nodes × unit price and the annual minimum) is applied by whoever drafts the invoice; see [PRICING.md](PRICING.md). Reconciliation supplies the node count, not the price.

## Not built

Payment provider adapters, webhook signature verification and replay rejection, and recurring charging (which requires an accepted agreement and payment authorization). These wait for a provider and commercial terms.
