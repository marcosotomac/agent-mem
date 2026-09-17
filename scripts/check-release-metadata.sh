#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT_DIR}/Cargo.toml" | head -n 1)"
NPM_VERSION="$(node -p "require('${ROOT_DIR}/npm/package.json').version")"
FORMULA_VERSION="$(sed -n 's/^[[:space:]]*version "\([^"]*\)"/\1/p' "${ROOT_DIR}/Formula/agent-mem.rb")"

if [ -z "$CARGO_VERSION" ] || [ "$CARGO_VERSION" != "$NPM_VERSION" ] || [ "$CARGO_VERSION" != "$FORMULA_VERSION" ]; then
  echo "release metadata mismatch: cargo=${CARGO_VERSION:-missing} npm=${NPM_VERSION:-missing} formula=${FORMULA_VERSION:-missing}" >&2
  exit 1
fi

if [ "$#" -gt 0 ]; then
  TAG_VERSION="${1#v}"
  if [ "$TAG_VERSION" != "$CARGO_VERSION" ]; then
    echo "tag version ${TAG_VERSION} does not match package version ${CARGO_VERSION}" >&2
    exit 1
  fi
fi

echo "release metadata synchronized at ${CARGO_VERSION}"
