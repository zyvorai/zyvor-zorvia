# Disk image upload (CDI upload proxy)

Status: **Experimental** (`image-upload`). Needs `ZORVIA_EXPERIMENTAL=1`. Verified only against a local mock of the upload proxy in unit tests; it has not been run against a live CDI.

Zorvia can take a local or streamed disk image, validate its format, create a CDI `DataVolume` with an `upload` source, and push the bytes through the CDI upload proxy. It does not shell out to `qemu-img`: CDI's upload server converts qcow2, vmdk, vhdx, vhd and vdi to raw itself. Raw and ISO images are stored as they are.

## What it does

1. Reads the first ~32 KiB and detects the format from magic numbers (qcow2, vmdk, vhdx, vhd, vdi, iso). Raw has no signature, so it is accepted only when declared `raw`. A declared format that disagrees with detection is rejected. This is the only buffering; the rest is streamed.
2. Creates `DataVolume` `NAME` (`source.upload`, `storage.bind.immediate.requested`) and waits up to 120 s for `UploadReady`.
3. Creates an `UploadTokenRequest` for the PVC and streams the body to `POST {proxy}/v1beta1/upload` with the token as a bearer credential.
4. While streaming it counts bytes against a hard cap (default 20 GiB, `ZORVIA_IMAGE_UPLOAD_MAX_BYTES`) and hashes with SHA-256. If the cap is exceeded or an expected `sha256` does not match, the request body is aborted with an error (never ended cleanly), so CDI does not finalise it, and the DataVolume is deleted.
5. Any failure after the DataVolume was created deletes it (best effort).

The token is never logged, stored or returned. TLS verification is never disabled: `https://` is required (plain `http://` only for loopback) and a private CDI CA can be supplied as PEM.

## Configuration

| Variable | Meaning |
|----------|---------|
| `ZORVIA_CDI_UPLOAD_PROXY_URL` | `https://` URL where the CDI upload proxy is reachable from where Zorvia runs (Route, Ingress, NodePort or in-cluster `https://cdi-uploadproxy.cdi.svc` when Zorvia runs in the cluster). Not auto-discovered. |
| `ZORVIA_CDI_UPLOAD_PROXY_CA_FILE` | Optional PEM CA for the proxy certificate |
| `ZORVIA_IMAGE_UPLOAD_MAX_BYTES` | Hard cap on bytes sent (default 20 GiB) |

## CLI

```bash
zorvia --namespace vms guest image-upload ./ubuntu.qcow2 --name ubuntu-24 --size 20Gi \
  --sha256 <hex> --proxy-url https://cdi-upload.example.com
zorvia guest image-upload ./disk.img --name x --size 10Gi --validate-only   # no cluster
```

`--size` is the PVC size and must cover the virtual disk size, not just the file size.

## API

All under `/api`, `POST` needs `vm.create` (same as other `/images/*` writes), and the feature gate applies.

| Route | Purpose |
|-------|---------|
| `POST /images/uploads/{name}?size=&namespace=&storage_class=&format=&sha256=` | Raw image as the request body; streamed through to CDI. `201` with name, format, bytes, sha256. Errors: `400 INVALID_IMAGE`, `413 IMAGE_TOO_LARGE`, `422 CHECKSUM_MISMATCH`, `502 UPLOAD_FAILED`, `503 UPLOAD_PROXY_NOT_CONFIGURED` |
| `GET /images/uploads/{name}?namespace=` | DataVolume phase and progress |
| `GET /images/upload-config` | The effective size cap: `{max_bytes, default_max_bytes}` (the web panel shows `max_bytes`) |
| `POST /images/validate?format=` | Send the first ~33 KiB of an image; returns the detected format |

These routes are registered outside the 30 s request timeout that wraps the rest of `/api`. A reverse proxy in front of Zorvia must also allow large bodies and long requests.

## Web console

`/app/disk-images` has an upload panel (user/admin role): choose a file, name, PVC size and optional format, namespace, StorageClass and SHA-256; **Validate only** sends just the first ~33 KiB to `/images/validate`; **Upload** streams the file with a progress bar (bytes sent to Zorvia; the request finishes once CDI accepts the image). The panel shows the server's real size cap (from `/images/upload-config`). Size-cap, checksum and proxy errors get plain-language messages. When the API answers 403 `EXPERIMENTAL_DISABLED` the panel shows an experimental, disabled notice instead.

## Not done

- No server-side conversion or conversion between formats (CDI converts to raw during upload).
- No compressed (`.gz`/`.xz`) input, no resumable uploads.
- No live-cluster verification; the CDI version and storage classes it works with are untested.
