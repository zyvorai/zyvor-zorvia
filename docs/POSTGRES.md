# PostgreSQL backend

Status: **Beta (auth store, operations queue, audit trail, JSON documents); two active replicas supported with the caveats below.**

## What moves

| Store | Backend with `ZORVIA_DATABASE_URL` set |
|---|---|
| Users, API tokens, session revocations, TOTP state | **PostgreSQL** |
| Failed-login lockout counters | **PostgreSQL** (`login_failures`), in addition to the per-replica in-memory count |
| Durable operations (`ops.db`): queue, state, progress, results | **PostgreSQL** |
| Audit trail (`audit.db`) | **PostgreSQL** (every replica writes to it; reads refresh at most once a second) |
| JSON documents: backup and power schedules, warm pools, alerts, webhooks, migration history | **PostgreSQL** (`state_docs` table, one row per document) |
| Commercial-offerings records (`commercial.db`), JSONL audit sidecar | Per replica, on local disk (not for multi-replica) |

Two active replicas are supported ([HA.md](HA.md), `values-ha.yaml`): any replica serves any request, token revocation and TOTP
replay protection hold across replicas, an idempotency key creates one operation even when replicas race on it, only one replica
claims an operation, and only the lease holder runs schedulers. What stays per replica is listed above; the chart makes you
acknowledge it (`ha.acceptLocalState`).

JSON documents are loaded on every request and saved whole, so replicas see each other's changes immediately; two replicas saving
the same document in the same instant are last-writer-wins. An installation switching to PostgreSQL keeps its files as a
fallback: a document is read from its file until the first save writes it to the database.

### Operations on a shared store

Only the lease leader runs operations. At start-up the leader re-queues work left `running` by a dead process, but on PostgreSQL it
leaves alone any operation whose heartbeat is newer than 90 seconds, so a replica that starts while another is healthy does not
steal its work; a genuinely orphaned operation is picked up by the normal 120 s orphan check.

## Configure

```yaml
database:
  existingSecret: zorvia-db   # Secret with key "url": postgres://user:pass@host:5432/zorvia
  import: true                # first start only
```

or `ZORVIA_DATABASE_URL=postgres://...` (`sslmode=require` enables TLS). The schema is created on start, guarded by a
Postgres advisory lock so replicas starting together do not race.

## Migrating from SQLite

Start once with `ZORVIA_DATABASE_IMPORT=1`: users, tokens and revocations are copied from `auth.db`, and the audit history from `audit.db`, into **empty** tables.
It is idempotent and does nothing when the database already has users. Remove the flag afterwards. The SQLite file is left
untouched, so rollback is unsetting the URL.

## Tests

`cargo test` runs the same conformance and race suite on SQLite always, and on Postgres when `ZORVIA_TEST_POSTGRES_URL` is set
(CI-less today: run it against a throwaway container). Live-verified on the lab cluster (single node, Postgres 17 in-cluster): import of the existing admin, sign-in, restart persistence, and two replicas where a logout on one replica made the token fail on the other. Not verified: Postgres failover, TLS-enabled connections to a managed service.

## When the database is down

Measured on the lab by deleting the PostgreSQL pod under load (single node, the pod took about 40 s to come back):

- Requests that need the user store answer **503 `AUTH_STORE_UNAVAILABLE`** with `Retry-After: 5` (never 401, so clients do not sign the
  user out); about 22 seconds of 503 in that run, zero 401. Nothing is authenticated while the store cannot be read.
- Every database request is bounded: `ZORVIA_DATABASE_TIMEOUT_SECS` (2 to 25, default 8), 5 s to connect, TCP keepalives after 15 s.
  A request already waiting when the database vanished can take up to that long; one such slow request was seen.
- The connection re-establishes by itself once the database is back; no restart is needed. Audit entries written while it is down are
  logged as warnings and lost (the in-memory view still shows them until the next refresh); the operations reconciler pauses and resumes.

## Limits

- The failed-login lockout (8 failures, 15 minutes) is counted in `login_failures`, so attempts spread over replicas add up to one lockout. If the database cannot be read the shared check is skipped (logged) and only the per-replica in-memory count applies; sign-in itself needs the same database anyway.
- No automatic failover of PostgreSQL itself; use your operator (CloudNativePG, Patroni, managed service).
- One statement is retried once on a dropped connection; there is no pool, each replica holds one connection.

## Audit trail notes

The API keeps the newest 10 000 entries in memory for filtering and refreshes them from PostgreSQL when the audit pages or the security
dashboard are read. The optional JSONL sidecar (`ZORVIA_AUDIT_JSONL`) is still written per replica to its own volume. A failed audit
write is logged as a warning and does not fail the request, as with SQLite.
