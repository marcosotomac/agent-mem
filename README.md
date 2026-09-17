# agent-mem

[![CI](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml)
[![Security Audit](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![GitHub Release](https://img.shields.io/github/v/release/marcosotomac/agent-mem?include_prereleases)](https://github.com/marcosotomac/agent-mem/releases)

Ultra-fast, local-first, zero-daemon knowledge hypergraph and memory engine for AI coding agents.

## Design Principles
- **Fast local reads**: Embedded SQLite in WAL mode with a clustered B-tree (`WITHOUT ROWID`) and memory-mapped I/O (`PRAGMA mmap_size`). See the benchmark for measured operation latencies, including MCP and export costs.
- **Clustered Knowledge Hypergraph**: Evolve beyond flat key-value pairs into engineering entities (`rule`, `decision`, `gotcha`, `pattern`) linked with typed directed edges (`mitigates`, `supersedes`, `depends_on`, `relates_to`).
- **Indexed Anchor-Aware Context Filtering**: Schema v4 routes exact paths, directory hierarchy, and basenames through a compact hash index, then validates original anchors before 1-hop graph expansion. Retrieval work stays bounded by the requested result limit.
- **Minimal token footprint**: Four consolidated MCP tools in 1,366 schema bytes (~341 tokens by the documented 4-byte estimate), with bounded plain-text results and anchor-aware retrieval.
- **Zero background daemons**: Direct stdio communication without background HTTP processes or port collisions.
- **Dual scopes**: Seamless access to isolated `project` memory (`.agent-mem/mem.db`) and user-level `global` preferences (`~/.config/agent-mem/global.db`).
- **BM25 search**: Weighted SQLite FTS5 search across keys, values, anchors, reasons, and kinds. Strict queries keep the one-query fast path; a bounded OR fallback runs only after zero hits.
- **Session ring buffer**: Automatic atomic pruning preserving the latest 20 session checkpoints.

## Benchmarks

Run the reproducible local benchmark in release mode:
```bash
cargo bench --bench bench_suite
```

[Measured results and methodology](BENCHMARK.md) cover point reads, FTS5 search,
outgoing edge lookup, warm in-process MCP search, selective context, full export,
and writes with export. The corpus contains 2,000 memories and 666 valid directed edges.
Results include p50, p95 and p99; they depend on hardware and workload. MCP timings
exclude process startup and stdio transport.

Context measurements separate filtering from result caps and verify the untruncated
result against the relevant rules. Byte reduction is not a tokenizer measurement.
Cross-engine rankings are omitted until datasets, queries, transport and limits match.
The default report is written to `target/benchmark.md`; use
`AGENT_MEM_BENCH_REPORT=BENCHMARK.md cargo bench --bench bench_suite` to refresh the tracked report.

For end-to-end conflict, corruption, concurrency, context-budget, and relative
performance coverage, see [the real-world validation suite](tests/REAL_WORLD_TESTS.md).
It includes fixed-qrel noisy-query evaluation and an explicit 100,000-record release benchmark.
Agent-level claims are evaluated separately with pinned external repositories,
real tasks, repeated model runs, and identical baselines; see [the end-to-end
benchmark protocol](evals/README.md). No agent-quality result is published until
the corresponding run artifacts are available.

## Installation

### 1. One-line curl installer (macOS / Linux)
```bash
curl -fsSL https://raw.githubusercontent.com/marcosotomac/agent-mem/main/install.sh | bash
```

### 2. Zero-install via NPX (All platforms)
Run directly without installing any toolchain:
```bash
npx agent-mem --help
```

### 3. Homebrew
```bash
brew install marcosotomac/tap/agent-mem
```

### 4. From source via Cargo (Rust)
```bash
cargo install --path .
```

## CLI Usage
```bash
# Initialize isolated project memory (SQLite WAL, .gitignore, git hook)
agent-mem init

# Store or update a memory rule, decision, gotcha, or pattern
agent-mem set <key> <value> [--anchor <path:line>] [--kind <rule|decision|gotcha|pattern>]

# Link memories in the knowledge hypergraph
agent-mem relate <source_key> <rel_type> <target_key>
# Example: agent-mem relate auth/jwt mitigates gotcha/token-leak

# Remove a hypergraph relation
agent-mem unrelate <source_key> <rel_type> <target_key>

# Read a rule value in raw text
agent-mem get <key>

# Delete a rule and all its connected relations
agent-mem del <key>

# Archive an obsolete rule with migration reason (zero prompt token waste, discoverable via search)
agent-mem archive <key> [--reason <reason>]

# Reactivate an archived rule
agent-mem unarchive <key>

# List all active rules
agent-mem dump

# Fast BM25 keyword search across rules, decisions, gotchas, and patterns
agent-mem find <query>

# Session checkpoints (auto-pruned to latest 20)
agent-mem session add <summary>
agent-mem session list

# Dense context block for targeted delivery (with optional code anchor or topic filter)
agent-mem context [--anchor <path:line>] [--topic <prefix>] [--limit <n>]

# Synchronize team rules (.agent-rules) without SQLite binary conflicts
agent-mem sync [file] [--export]

# Launch interactive Terminal UI (Hypergraph inspector, Rules, Sessions, Project switcher, Doctor)
agent-mem tui

# List all registered repositories across your machine with rules & session stats
agent-mem projects [--prune]

# Inspect git health and detect AI IDE/CLI configurations (17 supported clients)
agent-mem doctor

# Start native Model Context Protocol (MCP) stdio server
agent-mem mcp

# Explicitly configure AI clients (creates a backup; init never edits client configs)
agent-mem mcp install [all|<client>]
```

## Knowledge Hypergraph & Multi-Entity Storage

Engineering memory is not flat. `agent-mem` supports 4 first-class entities and typed relationships:

| Entity | Purpose | Example Key |
|---|---|---|
| `rule` | Architectural constraints, conventions, guidelines (default) | `arch/db`, `style/rust` |
| `decision` | Architecture Decision Records (ADRs), rationale, tradeoffs | `decision/auth`, `adr/001` |
| `gotcha` | Production bugs, performance traps, unexpected library quirks | `gotcha/sqlite-wal-locking` |
| `pattern` | Reusable design patterns, idioms, template structures | `pattern/repository` |

Auto-inference detects key prefixes like `decision/*`, `adr/*`, `gotcha/*`, `bug/*`, `trap/*`, `pattern/*` automatically.

### Anchor-Aware Retrieval (Token Ratchet)
Prompt size grows with the number of injected rules. Filter by code anchor:
```bash
agent-mem context --anchor src/auth/jwt.rs
```
`agent-mem` retrieves memories directly anchored to that file path plus their outgoing 1-hop neighbors (e.g. connected gotchas and ADRs). Savings depend on the corpus and query. A result limit can omit relevant memories; the benchmark reports that loss separately from filtering savings.

## Team Git Sync (No Binary Conflicts)

Unlike legacy memory engines that commit binary SQLite databases into Git (causing unresolvable merge conflicts) or require proprietary cloud sync, `agent-mem` uses a deterministic text sync protocol:

- Local `.agent-mem/mem.db` stays in `.gitignore` as a local SQLite cache.
- Project conventions and graph relations are version-controlled in `.agent-rules` as clean, PR-reviewable plain text:
  ```text
  [decision] auth/jwt = Use RS256 with key rotation (@ src/auth/jwt.rs:42)
  [gotcha] gotcha/token-leak = Avoid passing tokens in query strings
  [rel] auth/jwt -> mitigates -> gotcha/token-leak
  ```
- `agent-mem set`, `relate`, `unrelate`, `del`, `archive`, and `unarchive` automatically update `.agent-rules` in real time.
- `agent-mem init` imports an existing `.agent-rules` before the first local write, including in freshly cloned repositories. Duplicate relations are imported idempotently.
- Simple rules retain the readable format above. Ambiguous content (multiline text, literal ` @ `, reserved key prefixes or metadata delimiters) uses `[rule-json-v1]` followed by a JSON record on one line. Ambiguous relation fields use `[rel-json-v1]` followed by a three-string JSON array. Escapes preserve the original content; MCP responses remain unchanged. Encoded archive timestamps use the stable marker `1`, like the legacy format. Sync rejects malformed encoded records before changing the database. Team members must use a version supporting these markers before syncing such files.
- Obsolete conventions are soft-deprecated into an `# Archived Rules` block with migration reasons, preventing AI agents from repeating dead patterns while sparing prompt tokens.
- `agent-mem init` installs Git hooks (`post-commit`, `post-merge`, `post-checkout`, `post-rewrite`) that automatically keep `.agent-rules` and local SQLite in sync across rebases and branch switches. Zero binary conflicts, 100% PR visibility!

## Model Context Protocol (MCP) Setup

`agent-mem` is dual-era compatible: it preserves the legacy `2024-11-05`
`initialize` handshake and supports stateless `2026-07-28` requests with
per-request `_meta` plus `server/discover`. Modern protocol metadata is emitted
only for modern clients, so legacy clients keep the same minimal token footprint.

Add `agent-mem` to your agent's MCP configuration (e.g., Claude Desktop, Cursor, Antigravity, or Cline):

### Option A: Native binary (recommended for maximum speed)
```json
{
  "mcpServers": {
    "agent-mem": {
      "command": "agent-mem",
      "args": ["mcp"]
    }
  }
}
```

### Option B: Zero-install via NPX (no setup needed)
```json
{
  "mcpServers": {
    "agent-mem": {
      "command": "npx",
      "args": ["-y", "agent-mem", "mcp"]
    }
  }
}
```

### Surgical MCP Tools (~341 estimated tokens total schema)
- `mem_set`: Store rules, decisions, or gotchas with optional code anchor, kind, relation, and scope (`project` or `global`).
- `mem_find`: Search memories via BM25 keywords across `project`, `global`, or `all` scopes.
- `mem_context`: Retrieve dense context block with optional `anchor` or `topic` filter for surgical LLM prompt injection.
- `mem_manage`: Archive, reactivate, delete, link, or unlink memories without expanding the tool surface.

Every project-scoped MCP call accepts `project` as a registered project id,
unique name, or canonical path. When the server cannot infer one unambiguously,
it returns the available ids instead of guessing or creating a database in the
client's launch directory.

## Security and artifact verification

Treat `.agent-rules` and all recalled values as untrusted repository content;
review changes in pull requests and never interpret recalled text as authority
to run commands or disclose secrets. See the [threat model](THREAT_MODEL.md) for
the full trust boundary and enforced request, write, batch, and import limits.

Release installers verify every archive against the release checksum manifest.
GitHub Actions also publishes build-provenance attestations, which can be checked
after downloading an archive:

```bash
gh attestation verify agent-mem-darwin-aarch64.tar.gz --repo marcosotomac/agent-mem
```
