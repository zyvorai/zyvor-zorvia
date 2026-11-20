# TLS certificate rotation

The API server reloads its certificate **without a restart**. A background task fingerprints the
certificate and key files (`ZORVIA_TLS_CERT` / `ZORVIA_TLS_KEY`, or `--tls-cert` / `--tls-key`) every
`ZORVIA_TLS_RELOAD_SECS` seconds (default **60**; `0` turns it off). When their content changed it asks the
server to load them; new connections use the new certificate, connections already open are not dropped.

- A reload that fails (half-written files, a key that does not match the certificate) is logged
  (`TLS certificate reload failed (keeping the previous one)`, once per distinct file content) and the **previous
  certificate keeps serving**; the next change is tried again.
- File **content** is compared, not modification time, so Kubernetes' symlink swap when a mounted Secret updates is
  picked up (a mounted Secret can take a minute or more to reach the pod; that delay is Kubernetes', not Zorvia's).
- A file that cannot be read for a moment (a swap in progress) is ignored and re-read at the next poll.

## With cert-manager

Mount the `Certificate`'s Secret into the pod and point the TLS paths at it; cert-manager renews it and the new
files reach the pod through the Secret volume. The Helm chart's `tls.inPod` default generates a self-signed
certificate in an init container (valid 10 years, not rotated); for rotation use your issuer, not that one.

```yaml
# Certificate (cert-manager) -> Secret zorvia-tls, mounted at /tls in the Zorvia pod
env:
  - {name: ZORVIA_TLS_CERT, value: /tls/tls.crt}
  - {name: ZORVIA_TLS_KEY,  value: /tls/tls.key}
  - {name: ZORVIA_TLS_RELOAD_SECS, value: "30"}
```

## What this does not cover

Expiry monitoring (Zorvia does not parse the certificate or alert before it expires: use cert-manager or your
monitoring), the CA bundle used for *outgoing* connections, and client-certificate (mTLS) authentication.
