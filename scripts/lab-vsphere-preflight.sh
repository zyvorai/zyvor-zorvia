#!/usr/bin/env bash
# Lab/operator checklist before a real vCenter vm-import (mode=vsphere).
# Does not contact vCenter unless GOVC_* / VCENTER_* tools are present.
set -euo pipefail

fail=0
warn() { echo "WARN: $*" >&2; }
die() { echo "FAIL: $*" >&2; fail=1; }
ok() { echo "OK: $*"; }

echo "==> Zorvia experimental gates"
[[ "${ZORVIA_EXPERIMENTAL:-}" == "1" ]] || die "ZORVIA_EXPERIMENTAL=1 required"
[[ -n "${ZORVIA_H2KVM_IMAGE:-}" ]] || die "ZORVIA_H2KVM_IMAGE must point at a licensed h2kvm CLI image"
ok "experimental + h2kvm image env"

echo "==> Target cluster bits"
KUBECTL="${KUBECTL:-kubectl}"
if [[ -z "${KUBECONFIG:-}" && -f /etc/rancher/k3s/k3s.yaml ]]; then
  KUBECTL="sudo -n env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl"
fi
$KUBECTL get crd virtualmachines.kubevirt.io >/dev/null 2>&1 || die "KubeVirt CRDs missing"
$KUBECTL get crd datavolumes.cdi.kubevirt.io >/dev/null 2>&1 || die "CDI CRDs missing"
ok "KubeVirt + CDI CRDs"

NS="${ZORVIA_IMPORT_NS:-default}"
echo "==> Namespace $NS (privileged PSA + Secret + SA)"
if $KUBECTL get ns "$NS" >/dev/null 2>&1; then
  PSA=$($KUBECTL get ns "$NS" -o jsonpath='{.metadata.labels.pod-security\.kubernetes\.io/enforce}' 2>/dev/null || true)
  [[ "$PSA" == "privileged" ]] || warn "label pod-security.kubernetes.io/enforce=privileged recommended for h2kvm Jobs"
else
  warn "namespace $NS missing"
fi
SECRET="${ZORVIA_VC_SECRET:-vcenter-creds}"
if $KUBECTL -n "$NS" get secret "$SECRET" >/dev/null 2>&1; then
  ok "Secret $SECRET present"
else
  die "create Secret $SECRET with username/password keys (see deploy/h2kvm-import-rbac.yaml)"
fi
if $KUBECTL -n "$NS" get sa zorvia-h2kvm >/dev/null 2>&1; then
  ok "ServiceAccount zorvia-h2kvm"
else
  die "apply deploy/h2kvm-import-rbac.yaml for zorvia-h2kvm SA/Role"
fi

echo "==> Optional govc probe"
if command -v govc >/dev/null 2>&1 && [[ -n "${GOVC_URL:-}${VCENTER_URL:-}" ]]; then
  if govc about 2>/dev/null | head -3; then
    ok "govc reached vCenter"
  else
    die "govc about failed — check GOVC_URL / credentials"
  fi
else
  warn "no govc/GOVC_URL — skipping live vCenter reachability (hard block for mode=vsphere until you have a reachable vCenter)"
fi

if [[ "$fail" -ne 0 ]]; then
  echo
  echo "Real vCenter import is blocked until the FAIL items above are fixed."
  echo "Lab substitutes: mode=stub (Job only) or mode=local (stopped emptyDisk VM)."
  exit 1
fi
echo
echo "Preflight passed. POST /api/vm-imports with mode=vsphere when ready."
