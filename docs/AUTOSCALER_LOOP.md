# Autoscaler reconcile loop (Experimental)

A leader-gated loop that reads load, decides with hysteresis/cooldown/bounds, and either
recommends or performs a scale action. **Off by default.**

## Enabling

| Variable | Meaning |
|----------|---------|
| `ZORVIA_EXPERIMENTAL=1` | Required. |
| `ZORVIA_AUTOSCALER_LOOP=1` | Opt in to the loop. |
| `ZORVIA_AUTOSCALER_TARGETS` | Path to a JSON array of targets (required; invalid file means the loop does not start). |
| `ZORVIA_AUTOSCALER_APPLY=1` | Actually scale. Without it the loop is recommendation-only (default). |
| `ZORVIA_AUTOSCALER_INTERVAL_SECS` | Tick interval, default 60, clamped 10..3600. |
| `ZORVIA_AUTOSCALER_PROMETHEUS_URL` | Optional Prometheus fallback load source (see Load sources). |
| `ZORVIA_AUTOSCALER_PROMETHEUS_TOKEN_FILE` | Optional path to a file holding a bearer token for Prometheus. Read on every request (rotation needs no restart), sent only over https (a non-https URL with this set disables the Prometheus source), never logged; there is deliberately no plain env var for the token. Max 8 KiB, one token, no whitespace. |
| `ZORVIA_AUTOSCALER_METRICS_MAX_AGE_SECS` | Samples older than this are stale and ignored. Default 120, clamped 30..900. |

## Targets

```json
[{"id":"web","kind":"vm_pool","namespace":"prod","name":"web-pool",
  "metric_vms":["web-pool-0","web-pool-1"],"load_metric":"cpu","min":2,"max":10,"step":1,
  "scale_up_threshold":80,"scale_down_threshold":30,
  "required_samples":3,"cooldown_secs":300}]
```

- `vm_pool`: patches `spec.replicas` of a KubeVirt `VirtualMachinePool`.
- `vm_cpu`: total vCPUs of one VM through CPU hotplug (needs `cpu.maxSockets`).
- `load_metric`: `cpu` (default) or `memory`; the average over `metric_vms` (default `[name]` for `vm_cpu`).
- Validated: DNS-1123 names, `min <= max <= 1000`, `scale_up - scale_down >= 5` (dead band),
  `cooldown_secs >= 30`, at most 100 targets.

## Decision rules

- Load at or above the up threshold (or at or below the down threshold) for `required_samples`
  consecutive ticks triggers a step of `step`, clamped to `min`/`max`.
- A sample inside the dead band resets both streaks, so flapping load never acts.
- After any recommendation or attempt (including failures) the target is on cooldown.
- Only the lease leader acts (`ZORVIA_LEADER_ELECTION`, see SCHEDULER_LEADERSHIP.md); a
  follower does not even read metrics.
- Each recommendation or action writes an audit entry (`ScaleUp`/`ScaleDown`, user
  `system:autoscaler`); applied/failed actions also create an `autoscale` operation record.

## Load sources

Samples are marked real only when they come from one of these (`src/autoscaler/telemetry.rs`):

1. **Kubernetes metrics API** (first choice). Lists `metrics.k8s.io/v1beta1` `PodMetrics` in the
   target namespace with selector `kubevirt.io=virt-launcher,vm.kubevirt.io/name=<vm>` and reads
   the `compute` container. CPU percent is usage divided by the VM's vCPUs (sockets x cores x
   threads, else the CPU request); memory percent is usage divided by guest memory (or the
   memory request). Needs metrics-server (or equivalent) and RBAC to list `pods.metrics.k8s.io`.
2. **Prometheus** (optional, only when the metrics API gave no usable answer). Instant queries
   on `/api/v1/query` using KubeVirt metrics: CPU is
   `100 * sum(rate(kubevirt_vmi_cpu_usage_seconds_total{namespace,name}[2m])) / vcpus`; memory is
   `100 * (1 - usable_bytes / available_bytes)` (needs guest memory stats). The URL must be
   `https`, except `http` to a loopback host; no credentials, query or fragment in the URL;
   redirects are not followed; 5s connect / 10s total timeout; response capped at 1 MiB.
   No authentication header is sent, so the endpoint must be reachable without one.

**No decision on bad data.** Missing metrics, a stale timestamp (older than the max age, or more
than 60s in the future), several virt-launcher pods for one VM (e.g. mid-migration), a missing
`compute` container, an unparsable or out-of-range value, an HTTP error, an oversized or non-JSON
response: the source returns an error, the reconciler logs it and skips that target for the tick.
It does not touch the hysteresis streaks and never scales. For a pool, every member VM must have
fresh data or the whole target is skipped.

## Seeing decisions (read-only API, CLI, web)

Today's decisions used to appear only in logs and the audit trail. The loop now also keeps a
bounded in-memory record of each tick's outcome per target: timestamp, target, metric
(`cpu`/`memory`), load percent, load source (`metrics_api`, `prometheus`, `simulated`),
decision (`scale_up`, `scale_down`, `hold`, `skipped`), outcome (`applied`, `recommended`,
`failed`, `held`, `skipped`), from/to sizes and a short reason (`cooldown`, `within dead band`,
`at bound`, `load unavailable: ...`, ...).

- Up to 100 records per target (oldest dropped), at most 100 targets. Consecutive identical
  hold/skip ticks are folded into one record with a `count`, so steady-state ticks cannot push
  real scale decisions out. Reasons are one line, 200 characters, with URLs replaced by `<url>`.
- API (cluster.admin, `ZORVIA_EXPERIMENTAL=1`; otherwise 403 `EXPERIMENTAL_DISABLED`):
  `GET /api/v1/autoscaler/status` (loop mode, leader, last tick, targets with policy and
  hysteresis streaks) and `GET /api/v1/autoscaler/decisions?limit=50&target=web` (newest first,
  `limit` 1..=500, `target` a DNS-1123 label of a configured target, else 404). When the loop is
  not running, both answer 200 with `enabled: false` and a `disabled_reason`.
- CLI: `zorvia autoscaler status` and `zorvia autoscaler decisions [--target ID] [--limit N]`
  (see CLI_REMOTE.md). Web: `/app/autoscaler` (admin), with the same experimental, disabled
  state as the other experimental pages.
- Nothing secret is stored or returned: target ids/namespaces/names, numbers and the reasons
  above only. `metric_vms` and executor error detail beyond the first line are not exposed.
- **Limits.** Memory only: lost on restart and not persisted (no `state_docs` persistence).
  Per process: only the leader ticks, so a follower's log is empty and the view shows the
  replica that answered. Mock/unit tested only; not exercised against a live cluster.

## Limits (read before relying on it)

- **Real telemetry is new and not verified on a live cluster.** The metrics API and
  Prometheus sources are tested against a local mock HTTP server with fixture JSON only. The
  virt-launcher pod label, the KubeVirt metric names and the PromQL are from documentation, not
  from a run against KubeVirt.
- **What is measured.** The metrics API reports the virt-launcher `compute` container (the host
  side of the guest: QEMU plus overhead), so CPU tracks guest CPU use reasonably but memory is
  the pod working set against guest memory, which overstates idle guests with a warm page
  cache. Prefer the Prometheus memory query (guest-reported) when memory drives a target.
- The old `MetricsCollector` source (`MonitoringLoadSource`, random usage) is no longer used by
  the loop; samples flagged `simulated` are still never applied if some source produces them.
  Applying remains opt-in (`ZORVIA_AUTOSCALER_APPLY=1`), and every other gate is unchanged.
- The decision logic, cooldown, bounds, hysteresis, dry-run and leader gating are unit
  tested with fake metrics/executor. The Kubernetes executor (pool patch, CPU hotplug) and
  the audit/operations sink are **not verified on a live cluster**.
- Not fenced across leadership loss mid-action; cooldown state is in memory and resets on
  restart. The existing `AutoScaler` policy engine (`evaluate`) is not used by the loop.
