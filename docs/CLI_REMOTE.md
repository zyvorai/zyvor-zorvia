# CLI for API-backed experimental features

Status: **Experimental**. Thin clients over the REST APIs of [DR pairs](DR_REPLICATION.md),
[volume encryption](VOLUME_ENCRYPTION.md), [certificates](CERTIFICATE_LIFECYCLE.md) and [rescue operations](RESCUE.md). The server enforces RBAC
(`cluster.admin`), validation, audit and the `ZORVIA_EXPERIMENTAL=1` gate; the CLI only builds the request.
Tested against a mock HTTP server, never against a live cluster.

The resource-pool commands (zorvia inventory pool) use the same connection options; see [RESOURCE_POOLS.md](RESOURCE_POOLS.md).

## Connection

| Option | Env | Meaning |
|---|---|---|
| `--url` | `ZORVIA_URL` | API base URL, e.g. `https://zorvia.example.com:30152` |
| `--token` | `ZORVIA_TOKEN` | Bearer token (prefer the env var) |
| `--insecure`, `--ca-cert FILE` | | TLS for lab or private CAs |
| `-o table\|yaml\|json` | | Output format (default `table`) |

Redirects are not followed. If the server answers `403 EXPERIMENTAL_DISABLED` the CLI stops with a message
that an operator must set `ZORVIA_EXPERIMENTAL=1` on the API server.

## Commands

```bash
export ZORVIA_URL=https://zorvia.example.com:30152 ZORVIA_TOKEN=...

zorvia dr pair list
zorvia dr pair create p1 --vm web --rpo-seconds 900 [--dr-namespace zorvia-dr] [--rto-seconds N]
zorvia dr pair status p1
zorvia dr pair failover p1                                        # dry run: plan only
zorvia dr pair failover p1 --execute --confirm "failover p1"      # real; [--no-start]
zorvia dr pair failback-plan p1
zorvia dr pair resume p1 --confirm "resume p1"
zorvia dr pair delete p1 --yes                                    # forget the pair; VMs and backups untouched

zorvia volume encryption status
zorvia --namespace prod volume encryption plan web data --size 20Gi --storage-class ceph-enc
zorvia --namespace prod volume encryption apply web data --size 20Gi --storage-class ceph-enc --yes
zorvia --namespace prod volume encryption rotate web data --yes

zorvia certs list [--namespace NS] [--expiring-within-days 30]
zorvia certs status
zorvia certs alerts
zorvia certs cert-manager list
zorvia certs cert-manager renew web site --confirm "renew web/site"   # marks the Certificate Issuing, like cmctl renew

zorvia rescue reset-password web --user root     # password: no-echo prompt, or one stdin line
zorvia rescue install-packages web curl git
zorvia rescue inspect web
zorvia rescue inspect web --wait --timeout 300   # poll the job until it finishes (default 600 s)
zorvia rescue status web <job-name>
```

`pair delete` needs `--yes` and the server refuses it (409 `PAIR_BUSY`) while a failover or failback is active. `--wait` (rescue `reset-password`, `install-packages`, `inspect`) queues the job, then polls `rescue status` every 2 s until it is `succeeded` or `failed` (exit status non-zero on `failed`) or `--timeout` seconds pass; on timeout the job keeps running and the message names the `rescue status` command to check later.

Failover is a dry run unless `--execute` is given with the exact confirmation phrase; a wrong phrase builds
no request. `apply` and `rotate` need `--yes`; `certs cert-manager renew` needs the exact `--confirm "renew <namespace>/<name>"` phrase (a wrong phrase builds no request). The namespace for volume encryption is the global `--namespace` (or ZORVIA_NAMESPACE).

## Autoscaler loop (read-only)

```
zorvia autoscaler status
zorvia autoscaler decisions [--target web] [--limit 50]
```

`GET /api/v1/autoscaler/status|decisions` (cluster.admin, Experimental). When the loop is not running the
answer says `enabled: false` with the reason. See AUTOSCALER_LOOP.md for what a decision record contains and
its limits (in memory only, per process).

## Password handling

`rescue reset-password` has no password option and reads no environment variable. On a terminal it prompts
twice without echo; when stdin is a pipe it reads one line (`printf %s "$PW" | zorvia rescue reset-password ...`;
note that `echo`/`printf` of a literal in your shell history is yours to manage). The value is sent only in the
request body, is held in a type with a redacted `Debug`, and is scrubbed from any error text.

## Not covered

No `golden gates status` and no autoscaler loop status command: the server has no API for either
(scan/sign gates and the loop are configured by environment variables and visible only in server logs and the
golden-pipeline `plan` response), so nothing was added rather than inventing one. Shell completions are
generated from the command tree and pick up the new commands automatically; a unit test asserts the bash, zsh and fish scripts contain `dr pair`, `volume encryption` and `rescue` and their subcommands.

No `golden gates status` command: the server has no API for it (scan/sign gates are configured by
environment variables and visible only in server logs and the golden-pipeline `plan` response), so nothing was
added rather than inventing one. Shell completions are
generated from the command tree and pick up the new commands automatically.
