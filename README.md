# agent-mem

[![CI](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/ci.yml)
[![Security Audit](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml/badge.svg)](https://github.com/marcosotomac/agent-mem/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![GitHub Release](https://img.shields.io/github/v/release/marcosotomac/agent-mem?include_prereleases)](https://github.com/marcosotomac/agent-mem/releases)

Ultra-fast, local-first, zero-daemon memory engine for AI coding agents.

## Design Principles
- **Sub-millisecond latency**: Embedded SQLite in WAL mode with clustered B-tree index (`WITHOUT ROWID`) and memory-mapped I/O (`PRAGMA mmap_size`).
- **Minimal token footprint**: Surgical 3-tool MCP schema consuming <160 tokens (vs ~3,000+ tokens in other memory tools). Raw plain text output with zero Markdown overhead.
- **Zero background daemons**: Direct stdio communication without background HTTP processes or port collisions.
- **Code anchors**: Link decisions and rules to repository-relative files and lines (`--anchor src/auth.rs:40`).
- **Dual scopes**: Seamless access to isolated `project` memory (`.agent-mem/mem.db`) and user-level `global` preferences (`~/.config/agent-mem/global.db`).
- **BM25 search**: Full-text search powered by SQLite FTS5 with Porter stemming.
- **Session ring buffer**: Automatic atomic pruning preserving the latest 20 session checkpoints.

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

# Store or update a memory rule (with optional code anchor)
agent-mem set <key> <value> [--anchor <path:line>]

# Read a rule value in raw text (<0.2ms)
agent-mem get <key>

# Delete a rule
agent-mem del <key>

# Archive an obsolete rule with migration reason (zero prompt token waste, discoverable via search)
agent-mem archive <key> [--reason <reason>]

# Reactivate an archived rule
agent-mem unarchive <key>

# List all active rules
agent-mem dump

# Fast BM25 keyword search
agent-mem find <query>

# Session checkpoints (auto-pruned to latest 20)
agent-mem session add <summary>
agent-mem session list

# Dense context block for prompt injection
agent-mem context

# Synchronize team rules (.agent-rules) without SQLite binary conflicts
agent-mem sync [file] [--export]

# Launch interactive Terminal UI (Rules manager, Sessions, Project switcher, Doctor)
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

## Team Git Sync (No Binary Conflicts)

Unlike legacy memory engines that commit binary SQLite databases into Git (causing unresolvable merge conflicts) or require proprietary cloud sync, `agent-mem` uses a deterministic text sync protocol:

- Local `.agent-mem/mem.db` stays in `.gitignore` as an ultra-fast sub-millisecond local cache.
- Project conventions are version-controlled in `.agent-rules` as clean, PR-reviewable plain text.
- `agent-mem set`, `del`, `archive`, and `unarchive` automatically update `.agent-rules` in real time.
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

### Surgical MCP Tools (<160 tokens total schema)
- `mem_set`: Store or update rules with optional repo-relative code anchor and scope (`project` or `global`).
- `mem_find`: Search memories via BM25 keywords across `project`, `global`, or `all` scopes.
- `mem_context`: Retrieve dense context block formatted for immediate LLM injection.
