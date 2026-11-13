#!/usr/bin/env bash
# Lab-only: stand up a *second* Kubernetes API (k3d preferred, kind fallback),
# mint a read-only kubeconfig, and print a fleet registry entry that is not the
# same API as `local`. Requires docker/k3d (or kind) on the host.
#
# Usage:
#   ./scripts/lab-fleet-second-cluster.sh
#   # then merge the printed registry into ZORVIA_FLEET_CONFIG and remount Secrets
set -euo pipefail

NAME="${ZORVIA_FLEET_SECOND_NAME:-fleet-b}"
NS="${ZORVIA_FLEET_NS:-default}"
OUT="${ZORVIA_FLEET_OUT:-/tmp/zorvia-fleet-second}"
SA="zorvia-fleet-second"

mkdir -p "$OUT"

have() { command -v "$1" >/dev/null 2>&1; }

# k3d/kind talk to the docker socket; use sudo when the user is not in the docker group.
docker_ok() { docker info >/dev/null 2>&1; }
if ! docker_ok && sudo -n docker info >/dev/null 2>&1; then
  export DOCKER_HOST="${DOCKER_HOST:-unix:///var/run/docker.sock}"
  K3D_PREFIX=(sudo -n)
  # sudo preserves DOCKER_HOST only if we pass env; wrap helpers.
  k3d_cmd() { sudo -n env "PATH=$PATH" k3d "$@"; }
  kind_cmd() { sudo -n env "PATH=$PATH" kind "$@"; }
else
  k3d_cmd() { k3d "$@"; }
  kind_cmd() { kind "$@"; }
fi

echo "==> Create second cluster ($NAME)"
if have k3d; then
  if ! k3d_cmd cluster list 2>/dev/null | grep -qw "$NAME"; then
    k3d_cmd cluster create "$NAME" --agents 0 --servers 1 --wait
  else
    echo "    k3d cluster $NAME already exists"
  fi
  k3d_cmd kubeconfig get "$NAME" >"$OUT/kubeconfig-admin.yaml"
  KUBECTL="kubectl --kubeconfig=$OUT/kubeconfig-admin.yaml"
elif have kind; then
  if ! kind_cmd get clusters 2>/dev/null | grep -qx "$NAME"; then
    kind_cmd create cluster --name "$NAME"
  fi
  kind_cmd get kubeconfig --name "$NAME" >"$OUT/kubeconfig-admin.yaml"
  KUBECTL="kubectl --kubeconfig=$OUT/kubeconfig-admin.yaml"
else
  cat >&2 <<EOF
Neither k3d nor kind is installed. Install one, then re-run:
  curl -s https://raw.githubusercontent.com/k3d-io/k3d/main/install.sh | bash
EOF
  exit 1
fi

echo "==> Namespace + read-only SA on second cluster"
$KUBECTL create ns "$NS" --dry-run=client -o yaml | $KUBECTL apply -f -
$KUBECTL -n "$NS" apply -f - <<EOF
apiVersion: v1
kind: ServiceAccount
metadata:
  name: $SA
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: zorvia-fleet-nodes-reader
rules:
  - apiGroups: [""]
    resources: ["nodes"]
    verbs: ["list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: zorvia-fleet-second-nodes
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: zorvia-fleet-nodes-reader
subjects:
  - kind: ServiceAccount
    name: $SA
    namespace: $NS
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: zorvia-fleet-vms-reader
  namespace: $NS
rules:
  - apiGroups: ["kubevirt.io"]
    resources: ["virtualmachines"]
    verbs: ["list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: zorvia-fleet-second-vms
  namespace: $NS
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: zorvia-fleet-vms-reader
subjects:
  - kind: ServiceAccount
    name: $SA
    namespace: $NS
EOF

TOKEN=$($KUBECTL -n "$NS" create token "$SA" --duration=24h)
SERVER=$($KUBECTL config view --raw --minify -o jsonpath='{.clusters[0].cluster.server}')
CA=$($KUBECTL config view --raw --minify -o jsonpath='{.clusters[0].cluster.certificate-authority-data}')
CTX="${NAME}-readonly"

cat >"$OUT/${NAME}.yaml" <<EOF
apiVersion: v1
kind: Config
clusters:
  - name: $NAME
    cluster:
      server: $SERVER
      certificate-authority-data: $CA
users:
  - name: $SA
    user:
      token: $TOKEN
contexts:
  - name: $CTX
    context:
      cluster: $NAME
      user: $SA
current-context: $CTX
EOF

cat >"$OUT/registry-fragment.json" <<EOF
{
  "name": "$NAME",
  "kubeconfig": "/etc/zorvia/fleet/${NAME}.yaml",
  "context": "$CTX",
  "namespaces": ["$NS"],
  "environment": "lab",
  "region": "lab-b"
}
EOF

cat <<EOF

Second cluster ready (distinct API from the host's primary k3s/kind).

  kubeconfig: $OUT/${NAME}.yaml
  registry fragment: $OUT/registry-fragment.json

Mount ${NAME}.yaml as a Secret into the Zorvia API pod at
/etc/zorvia/fleet/${NAME}.yaml and add the fragment to registry.json
alongside any existing enrollments (not a duplicate of the same server).

  GET /api/v1/enterprise/fleet should list local + $NAME as separate clusters.

KubeVirt is optional on the second cluster: VM list may be empty; nodes must list.
EOF
