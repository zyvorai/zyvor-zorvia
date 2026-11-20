#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

# Create the zorvia-auth Secret with random credentials (never commit these).
set -euo pipefail

NS="${ZORVIA_NAMESPACE:-zorvia-system}"
SECRET_NAME="${ZORVIA_AUTH_SECRET:-zorvia-auth}"
ADMIN_USER="${ZORVIA_ADMIN_USER:-admin}"

if ! command -v kubectl >/dev/null 2>&1; then
  echo "kubectl is required" >&2
  exit 1
fi

kubectl get ns "$NS" >/dev/null 2>&1 || kubectl create namespace "$NS"

ADMIN_PASSWORD="$(openssl rand -base64 24 | tr -d '/+=' | head -c 24)"
JWT_SECRET="$(openssl rand -hex 32)"

kubectl -n "$NS" create secret generic "$SECRET_NAME" \
  --from-literal=admin-user="$ADMIN_USER" \
  --from-literal=admin-password="$ADMIN_PASSWORD" \
  --from-literal=jwt-secret="$JWT_SECRET" \
  --dry-run=client -o yaml | kubectl apply -f -

cat <<EOF

Created Secret ${NS}/${SECRET_NAME}

  admin user:     ${ADMIN_USER}
  admin password: ${ADMIN_PASSWORD}
  jwt secret:     (stored in Secret; not printed)

Store the password securely and rotate after first login.
For lab-only shared API keys, add --from-literal=api-key=... and set ZORVIA_LAB_MODE=1.

EOF
