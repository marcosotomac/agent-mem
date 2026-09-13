# agent-mem

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
curl -fsSL https://raw.githubusercontent.com/marcosotomaceda/agent-mem/main/install.sh | bash
```

### 2. Zero-install via NPX (All platforms)
Run directly without installing any toolchain:
```bash
npx agent-mem --help
```

### 3. Homebrew
```bash
brew install marcosotomaceda/tap/agent-mem
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

# List all rules
agent-mem dump

# Fast BM25 keyword search
agent-mem find <query>

# Session checkpoints (auto-pruned to latest 20)
agent-mem session add <summary>
agent-mem session list

# Dense context block for prompt injection
agent-mem context

# Start native Model Context Protocol (MCP) stdio server
agent-mem mcp
```

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
