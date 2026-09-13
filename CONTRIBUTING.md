# Contributing to agent-mem

Thank you for contributing to `agent-mem`! We aim to build the fastest, most resilient, and token-frugal memory engine for autonomous AI coding agents.

## Core Principles

1. **Sub-millisecond Latency**: Every SQLite query must execute in < 1ms using WAL mode and PRAGMA mmap_size.
2. **Minimal Token Footprint**: Context and MCP tool schemas must remain minimal (under 160 tokens). Never add conversational bloat to context payloads.
3. **Rock-solid Concurrency**: Safe concurrent access across threads and multi-process agents without database lockups.
4. **Git-Centric Team Sync**: Human-readable `.agent-rules` file synchronized automatically via Git hooks and union merge.

## Branch Strategy

- `main`: Production-ready, tagged releases.
- `dev`: Active development branch. All feature branches and pull requests should target `dev`.

## Development Workflow

1. Clone the repository and branch from `dev`:
   ```bash
   git checkout dev
   git checkout -b feat/your-feature
   ```

2. Verify formatting and linter before submitting:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   ```

3. Run the full test suite:
   ```bash
   cargo test
   ```

4. Use [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat: ...`
   - `fix: ...`
   - `chore: ...`
   - `test: ...`
   - `docs: ...`
   *Note: Do not add AI attribution or Co-Authored-By tags to commits.*

5. Open a Pull Request targeting the `dev` branch.
