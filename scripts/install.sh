#!/bin/sh
# Install the latest Tine release into /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/ernilambar/tine/main/scripts/install.sh | sh
#
# Files fetched with curl carry no quarantine flag, so the ad-hoc signed app
# opens without a Gatekeeper prompt.
set -eu

URL="https://github.com/ernilambar/tine/releases/latest/download/Tine-macos-arm64.zip"
DEST="/Applications/Tine.app"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "Tine requires macOS on Apple Silicon." >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading Tine..."
curl -fsSL "$URL" -o "$tmp/Tine.zip"
ditto -x -k "$tmp/Tine.zip" "$tmp"

if pkill -f "$DEST/Contents/MacOS/"; then
    while pgrep -f "$DEST/Contents/MacOS/" >/dev/null; do
        sleep 0.1
    done
fi

rm -rf "$DEST"
ditto "$tmp/Tine.app" "$DEST"

echo "Installed $DEST"
