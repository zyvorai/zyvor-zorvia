#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

# Render docs/social HTML cards to PNG/JPEG for README, OG, LinkedIn, and X.
# Needs Google Chrome and macOS `sips` (both already on a Mac); nothing is installed.
#   ./docs/social/build-social-cards.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
[[ -x "$CHROME" ]] || { echo "Google Chrome not found (set CHROME=...)" >&2; exit 1; }

render() {
  local html="$1" width="$2" height="$3" out="$4" fmt="$5" hash="${6:-}"
  local png
  png="$(mktemp "${TMPDIR:-/tmp}/zorvia-card.XXXXXX.png")"
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size="${width},${height}" --screenshot="$png" "file://${html}${hash}" >/dev/null 2>&1
  if [[ "$fmt" == "png" ]]; then
    sips -z "$height" "$width" "$png" --out "$out" >/dev/null
  else
    sips -s format jpeg -s formatOptions 92 -z "$height" "$width" "$png" --out "$out" >/dev/null
  fi
  rm -f "$png"
  echo "wrote $out ($(sips -g pixelWidth -g pixelHeight "$out" | awk '/pixel/{printf "%s ", $2}')px, $(du -k "$out" | cut -f1) KB)"
}

render "$HERE/zorvia-share-card.html" 1200 630 "$HERE/zorvia-share-card.png" png
render "$HERE/zorvia-share-card-dark.html" 1200 630 "$HERE/zorvia-share-card-dark.png" png
render "$HERE/zorvia-social-card.html" 1600 900 "$HERE/zorvia-social-card.jpg" jpeg
