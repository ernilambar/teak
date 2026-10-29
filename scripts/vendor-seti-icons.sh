#!/usr/bin/env bash
# Vendors the Seti file icons into assets/file-icons/.
#
#   ./scripts/vendor-seti-icons.sh
#
# Seti's own associations cannot be copied: VS Code ships the theme as a font
# (`seti.woff`) plus `languageIds`, not as an extension table. Only the art is
# fetched here; src/file_icons.rs maps Teak's names and extensions onto these
# icon names by hand.
#
# The art is normalized to a square viewBox. Twenty-nine of Seti's icons use a
# 6:5 canvas (the "1200 1000" set) and seventeen carry only width/height, so
# GPUI would rasterize them wider than tall and stretch them in the sidebar.
# Padding the shorter axis keeps every icon's aspect ratio and centers it.
#
# Bump REV and seti-ui's version in assets/file-icons/LICENSE together.
set -euo pipefail

REPO="jesseweed/seti-ui"
REV="2d6c5e68b4ded73c92dac291845ee44e1182d511"

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/assets/file-icons"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$REV" | tar -xz -C "$tmp"

# The LICENSE beside the icons is Teak's, not the tarball's, so only the art
# is replaced.
mkdir -p "$dest"
find "$dest" -name '*.svg' -delete
cp "$tmp"/seti-ui-*/icons/*.svg "$dest"/

python3 - "$dest" <<'PY'
import glob
import os
import re
import sys

VIEW_BOX = re.compile(
    r'viewBox="([\d.eE+-]+)[ ,]+([\d.eE+-]+)[ ,]+([\d.eE+-]+)[ ,]+([\d.eE+-]+)"'
)

for path in sorted(glob.glob(os.path.join(sys.argv[1], "*.svg"))):
    with open(path, encoding="utf-8") as handle:
        text = handle.read()

    match = VIEW_BOX.search(text)
    if not match:
        width_match = re.search(r'\bwidth="([\d.]+)"', text)
        height_match = re.search(r'\bheight="([\d.]+)"', text)
        if not (width_match and height_match):
            continue
        width, height = float(width_match.group(1)), float(height_match.group(1))
        side = max(width, height)
        box = (-(side - width) / 2, -(side - height) / 2, side, side)
        replacement = 'viewBox="{} {} {} {}" '.format(*(f"{value:g}" for value in box))
        text = text[:width_match.start()] + replacement + text[width_match.start():]
    else:
        x, y, width, height = (float(value) for value in match.groups())
        if abs(width - height) < 1e-9:
            continue
        box = (x - (max(width, height) - width) / 2,
               y - (max(width, height) - height) / 2,
               max(width, height),
               max(width, height))
        replacement = 'viewBox="{} {} {} {}"'.format(*(f"{value:g}" for value in box))
        text = text[:match.start()] + replacement + text[match.end():]

    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)

# Every other file name in the tree is lowercase; `R.svg` is the one outlier,
# and a case-only rename is invisible to a case-insensitive macOS checkout but
# breaks a case-sensitive CI checkout. Two steps, because renaming straight to
# the lowercase name is a no-op while the original still exists.
for path in sorted(glob.glob(os.path.join(sys.argv[1], "*.svg"))):
    lower = path.lower()
    if lower != path:
        os.replace(path, path + ".tmp")
        os.replace(path + ".tmp", lower)
PY

echo "vendored $(find "$dest" -name '*.svg' | wc -l | tr -d ' ') icons from $REPO@$REV"

# Seti's art is inset in its canvas; tighten each icon to its ink box so it
# reads at the same optical size as the rest of the UI's glyphs.
cargo run --quiet --manifest-path "$root/Cargo.toml" --example normalize-icons
