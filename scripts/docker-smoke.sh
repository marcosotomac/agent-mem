#!/bin/sh
set -eu

binary=${AGENT_MEM_SMOKE_BINARY:-/tmp/agent-mem-target/debug/agent-mem}
cargo build --locked --no-default-features --target-dir /tmp/agent-mem-target
sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT
export AGENT_MEM_GLOBAL_DIR="$sandbox/global"
project="$sandbox/project"
mkdir -p "$project"

"$binary" init "$project"
cd "$project"
"$binary" set decision/auth 'Use RS256'
"$binary" set decision/auth 'Use Ed25519'
test "$("$binary" get decision/auth)" = 'Use Ed25519'
"$binary" history decision/auth | grep -q 'Use RS256'
"$binary" history decision/auth | grep -q 'Use Ed25519'
"$binary" sync
test "$("$binary" get decision/auth)" = 'Use Ed25519'
"$binary" projects | grep -q project

printf 'decision/auth = Use RS256\n' >> .agent-rules
if "$binary" sync > "$sandbox/conflict.stdout" 2> "$sandbox/conflict.stderr"; then
    echo 'sync unexpectedly accepted contradictory definitions' >&2
    exit 1
fi
grep -q 'Conflicting definitions' "$sandbox/conflict.stderr"
test "$("$binary" get decision/auth)" = 'Use Ed25519'
"$binary" sync --accept-conflicts
test "$("$binary" get decision/auth)" = 'Use RS256'
"$binary" history decision/auth | grep -q 'conflict'

mkdir -p "$sandbox/parallel-a" "$sandbox/parallel-b"
"$binary" init "$sandbox/parallel-a" > "$sandbox/parallel-a.stdout" &
first_pid=$!
"$binary" init "$sandbox/parallel-b" > "$sandbox/parallel-b.stdout" &
second_pid=$!
wait "$first_pid"
wait "$second_pid"
"$binary" projects | grep -q parallel-a
"$binary" projects | grep -q parallel-b

printf '{broken' > "$AGENT_MEM_GLOBAL_DIR/projects.json"
if "$binary" init "$sandbox/other" > "$sandbox/other.stdout" 2> "$sandbox/other.stderr"; then
    echo 'init unexpectedly accepted a corrupt registry' >&2
    exit 1
fi
grep -q 'Project registry' "$sandbox/other.stderr"
test "$(cat "$AGENT_MEM_GLOBAL_DIR/projects.json")" = '{broken'
echo 'Docker CLI smoke passed'
