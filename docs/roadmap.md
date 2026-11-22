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
| Disaster recovery | Site failover, cross-site replication | DR replication pairs with failover and failback are Experimental ([DR_REPLICATION.md](DR_REPLICATION.md), same-cluster object store, no traffic repointing); a real cross-site engine is not built |
| Certificates & encryption | Cert lifecycle (Experimental, see [CERTIFICATE_LIFECYCLE.md](CERTIFICATE_LIFECYCLE.md)); volume encryption with local, Vault, AWS KMS and GCP KMS key wrapping and guest LUKS (Experimental, see [VOLUME_ENCRYPTION.md](VOLUME_ENCRYPTION.md)) | Issuance and alert delivery (email/webhook) are not built; cloud KMS never tried against a real endpoint; no Azure Key Vault; CSI does not consume the key; guest LUKS has no automatic token delivery |
| Image upload / convert | Upload a disk image from your browser, format conversion | Upload through the CDI proxy is Experimental ([IMAGE_UPLOAD.md](IMAGE_UPLOAD.md), mock-proxy tested only); conversion is left to CDI |
| Autoscaling | Policy-driven automatic VM scaling | An opt-in loop with metrics-API/Prometheus load is Experimental and recommendation-only by default ([AUTOSCALER_LOOP.md](AUTOSCALER_LOOP.md)); its executor is not verified on a live cluster |
| Datacenters & resource pools | vCenter-style hierarchical grouping | Label-based datacenter/cluster/folder inventory exists; resource pools (labels + advisory limits) are Experimental, see [RESOURCE_POOLS.md](RESOURCE_POOLS.md); no Kubernetes primitive, so limits are not enforced |
| Enterprise SSO | OIDC/SAML with PKCE + JWKS | OIDC Beta via `ZORVIA_OIDC_ENABLED=1`; SAML is Experimental, login only in a `saml`-feature build and untested against a real IdP ([SAML.md](SAML.md)) |
| S3 immutable backup / Transiva / GPU-NUMA | Phase 5 plan APIs | [docs/PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) |

If one of these is a blocker for adopting Zorvia in your environment, that's
exactly the kind of thing worth a conversation — [reach out](../README.md#get-involved)
and tell us what you need.
