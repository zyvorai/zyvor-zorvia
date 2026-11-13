#!/usr/bin/env bash
# Lab-only: enroll the same API twice so fleet remote probing is exercised
# without a second cluster. Do not use in production.
set -euo pipefail

NS="${ZORVIA_FLEET_NS:-default}"
OUT="${ZORVIA_FLEET_OUT:-/tmp/zorvia-fleet-lab}"
SA="zorvia-fleet-lab-alias"
ALIAS_NAME="${ZORVIA_FLEET_ALIAS:-lab-alias}"

KUBECTL="${KUBECTL:-kubectl}"
if [[ -z "${KUBECONFIG:-}" && -r /etc/rancher/k3s/k3s.yaml ]]; then
  export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
  KUBECTL="sudo -n KUBECONFIG=$KUBECONFIG kubectl"
fi

mkdir -p "$OUT"
echo "==> ServiceAccount + read-only RBAC in $NS"
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
  name: zorvia-fleet-lab-alias-nodes
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
  name: zorvia-fleet-lab-alias-vms
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

echo "==> Mint SA token kubeconfig"
TOKEN=$($KUBECTL -n "$NS" create token "$SA" --duration=24h)
SERVER=$($KUBECTL config view --minify -o jsonpath='{.clusters[0].cluster.server}')
CA=$($KUBECTL config view --raw --minify -o jsonpath='{.clusters[0].cluster.certificate-authority-data}')
if [[ -z "$CA" && -r /var/lib/rancher/k3s/server/tls/server-ca.crt ]]; then
  CA=$(sudo -n base64 -w0 /var/lib/rancher/k3s/server/tls/server-ca.crt 2>/dev/null || true)
fi
cat >"$OUT/lab-alias.yaml" <<EOF
apiVersion: v1
kind: Config
clusters:
- name: lab
  cluster:
    server: ${SERVER}
    certificate-authority-data: ${CA}
users:
- name: $SA
  user:
    token: ${TOKEN}
contexts:
- name: lab-readonly
  context:
    cluster: lab
    user: $SA
current-context: lab-readonly
EOF

cat >"$OUT/registry.json" <<EOF
[
  {
    "name": "${ALIAS_NAME}",
    "kubeconfig": "/etc/zorvia/fleet/lab-alias.yaml",
    "context": "lab-readonly",
    "namespaces": ["${NS}"],
    "environment": "lab",
    "region": "local"
  }
]
EOF

echo "==> Apply Secret + ConfigMap (mount these into the Zorvia pod)"
$KUBECTL -n "$NS" create configmap zorvia-fleet-registry \
  --from-file=registry.json="$OUT/registry.json" \
  -o yaml --dry-run=client | $KUBECTL apply -f -
$KUBECTL -n "$NS" create secret generic zorvia-fleet-lab-alias \
  --from-file=lab-alias.yaml="$OUT/lab-alias.yaml" \
  -o yaml --dry-run=client | $KUBECTL apply -f -

cat <<EOF

Next: mount into the zorvia Deployment (example):

  volumeMounts:
    - name: fleet-registry
      mountPath: /etc/zorvia/fleet/registry.json
      subPath: registry.json
      readOnly: true
    - name: fleet-alias
      mountPath: /etc/zorvia/fleet/lab-alias.yaml
      subPath: lab-alias.yaml
      readOnly: true
  volumes:
    - name: fleet-registry
      configMap:
        name: zorvia-fleet-registry
    - name: fleet-alias
      secret:
        secretName: zorvia-fleet-lab-alias
  env:
    - name: ZORVIA_EXPERIMENTAL
      value: "1"
    - name: ZORVIA_FEATURE_FLEET
      value: "1"
    - name: ZORVIA_FLEET_CONFIG
      value: /etc/zorvia/fleet/registry.json

Artifacts: $OUT
EOF
