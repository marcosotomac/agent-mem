# agent-mem Benchmark & Competitor Comparison

> Automated reproducible benchmark executed on v1.0.0

## Executive Summary

`agent-mem` delivers **sub-millisecond latency** and **extreme token efficiency** via embedded SQLite with Memory-Mapped I/O (`PRAGMA mmap_size`), clustered B-tree indexes (`WITHOUT ROWID`), and BM25 full-text search.

## Benchmark Results

- **Point Lookup (p50):** 1.38 µs
- **Point Lookup (p99):** 2.17 µs
- **BM25 FTS5 Search (p50):** 294.75 µs
- **BM25 FTS5 Search (p99):** 1.18 ms
- **Graph 1-Hop Traversal (p50):** 1.00 µs
- **MCP Tool Schema:** 982 characters (~245 tokens)
- **Anchor Context Reduction:** 99.0%

## Competitive Comparison

| Metric | agent-mem | agentmemory | mem0 | Static (CLAUDE.md) |
|---|---|---|---|---|
| **Point Lookup Latency** | **1.38 µs** | ~14 ms | ~150 ms | N/A |
| **BM25 Search Latency** | **294.75 µs** | ~14 ms | N/A (vector) | ~5 ms (grep) |
| **Graph 1-Hop Traversal** | **1.00 µs** | ~25 ms | ~200 ms | N/A |
| **MCP Schema Overhead** | **982 chars (~245 tokens)** | 54 tools (~5,000 tok) | ~3,500 tok | 0 tok |
| **Anchor Token Savings** | **99.0% reduction** | 0% (dump/vector) | 0% | N/A |
| **Architecture** | **Single static binary (2.5MB)** | Node.js + iii daemon + 4 ports | Python + Docker + Postgres | Static file |
| **Runtime Memory (RSS)** | **~3 MB** | ~250 MB | ~500 MB+ | 0 MB |
| **Daemon Requirement** | **Zero daemons** | Pinned iii background engine | Docker / Python server | None |
| **Git / Team Sync** | **Native `.agent-rules` (union merge)** | None (local state only) | Cloud / API only | Manual git merge |

## How to Reproduce

```bash
cargo bench
```
