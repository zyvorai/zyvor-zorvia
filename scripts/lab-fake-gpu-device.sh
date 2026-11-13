#!/usr/bin/env bash
# Lab-only: advertise a fake extended resource so GPU preflight/permit can be
# exercised without NVIDIA hardware. Does not install a real device plugin.
set -euo pipefail

RESOURCE="${ZORVIA_FAKE_GPU_RESOURCE:-lab.zorvia.io/fake-gpu}"
COUNT="${ZORVIA_FAKE_GPU_COUNT:-1}"

KUBECTL="${KUBECTL:-kubectl}"
if [[ -z "${KUBECONFIG:-}" && -r /etc/rancher/k3s/k3s.yaml ]]; then
  export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
  KUBECTL="sudo -n KUBECONFIG=$KUBECONFIG kubectl"
fi

NODE="${ZORVIA_FAKE_GPU_NODE:-}"
if [[ -z "$NODE" ]]; then
  NODE=$($KUBECTL get nodes -o jsonpath='{.items[0].metadata.name}')
fi

echo "==> Patching node $NODE allocatable/capacity with $RESOURCE=$COUNT"
# status.allocatable is owned by the kubelet; for lab demos we also label the
# node so operators can find it, and print the permit API payload to use after
# enabling Helm devices.managePermitted=true.
$KUBECTL label node "$NODE" "zorvia.io/fake-gpu=true" --overwrite

cat <<EOF

Advertise the resource with a tiny device-plugin DaemonSet of your choice, or
for inventory-only checks call:

  POST /api/v1/devices/permitted
  { "name": "fake-gpu", "deviceName": "${RESOURCE}", "kind": "gpu" }

  POST /api/v1/enterprise/placement/gpu-numa
  { "vm_name": "lab-gpu", "gpu_count": 1, "sriov_networks": [], "numa_passthrough": false }

Expect preflight_issues to mention missing capacity until a device plugin
actually sets allocatable[${RESOURCE}]=${COUNT}.

Node labeled: zorvia.io/fake-gpu=true
EOF
