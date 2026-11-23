# VM manifest lint

`zorvia change lint` checks a KubeVirt `VirtualMachine` (or a Zorvia `VMConfig`) for day-2 pitfalls **before**
it is applied. Drift Guard compares two states and the maintenance plan reasons about a running node; the
linter answers "will this definition bite me later?" from the manifest alone. It is offline and read-only:
no cluster access, nothing is changed. See [RESEARCH-VM-PLATFORM-2026.md](RESEARCH-VM-PLATFORM-2026.md) for why
these rules were chosen.

```bash
zorvia change lint vm.yaml                       # table, exit 1 on a high finding
zorvia change lint a.yaml b.yaml --fail-on medium
zorvia change lint vm.yaml -o json               # [{file, report}]
zorvia change lint vm.yaml --fail-on none        # report only
```

API (any caller allowed to create VMs; pure computation on the body, no audit entry, nothing stored):

```bash
curl -sk -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  --data @vm.json "https://<host>:30152/api/v1/vm-lint" | jq
curl -sk -H "Authorization: Bearer $TOKEN" "https://<host>:30152/api/v1/vm-lint/rules"
```

The report lists findings (`rule`, `severity`, RFC 6901 `pointer`, `message`, `remedy`), the highest
severity, and `live_migration_possible` plus `live_migration_blockers`: the rule IDs that stop the VM from
live-migrating, i.e. why a node drain would stop it instead of moving it.

## Rules

| ID | Severity | Blocks live migration | What it flags |
|----|----------|-----------------------|---------------|
| ZL001 | high | yes | `bridge` binding on the pod network |
| ZL002 | high | yes | masquerade interface listing ports 49152/49153 (used by the migration proxy) |
| ZL003 | medium | no | `emptyDisk`, or a `containerDisk` with no persistent volume: data lost on restart |
| ZL004 | medium | yes | `hostDevices`, `gpus`, SR-IOV interface or `hostDisk`: pinned to a node |
| ZL005 | medium | no | no memory declared |
| ZL006 | low | no | no `evictionStrategy` |
| ZL007 | medium | yes | CPU model `host-passthrough` |
| ZL008 | high | no | password, token or private key in inline cloud-init (`userData`/`networkData`) |
| ZL009 | low | no | deprecated `spec.running` without `runStrategy` |
| ZL010 | low | no | `ide`/`sata` disk bus or emulated NIC model (`e1000`, `rtl8139`, ...) |
| ZL011 | high | no | disk without volume, volume without disk, interface without network |
| ZL012 | medium | no | base64 cloud-init (cannot be reviewed or scanned) |
| ZL013 | medium | yes | `dedicatedCpuPlacement` |

## Safety properties

- A finding for ZL008 locates the secret (`.../cloudInitNoCloud/userData`) and never repeats it; a test
  asserts the secret text is absent from the serialized report.
- Input is untrusted: any JSON value is accepted without panicking, cloud-init text is scanned only up to
  256 KiB, a report holds at most 500 findings (`truncated: true` when cut), CLI files over 2 MiB are refused,
  and the API body is bounded by the server's normal request limit.
- `ZL008` is heuristic (key names such as `password:`/`token:` with a non-boolean value, `chpasswd` lists, PEM
  private key headers). It will miss credentials in other shapes and flags templated values only if they
  are not `{{...}}` or `${...}`.

## Limits

The rules are static: they look at the manifest, not the cluster. A clean report does not promise a
live migration (storage access modes, node capacity and the destination are checked by
[the maintenance plan](MAINTENANCE.md) and by KubeVirt itself), and ZL003 cannot know whether a
containerDisk-only VM is intentionally stateless. Verified by unit tests only; not run against a live KubeVirt.
