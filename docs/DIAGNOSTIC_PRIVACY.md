# Diagnostic privacy

> **Status: Beta.** `POST /api/v1/support/diagnostics` (`cluster.admin`) previews or downloads a bundle. Uploading is a separate route that needs `confirm: true`. Application logs are **not** collected yet (Zorvia does not retain them); the `logs` section says so.

## A bundle contains

Zorvia, Kubernetes, KubeVirt and CDI versions; sanitized VM specifications and recent events; relevant storage and scheduling conditions; operation failures from the audit trail; component health and configuration summaries.

## A bundle never contains

Credentials, Secret contents, kubeconfigs, tokens, cloud-init user data, guest disks or memory dumps.

## Handling rules

- Free-form text is redacted for sensitive values.
- An administrator can preview the exact contents before export.
- Default is a **local download**. Uploading to support is a separate, explicit action.
- Attachment size limits and retention are configurable (`ZORVIA_SUPPORT_ATTACHMENT_*`).
- Case and attachment access is isolated by customer organization.

## How it is enforced

- The bundle is built from an allow-list of fields; Secrets, kubeconfigs and guest disks are never requested from the cluster.
- Every value then passes through redaction: keys such as `password`, `secret`, `token`, `kubeconfig`, `cloudInit*`, `userData` lose their value, env-style `{name: DB_PASSWORD, value: ...}` pairs lose the value, and strings are scrubbed for bearer/basic credentials, JWTs, PEM blocks, `key=value` secrets, URL credentials, AWS keys and long base64 blobs. Redaction errs toward removing too much.
- Each section fails independently (`{"error": ...}`), so a partial cluster outage still yields a usable bundle.
- The default call is a **preview** that returns the manifest (sections, sizes, sha256, exclusions) and the full bundle. Nothing is sent anywhere.
- Upload attaches a freshly generated bundle to a case you can access, is limited by the attachment size cap, and is recorded on the case as a customer action.
