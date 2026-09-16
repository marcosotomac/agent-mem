# agent-mem benchmark

Version: 1.2.0. Platform: macos/aarch64. Profile: release/bench.

## Method

Synthetic corpus: 2,000 memories across 10 files and 666 directed edges with existing endpoints.
Point reads assert a hit; edge lookups assert one edge; FTS and MCP calls assert successful, nonempty results.
MCP is measured in-process after warmup, excluding process startup and stdio transport.
Export includes serialization and atomic file replacement; writes use SQLite WAL with synchronous=NORMAL.
These measurements describe this machine and workload, not a universal latency guarantee.

| Operation | Samples | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Point lookup | 5000 | 1.29 µs | 1.42 µs | 2.08 µs |
| FTS5 search | 1000 | 303.62 µs | 1.15 ms | 1.18 ms |
| Outgoing edge lookup (one edge) | 1000 | 1.17 µs | 1.33 µs | 1.79 µs |
| MCP search (warm, in-process) | 1000 | 1.14 ms | 1.18 ms | 1.20 ms |
| Anchor context (limit 20, including relations/sessions) | 200 | 542.96 µs | 566.58 µs | 580.08 µs |
| Export serialization (no file I/O) | 100 | 772.21 µs | 799.29 µs | 918.79 µs |
| Full export (serialization + atomic rename) | 100 | 1.21 ms | 2.10 ms | 2.55 ms |
| Set + full export | 100 | 1.29 ms | 1.69 ms | 3.25 ms |

## Context efficiency and retrieval quality

Payload metric: UTF-8 bytes of rule keys and values only; excludes formatting, anchors, relations and sessions.
This is not a tokenizer measurement. MCP schema: 1376 bytes (~344 tokens using bytes/4).

| Selection | Rules returned | Key/value bytes | Reduction vs full corpus | Relevant rules retained |
|---|---:|---:|---:|---:|
| Full corpus | 2000 | 249790 | 0% | 100% |
| Anchor + outgoing one hop, without truncation | 266 | 32604 | 86.9% | 100% |
| Same filter, limit 20 | 20 | 2455 | 99.0% | 7.5% |

The untruncated result is checked against an independent set of exact file matches and their outgoing neighbors.
Savings from the result cap must not be attributed to filtering; the cap can omit relevant rules.

## Comparisons

No competitor latency, memory usage or token estimates are reported. A comparative benchmark must first
use the same corpus, queries, transport, limits and correctness checks for both engines.

## Reproduce

```bash
cargo bench --bench bench_suite
# Explicitly refresh the tracked report:
AGENT_MEM_BENCH_REPORT=BENCHMARK.md cargo bench --bench bench_suite
```

The default report goes to target/benchmark.md, so tests do not rewrite tracked documentation.
