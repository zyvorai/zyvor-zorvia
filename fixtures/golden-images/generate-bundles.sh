#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

# Generate CDI golden-image bundles for major Linux quay.io/containerdisks.
#
# Usage:
#   STORAGE_CLASS=fast ./fixtures/golden-images/generate-bundles.sh
#   STORAGE_CLASS=fast NAMESPACE=vm-images SIZE=40Gi OUT_DIR=./out ./fixtures/golden-images/generate-bundles.sh
#
# Requires: zorvia on PATH (or set ZORVIA=/path/to/zorvia).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ZORVIA="${ZORVIA:-zorvia}"
NAMESPACE="${NAMESPACE:-vm-images}"
STORAGE_CLASS="${STORAGE_CLASS:-}"
SIZE="${SIZE:-40Gi}"
OUT_DIR="${OUT_DIR:-${ROOT}/fixtures/golden-images/out}"

if [[ -z "${STORAGE_CLASS}" ]]; then
  echo "error: set STORAGE_CLASS to a cluster StorageClass name" >&2
  echo "  STORAGE_CLASS=fast $0" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

bundle() {
  local name="$1"
  local version="$2"
  local image="$3"
  local size="${4:-$SIZE}"
  local out="${OUT_DIR}/${name}.yaml"

  echo "Generating ${name} (${image}) -> ${out}"
  "${ZORVIA}" --namespace "${NAMESPACE}" image-bundle "${name}" \
    --version "${version}" \
    --source "docker://${image}" \
    --source-type registry \
    --size "${size}" \
    --storage-class "${STORAGE_CLASS}" \
    --output "${out}"
}

# Latest stable / LTS tags per family (see docs/GOLDEN_IMAGES.md).
bundle ubuntu-golden        26.04-r1  quay.io/containerdisks/ubuntu:26.04
bundle fedora-golden        44-r1     quay.io/containerdisks/fedora:44
bundle debian-golden        13-r1     quay.io/containerdisks/debian:13
bundle almalinux-golden     10-r1     quay.io/containerdisks/almalinux:10
bundle centos-stream-golden 10-r1     quay.io/containerdisks/centos-stream:10
bundle opensuse-golden      16.0-r1   quay.io/containerdisks/opensuse-leap:16.0
# No rockylinux/alpine: those repositories do not exist on quay.io/containerdisks.

cat > "${OUT_DIR}/README.md" <<EOF
# Generated golden image bundles

Namespace: \`${NAMESPACE}\`
StorageClass: \`${STORAGE_CLASS}\`

Apply after CDI is installed:

\`\`\`bash
kubectl create ns ${NAMESPACE} --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f ${OUT_DIR}/
\`\`\`

Wait for each DataVolume to Succeed before relying on the stable DataSource alias.
EOF

echo "Done. Manifests in ${OUT_DIR}/"
