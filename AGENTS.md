<!-- agent-mem -->
## Persistent Memory Protocol
- **On start:** Run `agent-mem context --scope all` (or MCP `mem_context`) to load trusted project rules and global preferences.
- **On decisions:** Run `agent-mem set <key> "<rule>"` (or MCP `mem_set`) only for durable ADRs, conventions, patterns, and gotchas.
- **On search:** Run `agent-mem find <query> --scope all` (or MCP `mem_find`) when past decisions may affect the task.
<!-- /agent-mem -->
