#!/bin/sh
set -eu

baseline=$1
project_root=${2:-/testbed}
history=/opt/agent-mem-eval/history.tsv

case "$baseline" in
  agent-mem)
    binary=/opt/agent-mem-bin/agent-mem
    # Harbor exposes every evaluated repository at /testbed. Mirror that
    # basename in isolated state so project-scoped MCP calls resolve directly.
    state_root=/tmp/testbed
    mkdir -p "$state_root"
    cd "$state_root"
    "$binary" init >/tmp/agent-mem-init.log
    tab=$(printf '\t')
    while IFS="$tab" read -r key value; do
      case "$key" in ''|'#'*) continue ;; esac
      "$binary" set "$key" "$value" >/dev/null
      "$binary" untrust "$key" >/dev/null
    done < "$history"
    exec "$binary" mcp --read-only
    ;;
  projectmem)
    version=$3
    cd "$project_root"
    python3 -m pip install --quiet --disable-pip-version-check \
      --target /tmp/projectmem-package "projectmem==$version"
    export PYTHONPATH=/tmp/projectmem-package
    python3 -m projectmem.cli init --no-hooks --no-global --no-watch --no-backfill --no-claude-md \
      --no-stack-detect --no-mcp-config --no-structure >/tmp/projectmem-init.log
    tab=$(printf '\t')
    while IFS="$tab" read -r key value; do
      case "$key" in ''|'#'*) continue ;; esac
      python3 -m projectmem.cli note "$value" >/dev/null
    done < "$history"
    exec python3 -m projectmem.mcp_server --root "$project_root"
    ;;
  official-mcp-memory)
    version=$3
    cp /opt/agent-mem-eval/official-memory.jsonl /tmp/official-memory.jsonl
    export MEMORY_FILE_PATH=/tmp/official-memory.jsonl
    exec npx -y "@modelcontextprotocol/server-memory@$version"
    ;;
  *)
    echo "unknown benchmark baseline: $baseline" >&2
    exit 64
    ;;
esac
