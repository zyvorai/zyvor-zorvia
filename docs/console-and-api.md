# Web console and API

Route map, WebSocket endpoints and the Fabric HTTP API. The full route map lives in [WEB_CONSOLE.md](WEB_CONSOLE.md).

[Back to the README](../README.md) · [Docs index](README.md)

Same backend, same auth, as the CLI — the console is a signed-in SPA served
from the same NodePort as the API. This is the short version; the full route
map lives in [docs/WEB_CONSOLE.md](WEB_CONSOLE.md).

| Route | Purpose |
|-------|---------|
| `/app` | Dashboard |
| `/app/create` | Linux (cloud-init) or Windows (Kryton) create |
| `/app/vms/:name` | Power, expose, cloud-init, snapshots, hotplug, resize, Rescue mode (admin) — [docs/RESCUE.md](RESCUE.md) |
| `/app/vms/:name/console` | Serial · VNC · SSH |
| `/app/snapshots` · `/app/migrations` | Snapshots · live migration |
| `/app/storage` · `/app/volumes` | Rook-Ceph · fleet PVCs · optional Atlas backend/RBD/object-store/DR/governance |
| `/app/databridge` | Atlas DataBridge: cloud-to-edge DB migration (optional) — [docs/ATLAS_INTEGRATION.md](ATLAS_INTEGRATION.md) |
| `/app/access-control` | Users & roles (admin) |
| `/app/pods` | Every pod: Terminal.app-style live logs, shell, events, YAML, restart/delete (admin) — [docs/PODS.md](PODS.md) |
| `/app/disk-images` | Image catalog as a black Terminal.app listing (`zorvia template images` on the CLI) |
| `/app/quotas` · `/app/network-policies` | Real `ResourceQuota` / `NetworkPolicy` |
| `/app/templates` · `/app/windows` | OS catalog · Kryton inventory |
| `/app/placement` · `/app/capacity` · `/app/optimizer` | Placement · headroom · right-sizing |
| `/app/backups` · `/app/schedules` · `/app/alerts` | Backup · power cron · alerts |

```text
wss://<HOST>:30152/ws/console/<vm>?token=<jwt>
wss://<HOST>:30152/ws/vnc/<vm>?token=<jwt>
wss://<HOST>:30152/ws/ssh/<vm>?token=<jwt>&user=ubuntu
wss://<HOST>:30152/ws/pods/<ns>/<pod>/logs?token=<jwt>     # cluster.admin
wss://<HOST>:30152/ws/pods/<ns>/<pod>/exec?token=<jwt>     # cluster.admin
```

Set `ZORVIA_EXPOSE_HOST` for correct NodePort SSH/VNC/RDP hostnames in the UI.

**Fabric API (same auth as the console):**

| Method | Path | Notes |
|--------|------|-------|
| POST | `/api/v1/auth/login` | JWT |
| GET | `/api/v1/health` · `/api/v1/features` | Liveness · maturity registry |
| GET | `/api/v1/adopt/report` | Read-only adoption report (`?all_namespaces=true`); same as `zorvia adopt` |
| GET/POST | `/api/vms` | List / create |
| POST | `/api/vms/:name/start\|stop\|restart` | Power |
| GET/POST | `/api/vms/:name/snapshots` | Snapshots |
| GET | `/api/audit/export` | JSONL audit trail (`?format=jsonl`) |

Password from `zorvia-auth` — not committed defaults. Lab smoke: [docs/LAB.md](LAB.md).
