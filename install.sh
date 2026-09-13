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
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
fi

echo "Downloading agent-mem from ${URL}..."
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT

curl -sSfL "$URL" -o "${TEMP_DIR}/${ASSET}"
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
