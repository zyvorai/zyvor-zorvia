# Control-plane availability

By default Zorvia's state (users, audit trail, durable operations, schedules) is SQLite and JSON
files under `/data`, on one PersistentVolumeClaim, which decides the topology: one replica with fast
failover. With PostgreSQL configured it can run two active replicas (below).

## Supported: one replica, fast failover (active/standby)

`replicaCount: 1` with a `ReadWriteOnce` volume (the default; also `values-production.yaml`).
If the node dies, Kubernetes reschedules the pod and the volume re-attaches elsewhere.
Expect an outage of roughly the time the CSI driver takes to detach/attach plus pod start
(minutes, not seconds, and longer if the dead node must first be fenced). While the API is
down, running VMs are unaffected.

What survives a restart or failover: sessions (token versions live in the DB), the audit
trail, durable operations (the reconciler resumes them; restarts do not consume retry
attempts), schedules and backup history.

The chart uses `strategy: Recreate` on RWO so the new pod is not blocked by a
Multi-Attach error. Upgrades therefore have a short API outage too.

## Beta: two replicas on PostgreSQL (active/active API)

With `database.existingSecret` set ([POSTGRES.md](POSTGRES.md)) users, tokens, revocations, the
operations queue, the audit trail and the JSON documents (backup and power schedules, warm pools,
alerts, webhooks, migration history) live in PostgreSQL, so any replica can serve any request and a
replica loss costs only the in-flight requests, not a volume re-attach.

```bash
helm upgrade --install zorvia charts/zorvia -f charts/zorvia/values-production.yaml -f charts/zorvia/values-ha.yaml
```

The chart refuses `replicaCount > 1` unless a database is configured, `api.leaderElection` is on, and
`ha.acceptLocalState: true` confirms the two things that stay per replica: commercial-offerings records
(`ZORVIA_COMMERCIAL_DB`, an Experimental single-instance feature) and the optional JSONL audit sidecar.
With several replicas `/data` is an `emptyDir`, rolling updates replace `Recreate`, and a
PodDisruptionBudget plus preferred anti-affinity spread the pods.

Only the lease holder runs the schedulers and the operations reconciler; the others serve the API.
Two replicas saving the *same* JSON document in the same instant are last-writer-wins. The failed-login lockout is shared through the database.

PostgreSQL becomes the single point of failure, so run it with its own HA (CloudNativePG, Patroni, a
managed service). Measured on the lab (2026-10-02, single node, two replicas):

- **Leader killed with an off-cluster backup running** (`kubectl delete pod --force` on the lease holder): the operation stalled
  for about 120 s (heartbeat staleness plus lease takeover), was re-queued with its attempt refunded (`interruptions: 1`), re-run by the new
  leader and succeeded, encrypted, 188 s after the kill. The API itself kept answering on the other replica.
- **PostgreSQL pod deleted under load**: 503 while it was down, automatic reconnection, no restart ([POSTGRES.md](POSTGRES.md)).

Not measured: failover of a replicated PostgreSQL, node loss on a multi-node cluster, load.

## Not supported: two replicas on SQLite

Two writers on one SQLite file are unsafe, and an RWO volume cannot be mounted from two nodes anyway.
The chart fails to render `replicaCount > 1` without a database.

Measured so far: an API pod restart (`kubectl rollout restart`) takes about 40 to 50 s to a healthy API on the reference lab, with sessions and VMs unaffected ([SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)). That is a planned restart, not node loss; no node-loss time has been recorded.

## Node-loss test (lab)

Cordon-less failure test: power off or `kubectl drain --force --delete-emptydir-data` the
node running `zorvia-api`, time until `GET /api/v1/health` answers again, then confirm
sessions still work and a running operation (`GET /api/operations`) resumes. Record the
measured time against your storage class; it is the number to quote as your RTO.
