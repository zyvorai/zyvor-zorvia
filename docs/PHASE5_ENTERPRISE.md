# Phase 5 — Enterprise capabilities

Tracked in [FEATURE_MATURITY.md](FEATURE_MATURITY.md). All Phase 5 endpoints require:

1. `ZORVIA_EXPERIMENTAL=1`
2. The per-capability `ZORVIA_FEATURE_*=1` flag
3. An authenticated caller with **`cluster.admin`** (admin JWT or scoped token)

Plan endpoints return **dry-run plans** (JSON intent). Fleet inventory performs
live read-only Kubernetes queries; the golden-image run endpoint applies CDI
resources. Fleet does not mutate remote clusters.

| Capability | Env flag | API |
|------------|----------|-----|
| S3 immutable backups | `ZORVIA_FEATURE_S3_BACKUP=1` | `POST /api/v1/enterprise/s3-backup/plan` |
| Cross-cluster DR | `ZORVIA_FEATURE_CROSS_CLUSTER_DR=1` | `POST /api/v1/enterprise/cross-cluster-dr/plan` and `/run` (off-cluster backup→restore rehearsal) |
| Transiva VMware migration | `ZORVIA_FEATURE_TRANSIVA=1` | `POST /api/v1/enterprise/transiva/plan` |
| Golden-image pipeline | `ZORVIA_FEATURE_GOLDEN_PIPELINE=1` | `POST /api/v1/enterprise/golden-pipeline/plan` and `/run` |
| GPU/SR-IOV/NUMA | `ZORVIA_FEATURE_GPU_NUMA=1` | `POST /api/v1/enterprise/placement/gpu-numa` |
| Fleet multi-cluster | `ZORVIA_FEATURE_FLEET=1` | `GET /api/v1/enterprise/fleet` |

Implementation module: `src/enterprise/mod.rs`. Live registry: `GET /api/v1/features`.

## Example requests

```bash
TOKEN=$(curl -sk -X POST https://HOST:30152/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"…"}' | jq -r .token)

# Without flags → 403 feature_disabled
curl -sk -X POST https://HOST:30152/api/v1/enterprise/s3-backup/plan \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"vm_name":"app","snapshot_name":"snap-1","bucket":"zorvia-backups","region":"us-east-1","retention_days":30}'

# With ZORVIA_EXPERIMENTAL=1 and ZORVIA_FEATURE_S3_BACKUP=1 on the API pod → 200 plan
```

### Bodies

**S3 backup plan**

```json
{
  "vm_name": "app",
  "snapshot_name": "snap-1",
  "bucket": "zorvia-backups",
  "region": "us-east-1",
  "prefix": "prod",
  "retention_days": 30
}
```

**Transiva plan**

```json
{
  "source_vm": "win-sql",
  "vcenter_endpoint": "https://vcenter.example.com",
  "target_namespace": "migrated"
}
```

**Golden pipeline plan**

```json
{
  "image_name": "ubuntu-golden",
  "version": "24.04-20260922",
  "namespace": "default"
}
```

**Golden pipeline run** (applies CDI DataVolume + DataSource)

```json
{
  "image_name": "ubuntu-golden",
  "version": "24.04-20260922",
  "namespace": "default",
  "source": "quay.io/containerdisks/ubuntu:24.04",
  "size": "20Gi"
}
```

**Cross-cluster DR plan**

```json
{
  "source_cluster": "dc1",
  "target_cluster": "dc2",
  "vm_names": ["app", "db"],
  "rpo_seconds": 300
}
```

**GPU / SR-IOV / NUMA plan**

```json
{
  "vm_name": "ml-worker",
  "gpu_count": 1,
  "sriov_networks": ["sriov-net"],
  "numa_passthrough": true
}
```

**Fleet inventory** — `GET /api/v1/enterprise/fleet` (no body) collects live local
and explicitly enrolled remote nodes and VMs. Requires `cluster.admin`.
See [FLEET.md](FLEET.md) for registry, credentials, read-only RBAC and limits.

## Enabling on a lab Deployment

```yaml
env:
  - name: ZORVIA_EXPERIMENTAL
    value: "1"
  - name: ZORVIA_FEATURE_S3_BACKUP
    value: "1"
  # add other ZORVIA_FEATURE_* as needed
```

Do **not** leave `ZORVIA_EXPERIMENTAL=1` on production clusters.

## Related

- CDI golden images (shipping today): [GOLDEN_IMAGES.md](GOLDEN_IMAGES.md)
- OIDC SSO (separate from Phase 5 flags): [OIDC.md](OIDC.md)
- Roadmap wording: root [README.md](../README.md)
