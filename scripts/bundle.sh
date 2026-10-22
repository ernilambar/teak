#!/usr/bin/env bash
# Build a release macOS .app bundle you can run and share.
#
#   ./scripts/bundle.sh
#
# Produces target/release/bundle/osx/Teak.app and ad-hoc signs it so it
# launches locally. To distribute to other Macs without Gatekeeper warnings,
# sign with a Developer ID and notarize (see below).
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-bundle >/dev/null 2>&1; then
    echo "cargo-bundle is not installed. Run: cargo install cargo-bundle" >&2
    exit 1
fi

if [ ! -f assets/icon.icns ]; then
    echo "==> assets/icon.icns missing; generating the app icon"
    python3 scripts/make-icon.py
fi

cargo bundle --release

APP="target/release/bundle/osx/Teak.app"

# Ad-hoc signature: enough for this machine, not for distribution.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

DMG="target/release/bundle/dmg/Teak.dmg"

echo
echo "Bundled: $APP"
if [ -f "$DMG" ]; then
    echo "Installer: $DMG"
fi
echo "Run it with: open \"$APP\""
echo
echo "To share without Gatekeeper warnings, sign and notarize:"
echo "  codesign --force --deep --options runtime --sign \"Developer ID Application: NAME (TEAMID)\" \"$APP\""
echo "  ditto -c -k --keepParent \"$APP\" Teak.zip"
echo "  xcrun notarytool submit Teak.zip --keychain-profile PROFILE --wait"
echo "  xcrun stapler staple \"$APP\""
