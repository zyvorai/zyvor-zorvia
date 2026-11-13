#!/usr/bin/env bash
# Honest OVE vs Zorvia worksheet helper. Never invents OpenShift list prices.
# Usage: ./scripts/ove-cost-worksheet.sh <nodes> <inr|usd> <supported|supported-plus> [ove_year1]
set -euo pipefail

usage() {
  echo "Usage: $0 <nodes> <inr|usd> <supported|supported-plus> [ove_year1_amount]" >&2
  exit 1
}

[[ $# -ge 3 ]] || usage
NODES="$1"
CUR="$(printf '%s' "$2" | tr '[:upper:]' '[:lower:]')"
TIER="$(printf '%s' "$3" | tr '[:upper:]' '[:lower:]')"
OVE="${4:-}"

[[ "$NODES" =~ ^[0-9]+$ ]] && [[ "$NODES" -gt 0 ]] || { echo "nodes must be a positive integer" >&2; exit 1; }

case "$CUR:$TIER" in
  inr:supported) PER=30000; MIN=150000; SYM=₹ ;;
  inr:supported-plus) PER=60000; MIN=400000; SYM=₹ ;;
  usd:supported) PER=600; MIN=3000; SYM=\$ ;;
  usd:supported-plus) PER=1200; MIN=8000; SYM=\$ ;;
  *) echo "currency must be inr|usd and tier supported|supported-plus" >&2; exit 1 ;;
esac

NODE_TOTAL=$((NODES * PER))
if [[ "$NODE_TOTAL" -gt "$MIN" ]]; then
  ZORVIA=$NODE_TOTAL
  HOW="nodes × rate"
else
  ZORVIA=$MIN
  HOW="annual minimum"
fi

echo "Zorvia ${TIER} (${CUR}), ${NODES} worker nodes"
echo "  per-node: ${SYM}${PER}"
echo "  nodes×rate: ${SYM}${NODE_TOTAL}"
echo "  annual minimum: ${SYM}${MIN}"
echo "  => annual support estimate: ${SYM}${ZORVIA}  (${HOW})"
echo "  (excludes taxes, Managed, Deployment/Migration one-time; see docs/PRICING.md)"

if [[ -n "$OVE" ]]; then
  [[ "$OVE" =~ ^[0-9]+$ ]] || { echo "ove_year1_amount must be an integer in the same currency units" >&2; exit 1; }
  DIFF=$((OVE - ZORVIA))
  echo
  echo "Your OVE year-1 quote (input): ${SYM}${OVE}"
  echo "Difference (your quote − Zorvia estimate): ${SYM}${DIFF}"
  echo "This is not a savings claim. Re-read the Red Hat product page on the quote date;"
  echo "align scope with docs/OPENSHIFT_COMPARISON.md and docs/OVE_COST_WORKSHEET.md."
fi
