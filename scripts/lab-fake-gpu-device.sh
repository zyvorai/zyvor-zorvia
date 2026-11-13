#!/usr/bin/env bash
# Lab-only: label a node, deploy the fake GPU device-plugin, and print the
# correct permit + VM-create payloads (devices.managePermitted must be on).
set -euo pipefail

RESOURCE="${ZORVIA_FAKE_GPU_RESOURCE:-lab.zorvia.io/fake-gpu}"
COUNT="${ZORVIA_FAKE_GPU_COUNT:-1}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

KUBECTL="${KUBECTL:-kubectl}"
if [[ -z "${KUBECONFIG:-}" && -f /etc/rancher/k3s/k3s.yaml ]]; then
  KUBECTL="sudo -n env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl"
fi

NODE="${ZORVIA_FAKE_GPU_NODE:-}"
if [[ -z "$NODE" ]]; then
  NODE=$($KUBECTL get nodes -o jsonpath='{.items[0].metadata.name}')
fi

echo "==> Label node $NODE zorvia.io/fake-gpu=true"
$KUBECTL label node "$NODE" "zorvia.io/fake-gpu=true" --overwrite

echo "==> Ensure host char device /var/lib/zorvia-lab-devices/fake-gpu"
if [[ "$(id -u)" -eq 0 ]]; then
  mkdir -p /var/lib/zorvia-lab-devices
  [[ -c /var/lib/zorvia-lab-devices/fake-gpu ]] || mknod -m 666 /var/lib/zorvia-lab-devices/fake-gpu c 1 5
else
  sudo -n mkdir -p /var/lib/zorvia-lab-devices
  sudo -n sh -c 'test -c /var/lib/zorvia-lab-devices/fake-gpu || mknod -m 666 /var/lib/zorvia-lab-devices/fake-gpu c 1 5'
fi

echo "==> Apply fake GPU device-plugin DaemonSet"
$KUBECTL apply -f "$ROOT/deploy/lab-fake-gpu-device-plugin.yaml"

echo "==> Wait for plugin Ready"
$KUBECTL -n zorvia-lab-devices rollout status ds/fake-gpu-device-plugin --timeout=120s

echo "==> Check allocatable"
for i in $(seq 1 30); do
  alloc=$($KUBECTL get node "$NODE" -o jsonpath="{.status.allocatable['${RESOURCE}']}" 2>/dev/null || true)
  if [[ -n "$alloc" && "$alloc" != "0" ]]; then
    echo "    allocatable[$RESOURCE]=$alloc"
    break
  fi
  sleep 2
done
if [[ -z "${alloc:-}" || "$alloc" == "0" ]]; then
  echo "WARN: allocatable[$RESOURCE] still missing; check plugin logs:" >&2
  echo "  $KUBECTL -n zorvia-lab-devices logs -l app.kubernetes.io/name=fake-gpu-device-plugin --tail=50" >&2
fi

cat <<EOF

Permit (cluster.admin; Helm devices.managePermitted=true):

  POST /api/v1/devices/permitted
  {
    "kind": "pci",
    "resource_name": "${RESOURCE}",
    "pci_vendor_selector": "10DE:1EB8",
    "external_resource_provider": true
  }

Preflight / create:

  POST /api/v1/devices/preflight
  { "devices": [{ "name": "gpu0", "device_name": "${RESOURCE}", "kind": "gpu" }] }

  POST /api/vms
  {
    "name": "lab-fake-gpu",
    "namespace": "default",
    "cpu": 1,
    "memory": "256Mi",
    "devices": [{ "name": "gpu0", "device_name": "${RESOURCE}", "kind": "gpu" }]
  }

Expect capacity ${COUNT} once the device plugin registers.
EOF
