#!/bin/bash
# ============================================================================
# remote-deploy.sh — Full zorvia deployment to a remote server
# ============================================================================
# Rsync source to remote, compile there, install binary + service, verify.
#
# Usage:
#   ./deploy/remote-deploy.sh <host> [user] [password]
#   ./deploy/remote-deploy.sh 185.165.240.5 sus
#   ./deploy/remote-deploy.sh 185.165.240.5 sus mypassword
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --quick    # skip deps
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --uninstall
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --image      # no build: pull the public GHCR image
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --bootstrap --image   # brand-new host, zero to running
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --image=ghcr.io/zyvorai/zorvia:0.6.0
#   ./deploy/remote-deploy.sh 185.165.240.5 sus --bootstrap  # also install k3s+Cilium+KubeVirt+CDI+Envoy Gateway
#
# Environment variables:
#   DEPLOY_HOST=185.165.240.5
#   DEPLOY_USER=sus
#   DEPLOY_PASS=mypassword
#   DEPLOY_DIR=/home/sus/zorvia  (auto-detected from login user)
# ============================================================================

set -euo pipefail
trap 'echo "  [ERR] failed at line $LINENO: $BASH_COMMAND" >&2' ERR

info()  { echo "  [ok] $*"; }
warn()  { echo "  [!!] $*"; }
error() { echo "  [ERR] $*"; exit 1; }
step()  { echo ""; echo "  --- $*"; }

# ── Parse args ──
QUICK_MODE=false
UNINSTALL_MODE=false
BOOTSTRAP_MODE=false
IMAGE_MODE=false
IMAGE_REF="${ZORVIA_IMAGE:-ghcr.io/zyvorai/zorvia:latest}"
POSITIONAL=()
for arg in "$@"; do
    case "$arg" in
        --quick)     QUICK_MODE=true ;;
        --uninstall) UNINSTALL_MODE=true ;;
        --bootstrap) BOOTSTRAP_MODE=true ;;
        --image)     IMAGE_MODE=true ;;
        --image=*)   IMAGE_MODE=true; IMAGE_REF="${arg#--image=}" ;;
        --help|-h)
            echo "Usage: $0 <host> [user] [password] [--quick|--uninstall]"
            echo ""
            echo "  --quick      Skip Rust toolchain install (rsync + build only)"
            echo "  --uninstall  Remove zorvia from remote server"
            echo "  --image[=REF] Deploy a prebuilt image (default ghcr.io/zyvorai/zorvia:latest) instead of"
            echo "               building on the host: no rsync of source, no Rust toolchain, takes ~1 minute."
            echo "  --bootstrap  Install missing cluster prerequisites first: k3s server (no flannel),"
            echo "               Cilium, KubeVirt, CDI, Envoy Gateway + GatewayClass 'eg'. Refuses to run"
            echo "               on a host that is a k3s agent (set BOOTSTRAP_FORCE=1 to wipe it)."
            echo ""
            echo "Full mode installs: Rust toolchain, builds from source,"
            echo "deploys binary + systemd service + k3s manifest."
            exit 0
            ;;
        *)  POSITIONAL+=("$arg") ;;
    esac
done

HOST="${POSITIONAL[0]:-${DEPLOY_HOST:-}}"
USER="${POSITIONAL[1]:-${DEPLOY_USER:-sus}}"
PASS="${POSITIONAL[2]:-${DEPLOY_PASS:-}}"

[ -z "$HOST" ] && error "Usage: $0 <host> [user] [password] [--quick]"

# Determine remote home directory based on login user
if [ "$USER" = "root" ]; then
    REMOTE_HOME="/root"
else
    REMOTE_HOME="/home/${USER}"
fi
REMOTE_DIR="${DEPLOY_DIR:-${REMOTE_HOME}/zorvia}"

# Use sudo when not deploying as root
SUDO=""
[ "$USER" != "root" ] && SUDO="sudo"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

[ -f "$REPO_DIR/Cargo.toml" ] || error "Not in zorvia repo: $REPO_DIR"

# Keepalives stop long quiet steps (cargo build) from being dropped by NAT/idle timeouts.
SSH_OPTS=(-o StrictHostKeyChecking=no -o ServerAliveInterval=30 -o ServerAliveCountMax=40)

# ── SSH/rsync wrappers ──
_ssh() {
    if [ -n "$PASS" ]; then
        SSHPASS="$PASS" sshpass -e ssh "${SSH_OPTS[@]}" "${USER}@${HOST}" "$@"
    else
        # shellcheck disable=SC2029  # intentional: arguments expand on the client
        ssh "${SSH_OPTS[@]}" "${USER}@${HOST}" "$@"
    fi
}

_scp() {
    if [ -n "$PASS" ]; then
        SSHPASS="$PASS" sshpass -e scp "${SSH_OPTS[@]}" "$@"
    else
        scp "${SSH_OPTS[@]}" "$@"
    fi
}

_rsync() {
    local ssh_cmd="ssh ${SSH_OPTS[*]}"
    if [ -n "$PASS" ]; then
        ssh_cmd="sshpass -e $ssh_cmd"
    fi
    SSHPASS="$PASS" rsync -avz \
        --exclude='target/' \
        --exclude='.git' \
        --exclude='web/node_modules/' \
        --exclude='web/coverage/' \
        --exclude='*.qcow2' --exclude='*.vmdk' --exclude='*.raw' \
        --exclude='*.iso' --exclude='*.img' \
        -e "$ssh_cmd" \
        "$@"
}

# ── Preflight ──
if [ -n "$PASS" ] && ! command -v sshpass &>/dev/null; then
    error "sshpass required for password auth. Install: dnf install sshpass"
fi

# ── Uninstall mode ──
if $UNINSTALL_MODE; then
    echo ""
    echo "  ============================================"
    echo "    zorvia Remote Uninstall"
    echo "  ============================================"
    echo ""
    echo "  Host: ${USER}@${HOST}"
    echo ""

    step "Uninstalling zorvia"
    _ssh "
        $SUDO systemctl stop zorvia-web.service 2>/dev/null || true
        $SUDO systemctl disable zorvia-web.service 2>/dev/null || true
        $SUDO rm -f /usr/local/bin/zorvia
        $SUDO rm -f /etc/systemd/system/zorvia-web.service
        $SUDO systemctl daemon-reload
        rm -rf $REMOTE_DIR
        echo 'Done'
    " 2>&1
    info "zorvia removed from ${HOST}"
    echo ""
    exit 0
fi

# ── Bootstrap cluster prerequisites (opt-in) ──
if $BOOTSTRAP_MODE; then
    step "Bootstrap: k3s + Cilium + KubeVirt + CDI + Envoy Gateway"
    _ssh "SUDO='$SUDO' BOOTSTRAP_FORCE='${BOOTSTRAP_FORCE:-}' bash -s" <<'REMOTE' 2>&1
set -euo pipefail
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
k() { $SUDO -E kubectl "$@"; }
latest_tag() { curl -sIL -o /dev/null -w '%{url_effective}' "$1" | sed 's#.*/##'; }

# 1. k3s server without flannel (Cilium is the CNI)
if [ -f /etc/rancher/k3s/k3s.yaml ] && $SUDO systemctl is-active --quiet k3s; then
    echo "k3s: server already running (kept)"
else
    if systemctl list-unit-files 2>/dev/null | grep -q '^k3s-agent.service'; then
        if [ "${BOOTSTRAP_FORCE:-}" != "1" ]; then
            echo "ERROR: this host is a k3s AGENT of another cluster ($(grep -h K3S_URL /etc/systemd/system/k3s-agent.service.env 2>/dev/null))." >&2
            echo "       Re-run with BOOTSTRAP_FORCE=1 to uninstall the agent and make it a server." >&2
            exit 1
        fi
        echo "k3s: removing agent (BOOTSTRAP_FORCE=1)"
        $SUDO /usr/local/bin/k3s-agent-uninstall.sh </dev/null >/dev/null 2>&1 || true
        for l in cilium_vxlan cilium_host cilium_net; do $SUDO ip link del "$l" 2>/dev/null || true; done
    fi
    curl -sfL https://get.k3s.io | $SUDO INSTALL_K3S_EXEC="server --flannel-backend=none --disable-network-policy --disable=traefik" sh - >/dev/null
    for _ in $(seq 1 30); do k get nodes >/dev/null 2>&1 && break; sleep 2; done
    echo "k3s: server installed"
fi

# 2. Cilium
if k -n kube-system get ds cilium >/dev/null 2>&1; then
    echo "cilium: already installed (kept)"
else
    CLI=$(curl -sL https://raw.githubusercontent.com/cilium/cilium-cli/main/stable.txt)
    curl -sL --fail -o /tmp/cilium.tgz "https://github.com/cilium/cilium-cli/releases/download/$CLI/cilium-linux-amd64.tar.gz"
    $SUDO tar -xzf /tmp/cilium.tgz -C /usr/local/bin cilium && rm -f /tmp/cilium.tgz
    $SUDO env KUBECONFIG="$KUBECONFIG" cilium install --wait --wait-duration 8m >/dev/null
    echo "cilium: installed"
fi

# 3. KubeVirt + CDI
if k get crd kubevirts.kubevirt.io >/dev/null 2>&1; then
    echo "kubevirt: already installed (kept)"
else
    KV=$(curl -sL https://storage.googleapis.com/kubevirt-prow/release/kubevirt/kubevirt/stable.txt)
    k apply -f "https://github.com/kubevirt/kubevirt/releases/download/$KV/kubevirt-operator.yaml" >/dev/null
    k apply -f "https://github.com/kubevirt/kubevirt/releases/download/$KV/kubevirt-cr.yaml" >/dev/null
    echo "kubevirt: $KV applied"
fi
if k get crd cdis.cdi.kubevirt.io >/dev/null 2>&1; then
    echo "cdi: already installed (kept)"
else
    CV=$(latest_tag https://github.com/kubevirt/containerized-data-importer/releases/latest)
    k apply -f "https://github.com/kubevirt/containerized-data-importer/releases/download/$CV/cdi-operator.yaml" >/dev/null
    k apply -f "https://github.com/kubevirt/containerized-data-importer/releases/download/$CV/cdi-cr.yaml" >/dev/null
    echo "cdi: $CV applied"
fi

# 4. Envoy Gateway + GatewayClass 'eg'
if k get gatewayclass eg >/dev/null 2>&1; then
    echo "envoy-gateway: GatewayClass eg present (kept)"
elif command -v helm >/dev/null 2>&1; then
    EV=$(latest_tag https://github.com/envoyproxy/gateway/releases/latest)
    $SUDO -E helm upgrade --install eg oci://docker.io/envoyproxy/gateway-helm --version "$EV" \
        -n envoy-gateway-system --create-namespace --wait --timeout 5m >/dev/null
    k apply -f - >/dev/null <<EOF
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: eg
spec:
  controllerName: gateway.envoyproxy.io/gatewayclass-controller
EOF
    echo "envoy-gateway: $EV installed, GatewayClass eg created"
else
    echo "WARN: helm not found on remote; skipped Envoy Gateway" >&2
fi
REMOTE
    info "Cluster prerequisites ready"
fi

# ── Cluster preflight: fail loudly instead of reporting a false success ──
if _ssh "command -v kubectl >/dev/null 2>&1 || [ -x /usr/local/bin/kubectl ]" 2>/dev/null; then
    if ! _ssh "$SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl get nodes >/dev/null 2>&1"; then
        error "No reachable k3s cluster on ${HOST} (/etc/rancher/k3s/k3s.yaml missing or credentials rejected). Re-run with --bootstrap to install one."
    fi
    # KubeVirt must be Available before zorvia-api starts, or its readiness probe 503s on missing APIs.
    if _ssh "$SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl get crd kubevirts.kubevirt.io >/dev/null 2>&1"; then
        echo "  Waiting for KubeVirt to be Available (up to 10m)…"
        _ssh "$SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n kubevirt wait kubevirt/kubevirt --for=condition=Available --timeout=600s" >/dev/null 2>&1 \
            || warn "KubeVirt not Available yet; zorvia-api may stay unready until it is"
    else
        warn "KubeVirt CRDs not found; zorvia-api will not become ready (use --bootstrap)"
    fi
fi

# ── Image mode: deploy a prebuilt public image, no source build on the host ──
if $IMAGE_MODE; then
    step "Deploying image ${IMAGE_REF}"
    _ssh "mkdir -p ${REMOTE_DIR}/deploy" 
    _scp "$REPO_DIR/deploy/k8s.yaml" "${USER}@${HOST}:${REMOTE_DIR}/deploy/k8s.yaml"
    _ssh "SUDO='$SUDO' HOST='$HOST' IMAGE_REF='$IMAGE_REF' REMOTE_DIR='$REMOTE_DIR' bash -s" <<'REMOTE' 2>&1
set -euo pipefail
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
k() { $SUDO -E kubectl "$@"; }
cd "$REMOTE_DIR"
POLICY=IfNotPresent; case "$IMAGE_REF" in *:latest|*:*[!0-9a-zA-Z._-]*) POLICY=Always ;; esac
case "$IMAGE_REF" in *:*) ;; *) POLICY=Always ;; esac
sed -i "s#__ZORVIA_EXPOSE_HOST__#$HOST#g; s#image: zorvia:local#image: $IMAGE_REF#; s#imagePullPolicy: Never#imagePullPolicy: $POLICY#" deploy/k8s.yaml
k create namespace zorvia-system --dry-run=client -o yaml | k apply -f - >/dev/null
if ! k -n zorvia-system get secret zorvia-auth >/dev/null 2>&1; then
    pw=$(openssl rand -base64 18 | tr -d '/+=' | head -c 18)
    k -n zorvia-system create secret generic zorvia-auth \
        --from-literal=admin-user=admin --from-literal=admin-password="$pw" \
        --from-literal=jwt-secret="$(openssl rand -hex 32)" >/dev/null
    echo "auth secret: created. LOGIN  user=admin  password=$pw   (store it; shown once)"
else
    echo "auth secret: already present (kept)"
fi
k apply -f deploy/k8s.yaml >/dev/null
k -n zorvia-system rollout restart deployment/zorvia-api >/dev/null 2>&1 || true
k -n zorvia-system rollout status deployment/zorvia-api --timeout=300s
echo "k8s: zorvia-api running $IMAGE_REF"
REMOTE
    step "Verifying deployment"
    ok=""
    for _ in $(seq 1 30); do
        code=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 8 "https://${HOST}:30152/api/v1/health" || true)
        [ "$code" = "200" ] && { ok=1; break; }
        sleep 5
    done
    [ -n "$ok" ] || error "https://${HOST}:30152/api/v1/health did not return 200 (last: ${code:-none})"
    info "Healthy: https://${HOST}:30152/api/v1/health -> 200"
    echo ""
    echo "  Console: https://${HOST}:30152   (user: admin)"
    echo ""
    exit 0
fi

TOTAL_STEPS=5
$QUICK_MODE && TOTAL_STEPS=3

echo ""
echo "  ============================================"
echo "    zorvia Remote Deployment"
echo "  ============================================"
echo ""
echo "  Host:     ${USER}@${HOST}"
echo "  Auth:     $([ -n "$PASS" ] && echo "password" || echo "SSH key")"
echo "  Local:    $REPO_DIR"
echo "  Remote:   $REMOTE_DIR"
echo "  Mode:     $($QUICK_MODE && echo "quick (rsync + build only)" || echo "full (Rust toolchain + build + deploy)")"
echo ""

# ── Step 1: Rsync repo ──
step "Step 1/${TOTAL_STEPS}: Syncing repository to ${HOST}"

        if [ -f "$REPO_DIR/web/package.json" ]; then
    if [ ! -f "$REPO_DIR/web/dist/index.html" ]; then
        echo "  Building web UI (npm ci --legacy-peer-deps && npm run build)…"
        # Prefer locked install only (no unpinned `npm install`) for supply-chain hygiene.
        if (cd "$REPO_DIR/web" && npm ci --legacy-peer-deps >/dev/null 2>&1); then
            if ! (cd "$REPO_DIR/web" && npm run build); then
                warn "web build failed; deploy may serve placeholder UI"
            fi
        else
            warn "web npm ci failed; deploy may serve placeholder UI"
        fi
    else
        echo "  web/dist present — skipping local npm build"
    fi
fi

_rsync "$REPO_DIR/" "${USER}@${HOST}:${REMOTE_DIR}/" 2>&1 | tail -3
info "Synced to ${HOST}:${REMOTE_DIR}"

if ! $QUICK_MODE; then
    # ── Step 2: Install Rust toolchain if needed ──
    step "Step 2/${TOTAL_STEPS}: Ensuring Rust toolchain on remote"

    _ssh "
        if command -v cargo &>/dev/null; then
            echo \"Rust already installed: \$(rustc --version)\"
        else
            echo 'Installing Rust via rustup...'
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source \$HOME/.cargo/env
            echo \"Installed: \$(rustc --version)\"
        fi

        # Ensure build deps
        if command -v dnf &>/dev/null; then
            $SUDO dnf install -y gcc openssl-devel pkg-config 2>&1 | tail -1
        elif command -v apt-get &>/dev/null; then
            $SUDO apt-get install -y build-essential libssl-dev pkg-config 2>&1 | tail -1
        fi
    " 2>&1
    info "Rust toolchain ready"
fi

# ── Build on remote ──
if $QUICK_MODE; then
    BUILD_STEP=2
else
    BUILD_STEP=3
fi
step "Step ${BUILD_STEP}/${TOTAL_STEPS}: Building release binary on remote"

_ssh "
    source \$HOME/.cargo/env 2>/dev/null || true
    cd $REMOTE_DIR
    cargo build --release 2>&1 | tail -5
    strip target/release/zorvia 2>/dev/null || true
    ls -lh target/release/zorvia
" 2>&1
info "Binary built on remote"

# ── Install binary + service ──
if $QUICK_MODE; then
    INSTALL_STEP=3
else
    INSTALL_STEP=4
fi
step "Step ${INSTALL_STEP}/${TOTAL_STEPS}: Installing binary and service"

_ssh "
    cd $REMOTE_DIR

    # Stop host systemd front door — K8s NodePort HTTPS is primary (Veyron-style)
    $SUDO systemctl disable --now zorvia-web.service 2>/dev/null || true

    # Install binary (CLI / optional bare-metal)
    $SUDO cp target/release/zorvia /usr/local/bin/zorvia
    $SUDO chmod 755 /usr/local/bin/zorvia
    $SUDO cp deploy/zorvia-web.service /etc/systemd/system/zorvia-web.service
    $SUDO systemctl daemon-reload

    # Scoped kubeconfig for local CLI use
    $SUDO mkdir -p /etc/zorvia
    if [ -f /etc/rancher/k3s/k3s.yaml ]; then
        $SUDO cp /etc/rancher/k3s/k3s.yaml /etc/zorvia/kubeconfig
        $SUDO chown root:${USER} /etc/zorvia/kubeconfig
        $SUDO chmod 640 /etc/zorvia/kubeconfig
        echo 'kubeconfig: installed at /etc/zorvia/kubeconfig'
    fi

    if [ ! -f /etc/zorvia/env ]; then
        API_KEY=\$(openssl rand -hex 32)
        echo \"ZORVIA_API_KEY=\${API_KEY}\" | $SUDO tee /etc/zorvia/env > /dev/null
        $SUDO chmod 600 /etc/zorvia/env
        echo \"API key generated: \${API_KEY}\"
    else
        echo 'API key: already configured (kept existing)'
    fi

    # Build/import local image for k3s (zorvia:local) when container tooling exists.
    # Prefer docker; fall back to podman (common on Ubuntu labs without docker).
    # Use ubuntu:26.04 so host-built binaries (glibc 2.39+) run.
    BUILDER=""
    if command -v docker >/dev/null 2>&1; then
        BUILDER=docker
    elif command -v podman >/dev/null 2>&1; then
        BUILDER=podman
    fi
    if [ -n \"\$BUILDER\" ] && [ -x target/release/zorvia ]; then
        mkdir -p /tmp/zorvia-img/web
        cp -f target/release/zorvia /tmp/zorvia-img/zorvia
        cp -f deploy/Dockerfile.local /tmp/zorvia-img/Dockerfile
        if [ -d web/dist ] && [ -f web/dist/index.html ]; then
            rm -rf /tmp/zorvia-img/web
            cp -a web/dist /tmp/zorvia-img/web
            echo "web UI: bundled from web/dist"
        else
            echo '<!DOCTYPE html><html><body><p>Zorvia UI not built. Run npm run build in web/.</p></body></html>' \
              > /tmp/zorvia-img/web/index.html
            echo 'web UI: placeholder (web/dist missing)'
        fi
        # Podman often tags as localhost/zorvia:local — normalize for imagePullPolicy: Never
        # --network=host: rootful podman on some hosts cannot create a build
        # network (netavark 'create veth pair: Invalid argument'), which failed
        # the apt-get step and left the pod in ErrImageNeverPull while this
        # script still reported success. The build needs no isolated network.
        $SUDO \"\$BUILDER\" build --network=host -t zorvia:local /tmp/zorvia-img \
          || { echo 'ERROR: zorvia:local image build failed' >&2; exit 1; }
        { $SUDO \"\$BUILDER\" save zorvia:local | $SUDO k3s ctr images import - 2>/dev/null; } \
          || { $SUDO \"\$BUILDER\" save zorvia:local | $SUDO ctr -n k8s.io images import - 2>/dev/null; } \
          || { echo 'ERROR: could not import zorvia:local into the cluster runtime' >&2; exit 1; }
        $SUDO k3s ctr images tag zorvia:local docker.io/library/zorvia:local 2>/dev/null || true
        $SUDO k3s ctr images tag localhost/zorvia:local zorvia:local 2>/dev/null || true
        $SUDO k3s ctr images tag localhost/zorvia:local docker.io/library/zorvia:local 2>/dev/null || true
        echo \"image: zorvia:local built/imported via \$BUILDER\"
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system rollout restart deployment/zorvia-api 2>/dev/null || true
    else
        echo 'docker/podman/binary missing; using existing zorvia:local if present'
    fi

    # Apply Kubernetes HTTPS manifests (in-pod TLS + NodePort 30152)
    export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
    if command -v kubectl >/dev/null 2>&1 || [ -x /usr/local/bin/kubectl ]; then
        # Fill in this deploy's target host on the remote (freshly-synced,
        # never-committed) copy -- k8s.yaml ships with a placeholder so a
        # value hardcoded for one host can't silently carry into a deploy
        # to a different one (this bit us: ZORVIA_EXPOSE_HOST/HOST left
        # pointing at an old host after redeploying elsewhere).
        sed -i "s/__ZORVIA_EXPOSE_HOST__/$HOST/g" deploy/k8s.yaml
        # Auth Secret is no longer in k8s.yaml — create random lab creds if missing.
        # Lab mode is off: the random credentials below are unique per deploy.
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl create namespace zorvia-system --dry-run=client -o yaml | $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl apply -f -
        if ! $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system get secret zorvia-auth >/dev/null 2>&1; then
            # Generate unique lab credentials (never commit fixed defaults).
            _lab_pw=\$(openssl rand -base64 18 | tr -d '/+=' | head -c 18)
            _lab_jwt=\$(openssl rand -hex 32)
            $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system create secret generic zorvia-auth \
              --from-literal=admin-user=admin \
              --from-literal=admin-password=\"\$_lab_pw\" \
              --from-literal=jwt-secret=\"\$_lab_jwt\"
            echo \"auth secret: created random lab credentials (admin password: \$_lab_pw)\"
            echo 'auth secret: store the password'
        else
            echo 'auth secret: already present'
        fi
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl apply -f deploy/k8s.yaml
        # Clean up legacy namespace/manifest names from earlier HTTP NodePort deploys
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl delete namespace zorvia --ignore-not-found 2>/dev/null || true
        if [ -d /var/lib/rancher/k3s/server/manifests ]; then
            $SUDO cp deploy/k3s-zorvia-web.yaml /var/lib/rancher/k3s/server/manifests/zorvia-web.yaml \
              || $SUDO cp deploy/k3s-zorvia-web.yaml /var/lib/rancher/k3s/server/manifests/zorvia-api.yaml \
              || true
            echo 'k3s auto-manifest: installed'
        fi
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system rollout status deployment/zorvia-api --timeout=180s || true
        echo 'k8s: zorvia-api applied (HTTPS NodePort 30152)'
    else
        echo 'kubectl not found; skipped Kubernetes apply'
    fi
" 2>&1
info "Binary and Kubernetes manifests installed"

# ── Verify ──
if $QUICK_MODE; then
    VERIFY_STEP=3
else
    VERIFY_STEP=5
fi
step "Step ${VERIFY_STEP}/${TOTAL_STEPS}: Verifying deployment"

sleep 3
VERIFY_OUT=$(_ssh "
    echo \"Binary:  \$(which zorvia 2>/dev/null || echo NOT_FOUND)\"
    echo \"Version: \$(zorvia --version 2>/dev/null || echo FAILED)\"
    echo \"Host systemd zorvia-web: \$(systemctl is-active zorvia-web 2>/dev/null || echo inactive)\"
    echo \"\"
    $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system get deploy,svc,pods 2>/dev/null || true
    echo \"\"
    if curl -sk -o /dev/null -w 'NodePort health: HTTP %{http_code}\n' --max-time 10 https://127.0.0.1:30152/api/v1/health; then
        :
    else
        echo 'NodePort 30152: not ready yet'
        $SUDO env KUBECONFIG=/etc/rancher/k3s/k3s.yaml kubectl -n zorvia-system describe pods -l app.kubernetes.io/component=api 2>/dev/null | tail -40 || true
    fi
" 2>&1) || true
echo "$VERIFY_OUT"
echo "$VERIFY_OUT" | grep -q 'NodePort health: HTTP 200' \
    || error "zorvia-api is not healthy on ${HOST}:30152 (see output above); deployment is NOT complete"

echo ""
echo "  ============================================"
echo "  Deployment complete: ${USER}@${HOST}"
echo "  ============================================"
echo ""
echo "  Connect:"
echo "    ssh ${USER}@${HOST}"
echo ""
echo "  Web Dashboard (Kubernetes HTTPS, Veyron-style NodePort):"
echo "    https://${HOST}:30152"
echo "    https://${HOST}:30152/api/v1/health"
echo ""
echo "  Service management:"
echo "    sudo kubectl -n zorvia-system get pods,svc"
echo "    sudo kubectl -n zorvia-system logs -l app.kubernetes.io/component=api -f"
echo ""
