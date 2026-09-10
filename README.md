# agent-mem

Ultra-fast, local-first, zero-markdown key-value memory engine for AI coding agents.

## Design Principles
- **Sub-millisecond latency**: SQLite embedded in WAL mode with clustered B-tree index (`WITHOUT ROWID`).
- **Memory-Mapped I/O**: Direct virtual page reads from kernel page cache (`PRAGMA mmap_size`).
- **Zero DDL overhead on read**: Bypasses schema checks when the database already exists.
- **Minimal token footprint**: Raw plain text output with zero markdown formatting tokens.
- **Universal Project Root Traversal**: Automatically ascends directory tree to find `.git` or `.agent-mem`.

## Commands
```bash
# Initialize isolated project memory (SQLite WAL, .gitignore, git hook)
agent-mem init

# Store or update an architectural rule
agent-mem set <key> <value>

# Read a rule value in raw text (<0.2ms query)
agent-mem get <key>

# Delete a rule
agent-mem del <key>

# List all rules
agent-mem dump

# Fast keyword / BM25 search
agent-mem find <query>

# Session handoff for next agents
agent-mem session add <summary>
agent-mem session list

# Ultra-dense context block for prompt injection
agent-mem context
```
