#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

# Real-guest E2E against a running Zorvia API on a KVM-capable cluster.
#
# Unlike the kind smoke job this boots a guest and drives it through the API:
# create+boot (guest ready) -> snapshot/revert -> live migration under CPU
# load -> API restart with the guest untouched. Each scenario prints one
# markdown row (PASS / FAIL / SKIP + seconds) so a run can be pasted into
# docs/SUPPORT_MATRIX.md. Exit status is non-zero if any scenario FAILed.
#
#   ZORVIA_LAB_HOST=80.79.5.173 ZORVIA_E2E_PASSWORD=... tests/e2e/guest.sh
#
# Env: ZORVIA_LAB_HOST/PORT, ZORVIA_E2E_USER (admin) + ZORVIA_E2E_PASSWORD or
#      ZORVIA_E2E_TOKEN; E2E_IMAGE (containerdisk), E2E_VM (name prefix);
#      E2E_GUEST_AGENT=zyvor|qemu|none (default zyvor) installs and requires a connected guest agent;
#      E2E_BACKUP=1 adds off-cluster backup -> restore -> drill (the ZORVIA_BACKUP_* target
#      must be configured on the deployment);
#      KUBECTL (e.g. "ssh root@host KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl")
#      E2E_STORAGE_CLASS (a snapshot-capable CSI class) enables the data-snapshot scenario;
#      KUBECTL enables the node check (E2E_NAMESPACE = the API's default namespace) and the API-restart scenario.
set -uo pipefail

HOST="${ZORVIA_LAB_HOST:?set ZORVIA_LAB_HOST}"
BASE="https://${HOST}:${ZORVIA_LAB_PORT:-30152}"
IMAGE="${E2E_IMAGE:-quay.io/containerdisks/ubuntu:24.04}"
VM="${E2E_VM:-zorvia-e2e}-$RANDOM"
KUBECTL="${KUBECTL:-}"
NS="${E2E_NAMESPACE:-default}"
AGENT="${E2E_GUEST_AGENT:-zyvor}"   # zyvor | qemu | none: installed through the create API (cloud-init)   # the namespace the API deploys VMs into
FAILS=0

TOKEN="${ZORVIA_E2E_TOKEN:-}"
if [ -z "$TOKEN" ]; then
  : "${ZORVIA_E2E_PASSWORD:?set ZORVIA_E2E_PASSWORD or ZORVIA_E2E_TOKEN}"
  TOKEN=$(curl -skf -X POST "$BASE/api/v1/auth/login" -H 'Content-Type: application/json' \
    -d "{\"username\":\"${ZORVIA_E2E_USER:-admin}\",\"password\":\"${ZORVIA_E2E_PASSWORD}\"}" | jq -r .token)
fi
[ -n "$TOKEN" ] && [ "$TOKEN" != null ] || { echo "login failed" >&2; exit 2; }

api() { # method path [json]
  curl -sk -m 900 -X "$1" "$BASE/api$2" -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' ${3:+-d "$3"} -w '\n%{http_code}'
}
code() { tail -n1; }
body() { sed '$d'; }

row() { printf '| %s | %s | %ss | %s |\n' "$1" "$2" "$3" "${4:-}"; [ "$2" = FAIL ] && FAILS=$((FAILS+1)); }
run() { # name fn
  local t0=$SECONDS out rc
  out=$("$2" 2>&1); rc=$?
  case $rc in 0) row "$1" PASS $((SECONDS-t0)) "$out" ;; 77) row "$1" SKIP $((SECONDS-t0)) "$out" ;; *) row "$1" FAIL $((SECONDS-t0)) "$out" ;; esac
}
cleanup() {
  api DELETE "/vms/$VM" >/dev/null 2>&1
  if [ -n "$KUBECTL" ]; then
    api DELETE "/vms/$VM-pvc" >/dev/null 2>&1
    api DELETE "/vms/$VM-restored" >/dev/null 2>&1
    rm -f "${TMPDIR:-/tmp}/e2e-backup-op-$VM"
    if [ -s "${TMPDIR:-/tmp}/e2e-restored-pvcs-$VM" ]; then   # restored PVCs outlive their VM
      sleep 15
      while read -r pvc; do $KUBECTL -n "$NS" delete pvc "$pvc" --ignore-not-found --wait=false >/dev/null 2>&1; done < "${TMPDIR:-/tmp}/e2e-restored-pvcs-$VM"
      rm -f "${TMPDIR:-/tmp}/e2e-restored-pvcs-$VM"
    fi
    $KUBECTL -n "$NS" delete vmsnapshot "$VM-pvc-d1" --ignore-not-found >/dev/null 2>&1
    $KUBECTL -n "$NS" delete dv "$VM-pvc-data" --ignore-not-found >/dev/null 2>&1
  fi
}
trap cleanup EXIT

wait_for() { # seconds cmd...
  local deadline=$((SECONDS+$1)); shift
  until "$@"; do [ $SECONDS -ge $deadline ] && return 1; sleep 5; done
}

# GET /vms/{name} reports a lowercase `state` (starting, running, ...).
vm_running() { api GET "/vms/$VM" | body | jq -e '.state == "running"' >/dev/null 2>&1; }

s_create_boot() {
  local r ga=""
  [ "$AGENT" = none ] || ga="\"guest_agent\":\"$AGENT\","
  r=$(api POST /vms "{\"name\":\"$VM\",$ga\"image\":\"$IMAGE\",\"cpus\":2,\"memory\":2048,\"disk\":10,
    \"cloud_init\":{\"user_data\":\"#cloud-config\nruncmd:\n  - [sh, -c, 'while :; do :; done &']\n  - [sh, -c, 'dd if=/dev/zero of=/var/tmp/load bs=1M count=512 conv=fsync; sync']\"}}")
  # The API answers within 30 s; 408 only means the guest was not ready yet.
  case "$(echo "$r" | code)" in 2??|408) ;; *) echo "create: $(echo "$r" | body | head -c 200)"; return 1 ;; esac
  wait_for 900 vm_running || { echo "never Running"; return 1; }
  # wait-ready answers 200 when the guest has an IP and Ready=True; whether the
  # qemu guest agent itself is connected is reported separately, so show both.
  agent_up() { [ "$(api POST "/vms/$VM/wait-ready" | code)" = 200 ]; }
  wait_for 900 agent_up || { echo "guest never became ready"; return 1; }
  if [ "$AGENT" != none ]; then
    # cloud-init installs the agent after first boot (download + install), so give it time.
    connected() { [ "$(api POST "/vms/$VM/wait-ready" | body | jq -r .agent_connected)" = true ]; }
    wait_for 900 connected || { echo "guest ready but the $AGENT guest agent never connected"; return 1; }
    echo "guest ready (IP + Ready=True), $AGENT guest agent connected"
  else
    echo "guest ready (IP + Ready=True), no guest agent requested"
  fi
}

s_snapshot_revert() {
  local r id warn
  r=$(api POST "/vms/$VM/snapshots" '{"name":"e2e-snap","description":"e2e"}')
  [ "$(echo "$r" | code)" -lt 300 ] || { echo "snapshot: $(echo "$r" | body | head -c 200)"; return 1; }
  id=$(echo "$r" | body | jq -r .id)   # the API prefixes the VM name
  listed() { api GET "/vms/$VM/snapshots" | body | jq -e --arg id "$id" 'any(.[]; .id == $id)' >/dev/null 2>&1; }
  wait_for 120 listed || { echo "snapshot $id never listed"; return 1; }
  # A containerdisk guest has no PVC, so the snapshot holds configuration only;
  # the API says so in `warning`. Report it rather than calling that a data snapshot.
  warn=$(api GET "/vms/$VM/snapshots" | body | jq -r --arg id "$id" '.[] | select(.id == $id) | .warning // empty')
  r=$(api DELETE "/vms/$VM/snapshots/$id"); [ "$(echo "$r" | code)" -lt 300 ] || { echo "delete: $(echo "$r" | body | head -c 200)"; return 1; }
  [ -z "$warn" ] && echo "created, listed, deleted" || echo "created, listed, deleted (config-only: no PVC-backed disk)"
}

# A containerdisk guest has no PVC, so this one gives the guest a real
# CSI-backed data disk (a pre-created blank DataVolume) and checks that the
# snapshot captures it: the API reports no config-only warning and a
# VolumeSnapshot becomes ready. Filesystem mode is explicit because CDI would
# otherwise pick Block on RBD, whose importer cannot open the device here.
s_data_snapshot() {
  [ -n "$KUBECTL" ] && [ -n "${E2E_STORAGE_CLASS:-}" ] || { echo "set KUBECTL and E2E_STORAGE_CLASS"; return 77; }
  local pvm="$VM-pvc" dv="$VM-pvc-data" r
  $KUBECTL -n "$NS" apply -f - >/dev/null <<DV || { echo "could not create DataVolume"; return 1; }
apiVersion: cdi.kubevirt.io/v1beta1
kind: DataVolume
metadata:
  name: $dv
  namespace: $NS
  annotations:
    cdi.kubevirt.io/storage.bind.immediate.requested: "true"
spec:
  source:
    blank: {}
  storage:
    storageClassName: $E2E_STORAGE_CLASS
    volumeMode: Filesystem
    accessModes: ["ReadWriteOnce"]
    resources:
      requests:
        storage: 1Gi
DV
  $KUBECTL -n "$NS" wait "dv/$dv" --for=condition=Ready --timeout=300s >/dev/null 2>&1 || { echo "DataVolume never Ready"; return 1; }
  local ga=""
  [ "$AGENT" = none ] || ga="\"guest_agent\":\"$AGENT\","
  r=$(api POST /vms "{\"name\":\"$pvm\",$ga\"image\":\"$IMAGE\",\"cpus\":2,\"memory\":2048,\"disks\":[
    {\"name\":\"root\",\"size\":\"10Gi\",\"boot_order\":1,\"source\":{\"type\":\"containerDisk\",\"image\":\"$IMAGE\"}},
    {\"name\":\"data\",\"size\":\"1Gi\",\"boot_order\":2,\"source\":{\"type\":\"dataVolume\",\"name\":\"$dv\"}}]}")
  case "$(echo "$r" | code)" in 2??|408) ;; *) echo "create: $(echo "$r" | body | head -c 200)"; return 1 ;; esac
  running() { api GET "/vms/$pvm" | body | jq -e '.state == "running"' >/dev/null 2>&1; }
  wait_for 600 running || { echo "PVC guest never running"; return 1; }
  r=$(api POST "/vms/$pvm/snapshots" '{"name":"d1"}'); [ "$(echo "$r" | code)" -lt 300 ] || { echo "snapshot: $(echo "$r" | body | head -c 200)"; return 1; }
  snap_ok() { [ "$($KUBECTL -n "$NS" get vmsnapshot "$pvm-d1" -o jsonpath='{.status.readyToUse}' 2>/dev/null)" = true ]; }
  wait_for 300 snap_ok || { echo "VirtualMachineSnapshot never ready"; return 1; }
  [ -z "$(api GET "/vms/$pvm/snapshots" | body | jq -r '.[0].warning // empty')" ] || { echo "API still warns config-only"; return 1; }
  local vs; vs=$($KUBECTL -n "$NS" get volumesnapshot --no-headers 2>/dev/null | grep -c "$dv")
  [ "${vs:-0}" -ge 1 ] || { echo "no VolumeSnapshot for $dv"; return 1; }
  echo "data snapshot ready: $vs VolumeSnapshot(s) on $E2E_STORAGE_CLASS"
}

# Off-cluster backup -> restore -> recovery drill of the PVC-backed guest.
# Needs the ZORVIA_BACKUP_* target configured on the deployment (E2E_BACKUP=1)
# and runs after s_data_snapshot, which leaves "$VM-pvc" running.
op_wait() { # operation-id seconds -> prints final operation JSON, exit 0 only if succeeded
  local id=$1 deadline=$((SECONDS+$2)) j st
  while [ $SECONDS -lt $deadline ]; do
    j=$(api GET "/operations/$id" | body); st=$(echo "$j" | jq -r '.state // empty')
    case "$st" in succeeded) echo "$j"; return 0 ;; failed|cancelled) echo "$j"; return 1 ;; esac
    sleep 10
  done
  echo "$j"; return 1
}

s_offcluster_backup() {
  [ "${E2E_BACKUP:-}" = 1 ] || { echo "set E2E_BACKUP=1 (needs a configured S3/Atlas target)"; return 77; }
  local pvm="$VM-pvc" r id j
  [ "$(api GET "/vms/$pvm" | code)" = 200 ] || { echo "needs the data-snapshot guest"; return 77; }
  r=$(api POST /backups/offcluster "{\"vm_name\":\"$pvm\"}")
  [ "$(echo "$r" | code)" -lt 300 ] || { echo "enqueue: $(echo "$r" | body | head -c 200)"; return 1; }
  id=$(echo "$r" | body | jq -r .operation_id); echo "$id" > "${TMPDIR:-/tmp}/e2e-backup-op-$VM"
  j=$(op_wait "$id" 1800) || { echo "backup failed: $(echo "$j" | jq -r '.error // .phase' | head -c 300)"; return 1; }
  echo "$j" | jq -e '.result.offcluster | .encrypted == true and .verified == true' >/dev/null || { echo "backup not encrypted+verified: $(echo "$j" | jq -c '.result.offcluster | {encrypted, verified}')"; return 1; }
  echo "$id: $(echo "$j" | jq -r '.result.offcluster | "encrypted, read-back verified, \(.disks | length) disk(s), \([.disks[].size_bytes] | add) bytes"')"
}

s_offcluster_restore() {
  [ "${E2E_BACKUP:-}" = 1 ] || { echo "set E2E_BACKUP=1"; return 77; }
  local f="${TMPDIR:-/tmp}/e2e-backup-op-$VM" id r j
  [ -s "$f" ] || { echo "no backup from the previous scenario"; return 77; }
  id=$(cat "$f")
  r=$(api POST "/backups/offcluster/$id/restore" "{\"new_vm_name\":\"$VM-restored\",\"storage_class\":\"${E2E_STORAGE_CLASS:-}\",\"start\":false}")
  [ "$(echo "$r" | code)" -lt 300 ] || { echo "restore: $(echo "$r" | body | head -c 200)"; return 1; }
  j=$(op_wait "$(echo "$r" | body | jq -r .operation_id)" 1800) || { echo "restore failed: $(echo "$j" | jq -r '.error // .phase' | head -c 300)"; return 1; }
  echo "$j" | jq -r '.result.volumes[]?' > "${TMPDIR:-/tmp}/e2e-restored-pvcs-$VM"   # for cleanup
  [ "$(api GET "/vms/$VM-restored" | code)" = 200 ] || { echo "restored VM not found"; return 1; }
  echo "restored into $VM-restored from backup $id"
}

s_recovery_drill() {
  [ "${E2E_BACKUP:-}" = 1 ] || { echo "set E2E_BACKUP=1"; return 77; }
  local f="${TMPDIR:-/tmp}/e2e-backup-op-$VM" id r j
  [ -s "$f" ] || { echo "no backup from the previous scenario"; return 77; }
  id=$(cat "$f")
  r=$(api POST "/backups/offcluster/$id/drill"); [ "$(echo "$r" | code)" -lt 300 ] || { echo "drill: $(echo "$r" | body | head -c 200)"; return 1; }
  j=$(op_wait "$(echo "$r" | body | jq -r .operation_id)" 2400) || { echo "drill failed: $(echo "$j" | jq -r '.error // .phase' | head -c 300)"; return 1; }
  echo "$j" | jq -e '.result.drill.booted == true' >/dev/null || { echo "drill did not boot the restored guest"; return 1; }
  echo "$(echo "$j" | jq -r '.result.drill | "booted (\(.evidence)), restore \(.restore_seconds)s, boot \(.boot_seconds)s, guest_agent=\(.guest_agent)"')"
}

s_live_migration() {
  local nodes
  if [ -n "$KUBECTL" ]; then
    nodes=$($KUBECTL get nodes --no-headers 2>/dev/null | grep -vc SchedulingDisabled)
    [ "${nodes:-1}" -ge 2 ] || { echo "single schedulable node"; return 77; }
  else
    echo "KUBECTL not set; cannot confirm a second node"; return 77
  fi
  local before after r
  before=$($KUBECTL -n "$NS" get vmi "$VM" -o jsonpath='{.status.nodeName}' 2>/dev/null)
  r=$(api POST "/vms/$VM/migrate"); [ "$(echo "$r" | code)" -lt 300 ] || { echo "migrate: $(echo "$r" | body | head -c 200)"; return 1; }
  moved() { after=$($KUBECTL -n "$NS" get vmi "$VM" -o jsonpath='{.status.nodeName}' 2>/dev/null); [ -n "$after" ] && [ "$after" != "$before" ]; }
  wait_for 600 moved || { echo "still on $before"; return 1; }
  # guest must still answer after the move (the load loop keeps it busy)
  agent_up || wait_for 300 agent_up || { echo "agent lost after migration"; return 1; }
  echo "$before -> $after under load"
}

s_api_restart() {
  [ -n "$KUBECTL" ] || { echo "KUBECTL not set"; return 77; }
  $KUBECTL -n zorvia-system rollout restart deploy/zorvia-api >/dev/null 2>&1 || { echo "restart failed"; return 1; }
  up() { curl -skf -m 5 "$BASE/api/v1/health" >/dev/null 2>&1; }
  # Wait for the rollout itself, otherwise the old pod can still answer health.
  $KUBECTL -n zorvia-system rollout status deploy/zorvia-api --timeout=600s >/dev/null 2>&1 || { echo "rollout did not finish"; return 1; }
  wait_for 600 up || { echo "API did not return"; return 1; }
  # old token must still be valid (token_version lives in the DB) and the guest untouched
  [ "$(api GET "/vms/$VM" | code)" = 200 ] || { echo "session or VM lost after restart"; return 1; }
  vm_running || { echo "VM not Running after API restart"; return 1; }
  echo "session kept, VM untouched"
}

echo "| scenario | result | time | detail |"; echo "|---|---|---|---|"
run "create + boot + guest ready + $AGENT agent ($IMAGE)" s_create_boot
run "snapshot create/delete" s_snapshot_revert
run "data snapshot on CSI storage (${E2E_STORAGE_CLASS:-unset})" s_data_snapshot
run "off-cluster backup, encrypted + read-back verified" s_offcluster_backup
run "restore from off-cluster backup" s_offcluster_restore
run "recovery drill (restore + boot proof + teardown)" s_recovery_drill
run "live migration under load" s_live_migration
run "API restart, guest untouched" s_api_restart
echo; echo "VM: $VM  host: $HOST  $(date -u +%FT%TZ)"
exit $((FAILS > 0))
