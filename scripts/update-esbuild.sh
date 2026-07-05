#!/usr/bin/env bash
#
# Update the precompiled esbuild binary that Spaxum ships (x86_64 Linux).
#
# Spaxum embeds a precompiled esbuild binary at spaxum/esbuild and uses it to
# bundle/serve frontend assets. This script fetches the latest linux-x64 build
# from the npm registry and replaces that binary.
#
# Usage:
#   scripts/update-esbuild.sh [version]
#
#   version   Optional exact esbuild version (e.g. 0.28.1). Defaults to latest.
#
set -euo pipefail

PKG="@esbuild/linux-x64"
REGISTRY="https://registry.npmjs.org/${PKG}"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/spaxum/esbuild"

for cmd in curl tar grep; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required command '$cmd' not found" >&2
    exit 1
  fi
done

# Resolve the version doc (either a pinned version or the latest dist-tag).
REQUESTED="${1:-latest}"
echo "Fetching ${PKG}@${REQUESTED} metadata..."
META="$(curl -fsSL "${REGISTRY}/${REQUESTED}")"

VERSION="$(printf '%s' "$META" | grep -o '"version":"[^"]*"' | head -n1 | cut -d'"' -f4)"
TARBALL="$(printf '%s' "$META" | grep -o '"tarball":"[^"]*"' | head -n1 | cut -d'"' -f4)"

if [ -z "$VERSION" ] || [ -z "$TARBALL" ]; then
  echo "error: could not resolve esbuild version/tarball from registry" >&2
  exit 1
fi

CURRENT="$("$DEST" --version 2>/dev/null || echo "none")"
echo "Current: ${CURRENT}"
echo "Target:  ${VERSION}"

if [ "$CURRENT" = "$VERSION" ]; then
  echo "Already up to date."
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading ${TARBALL}..."
curl -fsSL "$TARBALL" -o "$TMP/esbuild.tgz"

# The npm platform package stores the executable at package/bin/esbuild.
echo "Extracting..."
tar -xzf "$TMP/esbuild.tgz" -C "$TMP" package/bin/esbuild

mv "$TMP/package/bin/esbuild" "$DEST"
chmod +x "$DEST"

echo "Updated ${DEST} to esbuild $("$DEST" --version)"
