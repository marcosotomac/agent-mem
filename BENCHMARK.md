# agent-mem Benchmark & Competitor Comparison

> Automated reproducible benchmark executed on v1.1.0

## Executive Summary

`agent-mem` delivers **sub-millisecond latency** and **extreme token efficiency** via embedded SQLite with Memory-Mapped I/O (`PRAGMA mmap_size`), clustered B-tree indexes (`WITHOUT ROWID`), and BM25 full-text search.

## Benchmark Results

- **Point Lookup (p50):** 1.33 µs
- **Point Lookup (p99):** 2.33 µs
- **BM25 FTS5 Search (p50):** 293.04 µs
- **BM25 FTS5 Search (p99):** 1.16 ms
- **Graph 1-Hop Traversal (p50):** 1.00 µs
- **MCP `mem_find` Dispatch (p50):** 1.09 ms
- **MCP Tool Schema:** 1376 characters (~344 tokens)
- **Anchor Context Reduction:** 99.0%

## Competitive Comparison

| Metric | agent-mem | engram 1.20.0 | agentmemory | mem0 | Static (CLAUDE.md) |
|---|---|---|---|---|---|
| **Point Lookup Latency** | **1.33 µs** | ~45 µs | ~14 ms | ~150 ms | N/A |
| **BM25 Search Latency** | **293.04 µs** | ~480 µs | ~14 ms | N/A (vector) | ~5 ms (grep) |
| **MCP Search Dispatch** | **1.09 ms** | 125.71 µs | N/A | N/A | N/A |
| **Graph 1-Hop Traversal** | **1.00 µs** | ~120 µs | ~25 ms | ~200 ms | N/A |
| **MCP Schema Overhead** | **1376 chars (~344 tokens)** | 20476 chars (~5119 tokens) | 54 tools (~5,000 tok) | ~3,500 tok | 0 tok |
| **Anchor Token Savings** | **99.0% reduction** | 0% (dump format) | 0% (dump/vector) | 0% | N/A |
| **Architecture** | **Single binary (2.4–2.7 MB)** | Single binary (18.0 MB, Go) | Node.js + iii daemon + 4 ports | Python + Docker + Postgres | Static file |
| **Runtime Memory (RSS)** | **~2 MB** | ~28 MB | ~250 MB | ~500 MB+ | 0 MB |
| **Daemon Requirement** | **Zero daemons** | Zero daemons | Pinned iii background engine | Docker / Python server | None |
| **Git / Team Sync** | **Native `.agent-rules` (union merge)** | Binary chunks / Cloud Sync | None (local state only) | Cloud / API only | Manual git merge |

*agent-mem and engram performance columns are measured directly on your machine when binaries are present; competitor values are historical reference estimates.*

## How to Reproduce

```bash
cargo bench
```
