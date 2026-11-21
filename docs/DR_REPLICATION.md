# DR replication pairs (Experimental)

A replication pair keeps a verified off-cluster backup of one VM no older than its
RPO, so the VM can be restored as `{vm}-dr`. It builds on the off-cluster S3
backup and restore code ([OFFCLUSTER_BACKUP.md](OFFCLUSTER_BACKUP.md)).

Enable with `ZORVIA_EXPERIMENTAL=1` and `ZORVIA_FEATURE_DR_REPLICATION=1`; an
off-cluster target must be configured. All routes need `cluster.admin`.

## What it does

- **Pairs persist** as one JSON document through the same state store as warm
  pools and schedules: a file under the data directory, or PostgreSQL when
  `ZORVIA_DATABASE_URL` is set (shared by replicas; concurrent edits of the
  document by two replicas are last-writer-wins, see [POSTGRES.md](POSTGRES.md)).
- **Scheduled replication.** Every 60 s the loop (leader-gated like the other
  schedulers, see [SCHEDULER_LEADERSHIP.md](SCHEDULER_LEADERSHIP.md)) polls each
  pair's in-flight backup and, when `rpo_seconds` has passed since the last
  attempt, queues a durable off-cluster backup (snapshot, upload, read-back
  verify). A backup counts as success only if it is restorable. A failed
  attempt is retried one interval later, not in a tight loop.
- **Lag** is the time since the last verified backup (or since the pair was
  created until the first one). RPO is met while lag <= `rpo_seconds`.
- **Alerts** go through the notifications module (and the log): one alert when
  the RPO is first breached, one when it recovers, one per failed backup.
  `GET /api/v1/enterprise/dr/alerts`. The notification list is in memory, per replica.

## API (`/api/v1/enterprise/dr/`)

| Route | Purpose |
|---|---|
| `GET/POST pairs` | list status / create `{name, vm, namespace?, dr_namespace?, rpo_seconds (60..604800), rto_seconds?}` |
| `GET/DELETE pairs/{name}` | detail / remove |
| `GET pairs/{name}/status` | RPO/RTO: health, lag, last success, next due, measured RTO |
| `GET pairs/{name}/failover/plan` | steps, warnings, blockers, confirm phrase |
| `POST pairs/{name}/failover` | `{dry_run (default true), confirm, start (default true)}` |
| `GET pairs/{name}/failback/plan` | manual failback steps and blockers |
| `POST pairs/{name}/resume` | `{confirm: "resume <name>"}` after failback |
| `GET alerts` | recent DR alerts |

## Failover safety rules

Failover restores the last verified backup as `{vm}-dr` in `dr_namespace`. It is
refused (409) when: the source VM is running or its state cannot be read
(anything but an explicit Stopped counts as running); a VM already exists at the
DR target (nothing is ever overwritten); there is no verified backup; or the
pair already failed over. `dry_run` defaults to true, and a real run also needs
`confirm` equal to `failover <pair name>`. A stale backup (RPO breached) is a
warning with the potential data loss shown, not a block. Measured RTO is the
time from the failover request to restore completion.

## Limits

- Same-cluster object-store replication, not site failover; nothing repoints
  traffic or DNS. A pair does not protect against losing the cluster that runs Zorvia.
- Failback is a plan plus a confirmed `resume`; the data return is manual.
- No incremental replication: each backup is a full one.
- Unit-tested with fake backup/restore backends; not exercised against a live
  cluster or PostgreSQL in this change.

## Web console

`/app/dr-pairs` (admin) lists pairs with health, lag against RPO and measured against target RTO, creates and removes pairs, and runs failover: **Dry run** always, **Fail over now** only when the plan is allowed and the exact confirm phrase is typed. When the API answers 403 `feature_disabled` the page shows an experimental, disabled notice. Failback plan and resume are not in the UI yet.
