# Real-guest E2E

`guest.sh` boots a Linux guest through the Zorvia API and exercises it: create + boot + guest
agent, snapshot, live migration under CPU/disk load (needs two schedulable nodes and `KUBECTL`),
and an API restart that must leave the session and the VM untouched. Output is a markdown table
meant for `docs/SUPPORT_MATRIX.md`; a scenario that cannot run reports SKIP with the reason, never
a silent pass.

```bash
ZORVIA_LAB_HOST=80.79.5.173 ZORVIA_E2E_PASSWORD=... \
KUBECTL="ssh root@80.79.5.173 KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl" \
tests/e2e/guest.sh
```

Not covered yet: Windows guests, node failure, upgrades, off-cluster backup/restore/drill (those
are exercised separately, see `docs/OFFCLUSTER_BACKUP.md`). CI runs it on demand through the
`guest-lab` job of `e2e-kubevirt.yml` on a self-hosted `zorvia-lab` runner.

## Data snapshot on CSI storage

Set `E2E_STORAGE_CLASS` (with `KUBECTL`) to add a scenario that boots a guest with a real
PVC-backed data disk and checks the snapshot captures it (no config-only warning, a
`VolumeSnapshot` ready). The blank DataVolume is created with `volumeMode: Filesystem`: on the
Ceph RBD class CDI defaults to Block mode and its importer crashes with
`cannot open /dev/cdi-block-volume: Permission denied`, (the backup agent itself handles Block sources, so this only concerns CDI imports).
