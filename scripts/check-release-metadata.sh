#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT_DIR}/Cargo.toml" | head -n 1)"
NPM_VERSION="$(node -p "require('${ROOT_DIR}/npm/package.json').version")"

if [ -z "$CARGO_VERSION" ] || [ "$CARGO_VERSION" != "$NPM_VERSION" ]; then
  echo "release metadata mismatch: cargo=${CARGO_VERSION:-missing} npm=${NPM_VERSION:-missing}" >&2
  exit 1
fi

if [ "$#" -gt 0 ]; then
  TAG_VERSION="${1#v}"
  if [ "$TAG_VERSION" != "$CARGO_VERSION" ]; then
    echo "tag version ${TAG_VERSION} does not match package version ${CARGO_VERSION}" >&2
    exit 1
  fi
fi

# The Homebrew formula is rendered only after immutable release archives exist,
# because its checksums cannot be known before that point. The release workflow
# synchronizes its version and SHA-256 values from the tag and built artifacts.
echo "release metadata synchronized at ${CARGO_VERSION}"
