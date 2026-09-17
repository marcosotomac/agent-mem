#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: render-homebrew-formula.sh <version> <asset-dir> <output>" >&2
  exit 2
fi

VERSION="${1#v}"
ASSET_DIR="$2"
OUTPUT="$3"

digest() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{ print $1 }'
  else
    shasum -a 256 "$file" | awk '{ print $1 }'
  fi
}

DARWIN_ARM="$(digest "${ASSET_DIR}/agent-mem-darwin-aarch64.tar.gz")"
DARWIN_X64="$(digest "${ASSET_DIR}/agent-mem-darwin-x86_64.tar.gz")"
LINUX_ARM="$(digest "${ASSET_DIR}/agent-mem-linux-aarch64.tar.gz")"
LINUX_X64="$(digest "${ASSET_DIR}/agent-mem-linux-x86_64.tar.gz")"

mkdir -p "$(dirname "$OUTPUT")"
sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@DARWIN_ARM_SHA@/${DARWIN_ARM}/g" \
  -e "s/@DARWIN_X64_SHA@/${DARWIN_X64}/g" \
  -e "s/@LINUX_ARM_SHA@/${LINUX_ARM}/g" \
  -e "s/@LINUX_X64_SHA@/${LINUX_X64}/g" \
  "$(dirname "${BASH_SOURCE[0]}")/agent-mem.rb.template" > "$OUTPUT"
