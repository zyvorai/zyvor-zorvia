# Zorvia + Kryton integration

Zorvia becomes the operator-facing control surface while Kryton remains the Windows virtualization control plane.

## Architecture

```text
Browser
  │  Zorvia JWT / API key
  ▼
Zorvia web + API
  │  server-side Kryton bearer token
  │  /api/v1/kryton/*
  ▼
Kryton REST API
  ├─ demo provider
  ├─ dockur/windows provider
  └─ KubeVirt provider
```

The browser never receives `KRYTON_TOKEN`. This is deliberate: Zorvia owns user authentication and Kryton credentials remain a server secret.

## Create VM wizard integration

Windows isn't a separate page you have to know to look for — it's the "Windows" toggle in the main Create VM wizard (`/app/create`). Picking it swaps the Linux cloud-init/disk-image steps for a live Kryton golden-image catalog (`GET /api/v1/kryton/images`), and submitting calls `POST /api/v1/kryton/machines` directly instead of the KubeVirt-backed `/api/vms`. None of the KubeVirt-specific advanced options (firmware, CPU model, TPM, HyperV, …) apply to this path — Kryton owns the VM's actual virtualization layer (KubeVirt, dockur, or its own KubeVirt provider, depending on how it's configured). A successful create navigates to `/app/windows`, where the machine shows up in the same inventory used for lifecycle/snapshot management.

If `KRYTON_URL` is unset, the Windows toggle still renders but shows a "Kryton not reachable" state instead of the catalog — the wizard doesn't hard-fail, it just can't create anything until Kryton is configured.

## Configuration

Set these on the Zorvia API process:

| Variable | Required | Meaning |
|---|---:|---|
| `KRYTON_URL` | yes to enable | Kryton base URL, e.g. `http://kryton.kryton.svc.cluster.local:8080` |
| `KRYTON_TOKEN` | for apikey mode | Kryton bearer token |
| `KRYTON_PROJECT` | yes for Windows inventory/actions | Default Kryton project/tenant used by the Windows page |
| `KRYTON_TIMEOUT_SECS` | no | Upstream HTTP timeout, default 30 |
| `KRYTON_TLS_INSECURE` | no | `true` only for explicitly trusted labs using self-signed TLS |

If `KRYTON_URL` is absent, Zorvia starts normally and the integration reports `enabled=false`. If Kryton is reachable but `KRYTON_PROJECT` is absent, the UI can still read the image catalog but blocks project-scoped inventory and write actions until the project is configured.

## Zorvia endpoints

- `GET /api/v1/kryton/status`
- `GET /api/v1/kryton/capabilities`
- `GET /api/v1/kryton/doctor`
- `GET /api/v1/kryton/images`
- `GET /api/v1/kryton/summary?project=...`
- `GET|POST /api/v1/kryton/machines`
- `GET|DELETE /api/v1/kryton/machines/:id`
- `POST /api/v1/kryton/machines/:id/start`
- `POST /api/v1/kryton/machines/:id/stop`
- `POST /api/v1/kryton/machines/:id/snapshot`
- `GET /api/v1/kryton/machines/:id/snapshots`
- `POST /api/v1/kryton/machines/:id/snapshots/:sid/restore`
- `DELETE /api/v1/kryton/machines/:id/snapshots/:sid`
- `GET|POST /api/v1/kryton/golden` (list or start a Windows golden build)
- `GET /api/v1/kryton/golden/:id`
- `GET /api/v1/kryton/golden/:id/passport` (guestkit Cutover Passport)
- `POST /api/v1/kryton/golden/:id/bootstrap` (import into a CDI DataSource)

The golden endpoints back the console's **Golden Images** page; see
[GOLDEN_IMAGES.md](GOLDEN_IMAGES.md#windows-golden-images-via-kryton).

All `/api/v1/kryton/*` routes are protected by Zorvia's existing auth middleware.

## Deliberately not proxied yet

Kryton's HTML console/VNC stream is not proxied yet, and neither is the dockur viewer URL (`consoleUrl`) on a golden build. A correct implementation needs websocket-aware reverse proxying and explicit origin/auth handling. The Windows page exposes machine lifecycle, images, RDP coordinates, diagnostics and snapshots without leaking the Kryton token.

## Production recommendations

1. Run Kryton behind a ClusterIP service reachable only from Zorvia and trusted operators.
2. Keep `KRYTON_TOKEN` in a Kubernetes Secret.
3. Use Kryton project scoping instead of exposing provider namespaces to the UI.
4. Keep `KRYTON_TLS_INSECURE=false` outside isolated labs.
5. Rotate the Kryton API key independently of Zorvia user credentials.
