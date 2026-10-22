#!/usr/bin/env bash
# Build the app, install it to /Applications, and relaunch it.
#
#   ./scripts/dev.sh [--release] [file ...]
#
# Use this to test behaviour that only works from an app bundle: the
# command-line launcher, Finder's "Open With", and files opened through
# LaunchServices. For plain editor changes, `bacon run` is faster.
#
# The running /Applications copy is killed, so its unsaved edits are lost.
set -euo pipefail

cd "$(dirname "$0")/.."

profile="debug"
flag=""
if [ "${1:-}" = "--release" ]; then
    profile="release"
    flag="--release"
    shift
fi

if ! command -v cargo-bundle >/dev/null 2>&1; then
    echo "cargo-bundle is not installed. Run: cargo install cargo-bundle" >&2
    exit 1
fi

cargo bundle $flag

SRC="target/$profile/bundle/osx/Tine.app"
DEST="/Applications/Tine.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

codesign --force --deep --sign - "$SRC"

if pkill -f "$DEST/Contents/MacOS/"; then
    while pgrep -f "$DEST/Contents/MacOS/" >/dev/null; do
        sleep 0.1
    done
fi

rm -rf "$DEST"
ditto "$SRC" "$DEST"
"$LSREGISTER" -f "$DEST"

echo "Installed $profile build to $DEST"

if [ "$#" -eq 0 ]; then
    open "$DEST"
else
    open -a "$DEST" -- "$@"
fi
