# Certificate lifecycle (Experimental)

Status: **Experimental**, behind `ZORVIA_EXPERIMENTAL=1`. Unit-tested with generated certificates and an in-memory fake of the cluster. **Not run against a live cluster, a real cert-manager or a real multi-namespace RBAC setup.**

What exists beyond this page: the API server reloads its own serving certificate without a restart ([TLS_ROTATION.md](TLS_ROTATION.md)). This feature adds visibility and alerting on top.

## What it does

1. **Inventory.** Lists X.509 certificates from:
   * `kubernetes.io/tls` Secrets (`tls.crt` and, when present, `ca.crt`) in every namespace the service account may read;
   * Zorvia's own serving certificate (`ZORVIA_TLS_CERT` / `--tls-cert`);
   * the CA files Zorvia is configured with: `ZORVIA_CDI_UPLOAD_PROXY_CA_FILE` and `ZORVIA_DATABASE_CA_FILE`.

   OIDC, Vault and S3 have **no CA-file setting** in Zorvia today, so there is nothing to inventory for them. (A CA you pass with `--ca-cert` to the CLI is the client's, not the server's.)
2. **Metadata only.** Per certificate: subject, issuer, SANs (DNS, IP, email, URI), not-before/after, whole days remaining, key type and size (RSA bits, EC curve), signature algorithm, self-signed, SHA-256 fingerprint, and flags (`expired`, `not-yet-valid`, `self-signed`, `weak-signature:<alg>` for MD2/MD4/MD5/SHA-1, `weak-key:RSA-<bits>` below 2048, DSA, EC below 224 bits).
3. **Expiry alerts.** A scheduler loop (leader-only, see [SCHEDULER_LEADERSHIP.md](SCHEDULER_LEADERSHIP.md)) rescans every `ZORVIA_CERT_CHECK_INTERVAL_SECS` (default 3600, minimum 60) and raises an alert when a certificate crosses a threshold from `ZORVIA_CERT_ALERT_DAYS` (default `30,14,7,1`; whole days, 1..3650, at most 10).
4. **cert-manager.** Lists `Certificate` objects with their Ready condition and offers an admin-only, audited, confirmed **renew**.

## Key and secret handling

* Only `tls.crt` and `ca.crt` are copied out of a Secret. `tls.key` and every other key are dropped on receipt and never parsed, logged, stored or returned. A private key given where a certificate is expected is rejected without echoing it.
* Responses and alerts contain metadata only: no PEM text, no Secret data. Unreadable files are reported as "not readable" without the path.
* Parsing uses OpenSSL (already a dependency, vendored), with a 1 MiB input cap and 64 certificates per bundle. **No new crate was added** (no `x509-parser`/`rcgen`), so `Cargo.lock` and `deny.toml` are unchanged.
* Listing Secrets means the Zorvia process briefly holds each TLS Secret's full API response in memory (the Kubernetes API cannot return only selected keys).

## Expiry alert semantics

* Once per threshold per certificate: a threshold is "crossed" when the remaining time is at or below `N * 24h`. Alerts are not repeated on later passes or after a restart (state is persisted through the same document store as DR pairs: a file, or PostgreSQL when `ZORVIA_DATABASE_URL` is set).
* A certificate first seen already inside several thresholds produces **one** alert, for the tightest one; all crossed thresholds are marked reported.
* Thresholds of 7 days or less raise an Error-level notification, larger ones a Warning. An expired certificate raises one Error. When the certificate content behind the same Secret changes (renewal) after alerts had fired, one Info alert says so and the counters reset.
* Intermediate certificates in a `tls.crt` chain are listed but not alerted on; the leaf, `ca.crt` entries and configured CA files are.
* A failed scan does not touch alert state (it would otherwise look like every certificate vanished); the error is shown in the status.
* State for a certificate not seen for 7 days is forgotten.
* Alerts go to the notifications module's persisted list (`GET /api/certificates/alerts`) and the server log. There is **no email/webhook delivery**; use your log pipeline or poll the endpoint. With several replicas only the lease leader scans.

## API (cluster.admin only, every method)

| Route | Purpose |
|-------|---------|
| `GET /api/certificates/status` | thresholds, interval, leader flag, last check, watched files |
| `GET /api/certificates/inventory[?namespace=NS]` | certificates + summary + `skipped` notes (the namespace filter also hides the file-based certificates) |
| `GET /api/certificates/alerts` | recent alerts |
| `GET /api/certificates/cert-manager` | `{installed, certificates:[{namespace,name,secret_name,ready,ready_reason,ready_message,issuing,not_after,renewal_time,issuer,dns_names}]}` |
| `POST /api/certificates/cert-manager/renew` | `{"namespace","name","confirm":"renew <namespace>/<name>"}`, audited (`certificate`, High) |

Without `ZORVIA_EXPERIMENTAL=1` every route answers `403 EXPERIMENTAL_DISABLED`.

## RBAC the service account needs

The Helm chart does **not** grant these; they are opt-in for this feature:

```yaml
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list"]            # tls.crt/ca.crt only are used; cluster-wide ClusterRole for all namespaces
- apiGroups: [""]
  resources: ["namespaces"]
  verbs: ["list"]                    # only for the per-namespace fallback
- apiGroups: ["cert-manager.io"]
  resources: ["certificates"]
  verbs: ["get", "list"]
- apiGroups: ["cert-manager.io"]
  resources: ["certificates/status"]
  verbs: ["update"]                  # only for the renew action
```

Granting `secrets` `get`/`list` cluster-wide lets that service account read every Secret, including private keys. Prefer a Role/RoleBinding in the namespaces you want inventoried. If the cluster-wide list is forbidden, Zorvia lists namespaces and each namespace it may read (or just its own namespace when namespaces cannot be listed); the unreadable ones appear under `skipped`.

## Renewing a cert-manager Certificate

The supported mechanism is the one `cmctl renew` uses: set the Certificate's `Issuing` condition to `True` (reason `ManuallyTriggered`) through the `status` subresource, with optimistic concurrency on `resourceVersion`. cert-manager's controller then re-issues and replaces the Secret. Zorvia does not delete the Secret (that breaks consumers and does not use cert-manager's own flow) and does not edit `spec`.

* It refuses when the Certificate is already `Issuing`, is not found, or cert-manager is not installed (404 `CERT_MANAGER_NOT_INSTALLED`).
* The request must carry the exact phrase `renew <namespace>/<name>`; the console asks for confirmation, the CLI needs `--confirm`.
* Renewal does not make workloads reload the new certificate; the Zorvia serving certificate does (see TLS_ROTATION.md), other consumers may need a restart.
* Not verified against a live cert-manager; field names follow the documented `cert-manager.io/v1` schema.

## CLI

```bash
zorvia certs list [--namespace NS] [--expiring-within-days 30]
zorvia certs status
zorvia certs alerts
zorvia certs cert-manager list
zorvia certs cert-manager renew web site --confirm "renew web/site"
```

See [CLI_REMOTE.md](CLI_REMOTE.md). `--expiring-within-days` filters on the client (expired certificates are kept).

## Web console

`/app/certificates` (admin): monitoring status, the inventory with search and an expiring/flagged filter, skipped-namespace notes, cert-manager Certificates with a Renew button behind a confirmation dialog, and recent alerts. A 403 `EXPERIMENTAL_DISABLED` shows the experimental, disabled notice.

## Not done

* No email/webhook/Slack delivery of alerts.
* No certificate issuance, no rotation of non-cert-manager Secrets, no `Ingress`/`Gateway` certificate discovery, no mTLS client certificates, no trust-store (system CA) inventory.
* No live-cluster, RBAC-matrix or cert-manager verification.
