# Guest Insight

`zorvia guest guest-insight` summarizes KubeVirt VMI guest integration from status alone. It reports phase, node, QEMU Guest Agent detection, guest OS/kernel metadata, interfaces, a readiness score (0–100), and recommendations — without logging into the VM.

## CLI

```bash
zorvia --namespace production guest guest-insight payments-01
zorvia --namespace production guest guest-insight payments-01 -o json
zorvia --namespace production guest guest-insight payments-01 -o yaml
zorvia --namespace production guest guest-insight payments-01 --strict
```

| Flag | Meaning |
|------|---------|
| `-o table\|json\|yaml` | Output format (default table) |
| `--strict` | Non-zero exit when the QEMU Guest Agent is not `connected` |

## Example table output

```text
Zorvia Guest Insight

  VM:          production/payments-01
  Phase:       Running
  Node:        worker-3
  Guest Agent: connected
  Readiness:   90/100
  Guest OS:    Ubuntu 22.04.4 LTS
  Kernel:      5.15.0-105-generic

Interfaces:
  - eth0           52:54:00:…        10.244.1.42
```

## HTTP API

```bash
curl -sk -H "Authorization: Bearer $TOKEN" \
  https://HOST:30152/api/vms/payments-01/guest-insight
```

Returns the same report shape used by the CLI (`agent_state`, `readiness_score`, `os_*`, `interfaces`, `recommendations`).

## How it works

KubeVirt surfaces guest-agent data on the VMI `status` when the agent is connected. Zorvia maps that into `GuestInsightReport` (`src/guest_insight`). No guest credentials are required.

## Related

- Wait for Ready + IP: `zorvia guest wait-ready <vm>`
- Web metrics: `GET /api/vms/:name/metrics` (guest OS / filesystem when the agent is up)
- [WEB_CONSOLE.md](WEB_CONSOLE.md) · [DRIFT_GUARD.md](DRIFT_GUARD.md)
