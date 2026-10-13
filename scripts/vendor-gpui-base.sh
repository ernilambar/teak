#!/usr/bin/env bash
# Re-vendor gpui-base from the cargo registry and reapply our patch.
#
#   ./scripts/vendor-gpui-base.sh <version>   re-vendor a version, apply the patch
#   ./scripts/vendor-gpui-base.sh --diff      regenerate the patch from vendor/
#
# The version must match the gpui-base that gpui-kit depends on (see
# Cargo.lock), or Cargo silently ignores the [patch.crates-io] entry.
set -euo pipefail

cd "$(dirname "$0")/.."
VENDOR=vendor/gpui-base
PATCH=patches/gpui-base.patch

# Locate a pristine copy of gpui-base <version> in the cargo registry.
pristine() {
    local dir
    dir=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"gpui-base-$1" 2>/dev/null | head -n1)
    if [[ -z "$dir" ]]; then
        echo "gpui-base $1 not in the cargo registry; run 'cargo fetch' first." >&2
        exit 1
    fi
    echo "$dir"
}

if [[ "${1:-}" == "--diff" ]]; then
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$VENDOR/Cargo.toml" | head -n1)
    src=$(pristine "$version")
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cp -R "$src/src" "$tmp/a"
    cp -R "$VENDOR/src" "$tmp/b"
    (cd "$tmp" && git diff --no-index --no-prefix a b || true) |
        sed -e 's#^--- a/#--- a/src/#' -e 's#^+++ b/#+++ b/src/#' \
            -e 's#^diff --git a/\(.*\) b/\(.*\)#diff --git a/src/\1 b/src/\2#' >"$PATCH"
    echo "Wrote $PATCH (against gpui-base $version)."
    exit 0
fi

version=${1:?usage: $0 <version> | --diff}
src=$(pristine "$version")
rm -rf "$VENDOR"
cp -R "$src" "$VENDOR"
rm -f "$VENDOR/.cargo-ok"
git apply --directory="$VENDOR" "$PATCH"
echo "Vendored gpui-base $version with $PATCH applied."
