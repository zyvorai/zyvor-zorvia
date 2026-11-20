#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

# Lab smoke against a running Zorvia API (default NodePort lab).
set -euo pipefail
HOST="${ZORVIA_LAB_HOST:-175.110.122.71}"
PORT="${ZORVIA_LAB_PORT:-30152}"
BASE="https://${HOST}:${PORT}"
USER="${ZORVIA_E2E_USER:-admin}"
PASS="${ZORVIA_E2E_PASSWORD:?set ZORVIA_E2E_PASSWORD}"

echo "== health =="
curl -skf "${BASE}/api/v1/health" | head -c 200
echo
echo "== features (audit-trail) =="
curl -skf "${BASE}/api/v1/features" | jq -r '.features[]? // .data.features[]? | select(.id=="audit-trail") | [.id,.maturity // .level] | @tsv'
echo "== login =="
TOKEN=$(curl -skf -X POST "${BASE}/api/v1/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"${USER}\",\"password\":\"${PASS}\"}" | jq -r .token)
test -n "$TOKEN" && test "$TOKEN" != null
echo "== audit export =="
curl -skf -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/audit/export?limit=5" | head -c 400
echo
echo "OK lab smoke ${BASE}"
