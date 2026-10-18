<!-- agent-mem -->
## Persistent Memory Protocol
- **Task context:** Once the task is clear, run `agent-mem context --query "<task>" --scope all --limit 8` (or MCP `mem_context` with `query`, `scope: "all"`, and `limit: 8`). Add relevant `--files <paths>` when known; use `--diff` only when current changes concern the task. Use the CLI if the available MCP schema lacks `query`.
- **Past decisions:** Run `agent-mem find "<topic>" --scope all` (or MCP `mem_find` with `scope: "all"`) before changing a convention or architecture decision.
- **Durable capture:** Run `agent-mem set <key> "<rule>"` (or MCP `mem_set`) only for durable ADRs, conventions, patterns, and gotchas; use `--scope global` only for cross-project preferences.
- **Diagnostics:** Use `context --explain` only when checking why relevant memories were included or omitted.
<!-- /agent-mem -->
