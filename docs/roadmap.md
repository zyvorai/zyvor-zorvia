# Roadmap

A feature-maturity registry, not a marketing slide.

[Back to the README](../README.md) · [Docs index](README.md)

The roadmap is a feature-maturity registry, not a marketing slide: see
**[docs/FEATURE_MATURITY.md](FEATURE_MATURITY.md)** for GA / Beta /
Experimental / Model-only. Only **GA** and documented **Beta** paths are
production promises — query `GET /api/v1/features` on a running API for the
live registry.

Capabilities that need genuinely new infrastructure (not yet shipped):

| Area | What's planned | Why it's not here yet |
|------|-----------------|------------------------|
| Disaster recovery | Site failover, cross-site replication | Needs a real DR/replication engine (`dr-replication` is model-only) |
| Certificates & encryption | Cert lifecycle, disk/volume encryption (KMS) | Needs PKI and key-management integration from scratch |
| Image upload / convert | Upload a disk image from your browser, format conversion | Needs a CDI upload-proxy client (TLS, multipart streaming) |
| Autoscaling | Policy-driven automatic VM scaling | Policy engine exists; needs an execution loop against real load |
| Datacenters & resource pools | vCenter-style hierarchical grouping | No equivalent Kubernetes primitive to build on yet |
| Enterprise SSO | OIDC/SAML with PKCE + JWKS | OIDC Beta via `ZORVIA_OIDC_ENABLED=1`; SAML not yet |
| S3 immutable backup / Transiva / GPU-NUMA | Phase 5 plan APIs | [docs/PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) |

If one of these is a blocker for adopting Zorvia in your environment, that's
exactly the kind of thing worth a conversation — [reach out](../README.md#get-involved)
and tell us what you need.
