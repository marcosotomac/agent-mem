#!/usr/bin/env bash
set -euo pipefail

REPO="marcosotomac/agent-mem"
INSTALL_DIR="${AGENT_MEM_INSTALL_DIR:-$HOME/.local/bin}"
TAG="${1:-latest}"

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  arm64|aarch64)
    ARCH="aarch64"
    ;;
  *)
    echo "Error: Unsupported architecture $ARCH" >&2
    exit 1
    ;;
esac

case "$OS" in
  darwin)
    ASSET="agent-mem-darwin-${ARCH}.tar.gz"
    ;;
  linux)
    ASSET="agent-mem-linux-${ARCH}.tar.gz"
    ;;
  *)
    echo "Error: Unsupported OS $OS. For Windows, download the release zip." >&2
    exit 1
    ;;
esac

if [ "$TAG" = "latest" ]; then
  RELEASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  RELEASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
fi
URL="${RELEASE_URL}/${ASSET}"

echo "Downloading agent-mem from ${URL}..."
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT

curl -sSfL "$URL" -o "${TEMP_DIR}/${ASSET}"
curl -sSfL "${RELEASE_URL}/SHA256SUMS.txt" -o "${TEMP_DIR}/SHA256SUMS.txt"
EXPECTED_SHA="$(awk -v asset="$ASSET" '$2 == asset { print $1 }' "${TEMP_DIR}/SHA256SUMS.txt")"
if [ -z "$EXPECTED_SHA" ]; then
  echo "Error: No published SHA-256 checksum for ${ASSET}" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA="$(sha256sum "${TEMP_DIR}/${ASSET}" | awk '{ print $1 }')"
else
  ACTUAL_SHA="$(shasum -a 256 "${TEMP_DIR}/${ASSET}" | awk '{ print $1 }')"
fi
if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
  echo "Error: SHA-256 verification failed for ${ASSET}" >&2
  exit 1
fi
tar -xzf "${TEMP_DIR}/${ASSET}" -C "${TEMP_DIR}"

mkdir -p "$INSTALL_DIR"
install -m 755 "${TEMP_DIR}/agent-mem" "${INSTALL_DIR}/agent-mem"

echo "agent-mem successfully installed to ${INSTALL_DIR}/agent-mem"

# Check if INSTALL_DIR is in PATH
if ! echo ":$PATH:" | grep -q ":${INSTALL_DIR}:"; then
  echo ""
  echo "Add ${INSTALL_DIR} to your PATH to run 'agent-mem':"
  echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
fi
