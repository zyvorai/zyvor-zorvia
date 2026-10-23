#!/usr/bin/env bash
# Sign (inside-out, hardened runtime), package, notarize and staple a macOS app.
# Run on the Mac mini. The notary API key stays in the keychain profile
# (Macora-Notary); this script never reads or copies the .p8.
# Usage: desktop-sign-notarize.sh path/to/App.app out.dmg [volume-name]
set -euo pipefail
APP="$1"; DMG="$2"; VOL="${3:-$(basename "$APP" .app)}"
ID="${SIGN_IDENTITY:-Developer ID Application: ZYVORAI LABS PRIVATE LIMITED (NX667LTDQ2)}"
PROFILE="${NOTARY_PROFILE:-Macora-Notary}"
ENT="$(mktemp -t ent).plist"
cat > "$ENT" <<'P'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
<key>com.apple.security.cs.allow-jit</key><true/>
<key>com.apple.security.cs.disable-library-validation</key><true/>
</dict></plist>
P

sign() { codesign --force --timestamp --options runtime --entitlements "$ENT" -s "$ID" "$1"; }

# 1. every Mach-O inside the bundle, deepest first (the main executable last)
MAIN="$APP/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP/Contents/Info.plist")"
find "$APP" -type f ! -path "$MAIN" -print0 | while IFS= read -r -d '' f; do
  if file -b "$f" | grep -q 'Mach-O'; then sign "$f"; fi
done
sign "$MAIN"
sign "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

# 2. dmg
STAGE="$(mktemp -d)"; cp -R "$APP" "$STAGE/"; ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"; diskutil image create from --format UDZO --volumeName "$VOL" "$STAGE" "$DMG" >/dev/null
codesign --force --timestamp -s "$ID" "$DMG"

# 3. notarize + staple
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$DMG"
spctl -a -t open --context context:primary-signature -vv "$DMG"
echo "OK: $DMG"
