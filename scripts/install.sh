#!/bin/sh
# Install the latest release into /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/ernilambar/tinytext/main/scripts/install.sh | sh
#
# Files fetched with curl carry no quarantine flag, so the ad-hoc signed app
# opens without a Gatekeeper prompt.
set -eu

URL="https://github.com/ernilambar/tinytext/releases/latest/download/Tinytext-macos-arm64.zip"
DEST="/Applications/Tinytext.app"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "This app requires macOS on Apple Silicon." >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading the app..."
curl -fsSL "$URL" -o "$tmp/Tinytext.zip"
ditto -x -k "$tmp/Tinytext.zip" "$tmp"

if pkill -f "$DEST/Contents/MacOS/"; then
    while pgrep -f "$DEST/Contents/MacOS/" >/dev/null; do
        sleep 0.1
    done
fi

rm -rf "$DEST"
ditto "$tmp/Tinytext.app" "$DEST"

echo "Installed $DEST"
