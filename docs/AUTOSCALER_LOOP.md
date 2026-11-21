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
