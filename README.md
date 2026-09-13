# agent-mem

[![CI](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml)
[![Security Audit](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![GitHub Release](https://img.shields.io/github/v/release/marcosotomac/agent-mem?include_prereleases)](https://github.com/marcosotomac/agent-mem/releases)

Ultra-fast, local-first, zero-daemon knowledge hypergraph and memory engine for AI coding agents.

## Design Principles
- **Sub-millisecond latency**: Embedded SQLite in WAL mode with clustered B-tree index (`WITHOUT ROWID`) and memory-mapped I/O (`PRAGMA mmap_size`). Sub-60µs point reads, <15µs relation traversal.
- **Clustered Knowledge Hypergraph**: Evolve beyond flat key-value pairs into engineering entities (`rule`, `decision`, `gotcha`, `pattern`) linked with typed directed edges (`mitigates`, `supersedes`, `depends_on`, `relates_to`).
- **Anchor-Aware Context Filtering**: Surgical prompt retrieval by repository anchor (`context --anchor src/auth.rs:10`) with 1-hop graph expansion, slashing token waste by >95% (from ~1,500 tokens down to <60 tokens).
- **Minimal token footprint**: Surgical 3-tool MCP schema consuming <150 tokens (982 characters vs ~3,000+ tokens in other memory tools). Raw plain text output with zero Markdown overhead.
- **Zero background daemons**: Direct stdio communication without background HTTP processes or port collisions.
- **Dual scopes**: Seamless access to isolated `project` memory (`.agent-mem/mem.db`) and user-level `global` preferences (`~/.config/agent-mem/global.db`).
- **BM25 search**: Full-text search powered by SQLite FTS5 with Porter stemming indexing keys, values, anchors, reasons, and entity kinds.
- **Session ring buffer**: Automatic atomic pruning preserving the latest 20 session checkpoints.

## Benchmarks & Competitor Comparison

`agent-mem` is engineered for extreme sub-millisecond execution, zero daemons, and radical prompt token discipline.

| Metric | agent-mem | agentmemory | mem0 | Static (CLAUDE.md) |
|---|---|---|---|---|
| **Point Lookup Latency (p50)** | **1.38 µs** | ~14 ms | ~150 ms | N/A |
| **BM25 Search Latency (p50)** | **294.75 µs** | ~14 ms | N/A (vector) | ~5 ms (grep) |
| **Graph 1-Hop Traversal (p50)**| **1.00 µs** | ~25 ms | ~200 ms | N/A |
| **MCP Schema Overhead** | **982 chars (~245 tok)** | 54 tools (~5,000 tok) | ~3,500 tok | 0 tok |
| **Anchor Token Savings** | **99.0% reduction** | 0% (dump/vector) | 0% | N/A |
| **Architecture** | **Single static binary (2.5MB)** | Node.js + iii daemon + 4 ports | Python + Docker + Postgres | Static file |
| **Runtime Memory (RSS)** | **~3 MB** | ~250 MB | ~500 MB+ | 0 MB |
| **Daemon Requirement** | **Zero daemons** | Pinned iii background engine | Docker / Python server | None |
| **Git / Team Sync** | **Native `.agent-rules` (union merge)** | None (local state only) | Cloud / API only | Manual git merge |

*Run the benchmark locally on your machine:*
```bash
cargo bench
```

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

# Read a rule value in raw text (<0.2ms)
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

# Dense context block for prompt injection (with optional code anchor or topic filter)
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

# Automatically configure AI clients (Claude, Cursor, Antigravity, Codex, Zed, Windsurf, Trae, etc.)
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
Dumping 50+ rules into every agent prompt wastes 1,500+ tokens. With code anchors:
```bash
agent-mem context --anchor src/auth/jwt.rs
```
`agent-mem` retrieves only memories directly anchored to that file path plus their 1-hop hypergraph neighbors (e.g. connected gotchas and ADRs), cutting prompt bloat down to **<60 tokens (>95% reduction)**.

## Team Git Sync (No Binary Conflicts)

Unlike legacy memory engines that commit binary SQLite databases into Git (causing unresolvable merge conflicts) or require proprietary cloud sync, `agent-mem` uses a deterministic text sync protocol:

- Local `.agent-mem/mem.db` stays in `.gitignore` as an ultra-fast sub-millisecond local cache.
- Project conventions and graph relations are version-controlled in `.agent-rules` as clean, PR-reviewable plain text:
  ```text
  [decision] auth/jwt = Use RS256 with key rotation (@ src/auth/jwt.rs:42)
  [gotcha] gotcha/token-leak = Avoid passing tokens in query strings
  [rel] auth/jwt -> mitigates -> gotcha/token-leak
  ```
- `agent-mem set`, `relate`, `unrelate`, `del`, `archive`, and `unarchive` automatically update `.agent-rules` in real time.
- Obsolete conventions are soft-deprecated into an `# Archived Rules` block with migration reasons, preventing AI agents from repeating dead patterns while sparing prompt tokens.
- `agent-mem init` installs Git hooks (`post-commit`, `post-merge`, `post-checkout`, `post-rewrite`) that automatically keep `.agent-rules` and local SQLite in sync across rebases and branch switches. Zero binary conflicts, 100% PR visibility!

## Model Context Protocol (MCP) Setup

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

### Surgical MCP Tools (<150 tokens total schema)
- `mem_set`: Store rules, decisions, or gotchas with optional code anchor, kind, relation, and scope (`project` or `global`).
- `mem_find`: Search memories via BM25 keywords across `project`, `global`, or `all` scopes.
- `mem_context`: Retrieve dense context block with optional `anchor` or `topic` filter for surgical LLM prompt injection.
