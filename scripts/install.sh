#!/bin/sh
# Install the latest Teak release into /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/ernilambar/teak/main/scripts/install.sh | sh
#
# Files fetched with curl carry no quarantine flag, so the ad-hoc signed app
# opens without a Gatekeeper prompt.
set -eu

URL="https://github.com/ernilambar/teak/releases/latest/download/Teak-macos-arm64.zip"
DEST="/Applications/Teak.app"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "Teak requires macOS on Apple Silicon." >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading Teak..."
curl -fsSL "$URL" -o "$tmp/Teak.zip"
ditto -x -k "$tmp/Teak.zip" "$tmp"

if pkill -f "$DEST/Contents/MacOS/"; then
    while pgrep -f "$DEST/Contents/MacOS/" >/dev/null; do
        sleep 0.1
    done
fi

rm -rf "$DEST"
ditto "$tmp/Teak.app" "$DEST"

echo "Installed $DEST"
