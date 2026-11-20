# Change Planner

`zorvia change plan` turns Drift Guard findings into a conservative operational execution plan: what can change online, what needs a restart, what requires recreate, and what needs manual review.

It does not apply changes. Use it for change windows, CI gates, and operator review before you mutate the cluster.

## Why plan after drift

| Tool | Job |
|------|-----|
| `zorvia change drift` | Semantic desired-vs-live (or file-vs-file) diff with severity |
| `zorvia change plan` | Turn those findings into downtime-aware steps |

The planner assumes CPU/memory and network changes need a controlled restart unless you have confirmed hotplug support. Storage-source, firmware, machine-type, and TPM changes require recreation or explicit review.

## Examples

```bash
# Offline desired-vs-actual planning
zorvia change plan desired.yaml --actual live-capture.yaml

# Plan against the live cluster
zorvia --namespace production plan desired.yaml --vm payments-01

# CI gates
zorvia change plan desired.yaml --actual current.yaml --fail-on-downtime
zorvia change plan desired.yaml --actual current.yaml --fail-on-recreate

# Machine-readable output
zorvia change plan desired.yaml --actual current.yaml -o json
zorvia change plan desired.yaml --actual current.yaml -o yaml
```

## Typical plan shape

Plans group steps by risk class:

1. **Online** — metadata / labels that can apply without stopping the VMI
2. **Restart** — capacity or network topology that needs a controlled stop/start
3. **Recreate** — disks, volumes, firmware, machine type, TPM
4. **Manual review** — ambiguous or high-blast-radius findings

Use `--fail-on-downtime` or `--fail-on-recreate` in CI when a PR must not introduce restart/recreate work without review.

## Workflow

```bash
# 1. Capture drift
zorvia change drift desired.yaml --vm payments-01 -o json > drift.json

# 2. Build an execution plan
zorvia change plan desired.yaml --vm payments-01 -o json > plan.json

# 3. Apply only after review (CLI create/start/stop, web console, or GitOps)
```

## Related

- [DRIFT_GUARD.md](DRIFT_GUARD.md) — severity model, ignores, CI `--fail-on`
- [SNAPSHOTS.md](SNAPSHOTS.md) — snapshot before recreate windows
- [WEB_CONSOLE.md](WEB_CONSOLE.md) — day-2 power / clone / console
