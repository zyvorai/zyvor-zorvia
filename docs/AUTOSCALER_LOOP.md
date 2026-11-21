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

## Targets

```json
[{"id":"web","kind":"vm_pool","namespace":"prod","name":"web-pool",
  "metric_vms":["web-pool-0","web-pool-1"],"min":2,"max":10,"step":1,
  "scale_up_threshold":80,"scale_down_threshold":30,
  "required_samples":3,"cooldown_secs":300}]
```

- `vm_pool`: patches `spec.replicas` of a KubeVirt `VirtualMachinePool`.
- `vm_cpu`: total vCPUs of one VM through CPU hotplug (needs `cpu.maxSockets`).
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

## Limits (read before relying on it)

- **The existing monitoring source is simulated.** `MetricsCollector` returns real
  allocations but random usage. Samples from it are flagged simulated and the loop
  never applies them, even with `ZORVIA_AUTOSCALER_APPLY=1`: it only recommends. Applying
  needs a real telemetry source behind the `LoadSource` trait.
- The decision logic, cooldown, bounds, hysteresis, dry-run and leader gating are unit
  tested with fake metrics/executor. The Kubernetes executor (pool patch, CPU hotplug) and
  the audit/operations sink are **not verified on a live cluster**.
- Not fenced across leadership loss mid-action; cooldown state is in memory and resets on
  restart. The existing `AutoScaler` policy engine (`evaluate`) is not used by the loop.
