#!/usr/bin/env bash
# Lab-only: deploy an in-cluster S3-compatible store (adobe/s3mock — pulls on
# restricted networks where MinIO/Quay do not), create credential Secrets, and
# print ZORVIA_BACKUP_S3_* env for the API Deployment.
#
# Override image with ZORVIA_LAB_S3_IMAGE if you can pull MinIO.
set -euo pipefail

NS="${ZORVIA_MINIO_NS:-zorvia-lab-minio}"
BUCKET="${ZORVIA_BACKUP_S3_BUCKET:-zorvia-backups}"
SECRET_NAME="${ZORVIA_BACKUP_S3_SECRET:-backup-s3}"
ACCESS_KEY="${ZORVIA_MINIO_ACCESS_KEY:-zorviaminio}"
SECRET_KEY="${ZORVIA_MINIO_SECRET_KEY:-zorviaminio-secret-change-me}"
COPY_NS="${ZORVIA_BACKUP_COPY_NS:-default zorvia-dr}"
# S3Mock listens on 9090; MinIO on 9000. Detect from image name.
IMAGE="${ZORVIA_LAB_S3_IMAGE:-docker.io/adobe/s3mock:3.1.0}"

KUBECTL="${KUBECTL:-kubectl}"
if [[ -z "${KUBECONFIG:-}" && -f /etc/rancher/k3s/k3s.yaml ]]; then
  KUBECTL="sudo -n env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl"
fi

PORT=9090
READY_PATH="/"
if [[ "$IMAGE" == *minio* ]]; then
  PORT=9000
  READY_PATH="/minio/health/ready"
fi

echo "==> Namespace $NS"
$KUBECTL create ns "$NS" --dry-run=client -o yaml | $KUBECTL apply -f -

echo "==> S3-compatible Deployment + Service ($IMAGE)"
$KUBECTL -n "$NS" apply -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: minio
  labels: { app: minio }
spec:
  replicas: 1
  selector:
    matchLabels: { app: minio }
  template:
    metadata:
      labels: { app: minio }
    spec:
      containers:
        - name: minio
          image: ${IMAGE}
          ports:
            - containerPort: ${PORT}
              name: s3
          env:
            - name: initialBuckets
              value: "${BUCKET}"
            - name: COM_ADOBE_TESTING_S3MOCK_STORE_REGION
              value: us-east-1
          readinessProbe:
            tcpSocket:
              port: ${PORT}
            initialDelaySeconds: 3
            periodSeconds: 3
---
apiVersion: v1
kind: Service
metadata:
  name: minio
spec:
  selector: { app: minio }
  ports:
    - name: s3
      port: ${PORT}
      targetPort: ${PORT}
EOF

echo "==> Wait for Ready"
$KUBECTL -n "$NS" rollout status deploy/minio --timeout=180s

ENDPOINT="http://minio.${NS}.svc:${PORT}"
echo "==> Credential Secrets in: $COPY_NS"
for n in $COPY_NS; do
  $KUBECTL create ns "$n" --dry-run=client -o yaml | $KUBECTL apply -f -
  $KUBECTL -n "$n" create secret generic "$SECRET_NAME" \
    --from-literal=access-key="$ACCESS_KEY" \
    --from-literal=secret-key="$SECRET_KEY" \
    --dry-run=client -o yaml | $KUBECTL apply -f -
done

cat <<EOF

Wire these onto the zorvia-api Deployment (and restart):

  ZORVIA_BACKUP_S3_ENDPOINT=${ENDPOINT}
  ZORVIA_BACKUP_S3_BUCKET=${BUCKET}
  ZORVIA_BACKUP_S3_SECRET=${SECRET_NAME}
  ZORVIA_BACKUP_S3_REGION=us-east-1
  ZORVIA_BACKUP_AGENT_IMAGE=\${ZORVIA_BACKUP_AGENT_IMAGE:-ghcr.io/zyvorai/zorvia-backup-agent:latest}

Then create a PVC-backed stopped VM (needs a CSI VolumeSnapshotClass) and:

  POST /api/v1/enterprise/cross-cluster-dr/run
  { "source_cluster": "lab", "target_cluster": "lab", "vm_names": ["<vm>"], "rpo_seconds": 300 }

Poll backup ops, then call /run again with backup_operation_ids and
restore_namespace=zorvia-dr.

Note: adobe/s3mock is in-memory lab only; use real MinIO/RGW for durable backups.
EOF
