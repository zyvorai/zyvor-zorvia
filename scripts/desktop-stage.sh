#!/usr/bin/env bash
# Stage the backend binary + web UI into desktop/src-tauri/stage/ for bundling.
# Usage: scripts/desktop-stage.sh [rust-target]   (default: host)
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:-}"
STAGE=desktop/src-tauri/stage
rm -rf "$STAGE" && mkdir -p "$STAGE/bin"
if [ -n "$TARGET" ]; then
  cargo build --release --target "$TARGET" --bin zorvia
  cp "target/$TARGET/release/zorvia" "$STAGE/bin/zorvia"
else
  cargo build --release --bin zorvia
  cp target/release/zorvia "$STAGE/bin/zorvia"
fi
(cd web && npm ci && npm run build)
cp -R web/dist "$STAGE/web"
echo "staged: $(du -sh "$STAGE" | cut -f1)"
