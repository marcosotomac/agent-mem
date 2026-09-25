#!/bin/sh
set -eu

if ! command -v git >/dev/null 2>&1; then
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq git >/dev/null
fi

export CARGO_TARGET_DIR=/tmp/agent-mem-target
cargo test --locked --no-default-features --test registry_test --test store_test --test team_conflict_e2e_test
sh scripts/docker-smoke.sh
