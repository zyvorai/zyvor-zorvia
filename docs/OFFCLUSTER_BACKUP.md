# Off-cluster backup, restore and recovery drills

A backup normally ends as a cluster-local `VirtualMachineSnapshot`. With an S3
target configured, the backup operation continues: each volume snapshot is
restored to a temporary PVC, streamed to S3 by the `backup-agent` Job
(checksummed, optionally AES-256-GCM encrypted, Object Lock aware), read back
and re-verified, and only then is the manifest published.

## Configure (environment on the Zorvia deployment)

| Variable | Purpose |
|---|---|
| `ZORVIA_BACKUP_S3_ENDPOINT`, `_BUCKET`, `_SECRET` | Required to enable off-cluster backups |
| `ZORVIA_BACKUP_S3_REGION`, `_PREFIX` | Optional (`us-east-1`, empty) |
| `ZORVIA_BACKUP_S3_ACCESS_KEY_KEY`, `_SECRET_KEY_KEY` | Key names inside the Secret (`access-key`, `secret-key`) |
| `ZORVIA_BACKUP_ENCRYPTION_KEY_KEY`, `_ID` | Secret key holding a 64-hex-char AES-256 key; label recorded in the manifest |
| `ZORVIA_BACKUP_LOCK_MODE` (`GOVERNANCE`/`COMPLIANCE`), `_LOCK_DAYS` | Object Lock retention applied to every object |
| `ZORVIA_BACKUP_PART_MB`, `_JOB_DEADLINE_SECS` | Multipart size (default 64) and Job deadline (default 6h) |
| `ZORVIA_BACKUP_AGENT_IMAGE` | Agent image (`deploy/backup-agent.Dockerfile`) |

### Using an Atlas bucket as the target

If you run [Atlas](https://github.com/zyvorai/atlas), point Zorvia at one of its
buckets instead of typing the endpoint, bucket and Secret yourself:

```
ATLAS_URL=http://atlas-gateway.zyvor-system:5110      # already needed for the Atlas integration
ZORVIA_BACKUP_ATLAS_BUCKET=bkt_9624f5a6596f            # id from GET /api/atlas/v1/buckets
```

Zorvia reads the bucket record from Atlas (it must be `bound`) and takes the
S3 `endpoint`, `bucket_name`, `region` and credential `secret_ref` from it. Atlas
buckets are Rook ObjectBucketClaims, so the Secret uses the `AWS_ACCESS_KEY_ID` /
`AWS_SECRET_ACCESS_KEY` keys; Zorvia defaults to those names in this mode.
Prefix, encryption, Object Lock and part size still come from the
`ZORVIA_BACKUP_*` variables above. Atlas's object storage is Ceph RGW (or any
S3 endpoint you register with it), so this needs a working RGW behind Atlas.

The Secret lives in the bucket's namespace (the OBC namespace, e.g.
`rook-ceph`), but the Job runs in the VM's namespace, so copy it there (Zorvia
does not read Secrets).

The credential **Secret must exist in the namespace of every VM you back up**
(and in the drill namespace). Zorvia deliberately has no cluster-wide Secret
access; a missing Secret fails the operation immediately with a clear message.
Keep the encryption key somewhere other than the bucket: without it an
encrypted backup cannot be restored.

Verified end to end (encrypted backup, read-back, restore, recovery drill, key check) on KubeVirt v1.9.0 / CDI v1.66.0 with Rook-Ceph RBD and the Kubernetes hostpath CSI driver, and an Atlas-provisioned RGW bucket (maturity: Beta); see [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for versions and timings. Block-mode source volumes are supported for backup (see below).

## Back up now

`POST /api/backups/offcluster` with `{"vm_name": "...", "namespace"?, "retention_days"?, "description"?}`
queues the same durable operation the scheduler uses (snapshot, upload, read-back verify) and returns
`{"operation_id"}` to poll on `GET /api/operations/{id}`. It answers 409 if no target is configured.
Scheduled backups use it automatically once a target is set.

### Block-mode volumes

Source volumes with `volumeMode: Block` (the default on some RBD classes) are supported for backup. The temp PVC stays Block and is attached to the agent Job as a raw device; the agent measures it by seeking to the end and streams it. Because the device node is `root:disk 0660`, Jobs with a Block source run the agent as root (still `privileged: false`, no privilege escalation, every capability dropped); filesystem-only Jobs keep the unprivileged CDI UID. The manifest records that a disk was Block (`source_block`), and **a restore defaults to the same mode**: a Block source is restored into a new Block volume (sized to the raw image rounded up to whole GiB, attached to the restore Job as a writable device, written without truncation and refused if smaller than the backup), a Filesystem source into a `disk.img` file as before. Override with `volume_mode` (`Block` or `Filesystem`) on the restore request, e.g. to move a Block disk onto a storage class that only offers Filesystem. Jobs that attach a Block volume run the agent as root (still unprivileged, every capability dropped). Backups taken before this change carry no `source_block` and restore as Filesystem. Verified on Ceph RBD: backup -> restore into a Block volume and, separately, into a Filesystem volume both reproduce the source SHA-256.

## Restore

`GET /api/backups/offcluster` lists restorable backups.
`POST /api/backups/offcluster/{operation_id}/restore` with
`{"new_vm_name": "...", "namespace"?, "storage_class"?, "start"?, "volume_mode"?}` restores the
disks into new PVCs (each part verified before it is written; existing files
are never overwritten) and creates a new VM from the saved spec. Firmware
uuid/serial and MAC addresses are dropped so it cannot collide with the
original. The VM is created halted unless `start` is true. Poll
`GET /api/operations/{id}`.

## Checking that the encryption key still works

`POST /api/backups/offcluster/{operation_id}/verify-key` (cluster.admin) queues a `backup-verify-key`
operation. It runs the agent in `verify-key` mode in the VM's namespace: it reads the manifest and the
**first part of each disk**, checks it against the manifest checksum and proves the configured key
decrypts it, then stops. No volumes, no VM, and only about one part (default 64 MiB) per disk is read.
A wrong or rotated key fails with `decryption failed (wrong key or corrupted part)`.

It proves the key opens the backup, not that the whole disk restores or boots; schedule recovery drills
for that. For an unencrypted backup it still checks the first part's checksum.

## Recovery drills

`POST /api/backups/offcluster/{operation_id}/drill`, or set
`ZORVIA_DRILL_INTERVAL_HOURS` to drill the latest backup of every VM on a
schedule. A drill restores into `ZORVIA_DRILL_NAMESPACE` (default
`zorvia-drill`, with a NetworkPolicy that denies all traffic to and from the restored guest's virt-launcher pod; the restore Job in the same namespace is unaffected), replaces the VM's
networks with a bare pod network, boots it, waits for the guest agent
(`ZORVIA_DRILL_BOOT_TIMEOUT_SECS`, default 600), then deletes the VM and its
volumes. The operation result records restore and boot times. A drill proves
the backup restores and the guest boots with a working agent; it does not run
application-level checks.

## Retention

Cluster-local backup snapshots are deleted hourly once past their
`retention_days`; the newest backup of each VM is always kept. Off-cluster
objects are protected by Object Lock at write time and expire through a bucket
lifecycle rule -- Zorvia holds no delete rights on the store.

## Lab S3 (same-cluster rehearsal)

`scripts/lab-minio-backup.sh` deploys an in-cluster S3-compatible store
(default `adobe/s3mock` — pulls where MinIO/Quay do not; override with
`ZORVIA_LAB_S3_IMAGE`), creates bucket `zorvia-backups`, and copies Secret
`backup-s3` (`access-key` / `secret-key`) into the VM and DR namespaces.
Point the API Deployment at:

```
ZORVIA_BACKUP_S3_ENDPOINT=http://minio.zorvia-lab-minio.svc:9090   # 9000 for MinIO
ZORVIA_BACKUP_S3_BUCKET=zorvia-backups
ZORVIA_BACKUP_S3_SECRET=backup-s3
ZORVIA_BACKUP_AGENT_IMAGE=ghcr.io/zyvorai/zorvia-backup-agent:latest
```

PVC disks need a CSI `VolumeSnapshotClass` for off-cluster upload (k3s
`local-path` alone yields a Succeeded VirtualMachineSnapshot with no
volumeBackups — `/run` still queues; restore correctly refuses
cluster-local-only ops). With CSI snapshots, `/run` backup → `/run` with
`backup_operation_ids` restores as `{vm}-dr`.

## Limits

Filesystem-mode volumes only (block-mode volumes are rejected). Restores read
`<pvc>/disk.img` -- verify the layout and that the agent's UID (107 by
default, `ZORVIA_BACKUP_AGENT_UID`) can read it on your storage class before
relying on this in production.
