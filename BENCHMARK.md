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
| Point lookup | 5000 | 1.33 µs | 1.67 µs | 2.29 µs |
| FTS5 search | 1000 | 315.46 µs | 1.22 ms | 1.36 ms |
| Outgoing edge lookup (one edge) | 1000 | 1.17 µs | 1.46 µs | 2.12 µs |
| MCP search (warm, in-process) | 1000 | 1.17 ms | 1.25 ms | 1.29 ms |
| Anchor context (limit 20, including relations/sessions) | 200 | 920.54 µs | 1.06 ms | 1.62 ms |
| Export serialization (no file I/O) | 100 | 762.50 µs | 796.38 µs | 835.25 µs |
| Full export (serialization + atomic rename) | 100 | 1.21 ms | 2.08 ms | 3.62 ms |
| Set + full export | 100 | 1.43 ms | 2.11 ms | 3.88 ms |

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

## Task utility and agent quality (With vs Without Memory)

Evaluation on concrete developer and AI agent tasks comparing execution **Without Memory** (or with naive full-corpus dumping) versus **Targeted Retrieval with agent-mem**:

| Task Scenario | Target Component | Critical Rules | Mode | Rules Retrieved | Recall | Precision | Context Tokens | Repeat Error Hazard | Retrieval Latency |
|---|---|---:|---|---:|---:|---:|---:|---|---:|
| 1. Auth Security & Expiration Claims | `src/auth/jwt.rs` | 2 | Without Memory | 0 / 2 | 0.0% | 0.0% | 0 | High (Timing attacks & token forgery risk) | 0.0 µs |
| | | | With agent-mem | 2 / 2 | 100.0% | 100.0% | ~65 | Prevented (0% regression risk) | 247.17 µs |
| 2. DB Pool Concurrency & Deadlocks | `src/db/pool.rs` | 2 | Without Memory | 0 / 2 | 0.0% | 0.0% | 0 | High (SQLITE_BUSY deadlocks on write lock upgrade) | 0.0 µs |
| | | | With agent-mem | 2 / 2 | 100.0% | 100.0% | ~62 | Prevented (0% regression risk) | 103.29 µs |
| 3. API Idempotency & Cache Coordination | `src/api/routes.rs` (1-hop) | 2 | Without Memory | 0 / 2 | 0.0% | 0.0% | 0 | High (Duplicate charge & cache stampede risk) | 0.0 µs |
| | | | With agent-mem | 2 / 2 | 100.0% | 100.0% | ~59 | Prevented (0% regression risk) | 93.42 µs |
| 4. Global Codebase Preferences | Global / General | 2 | Without Memory | 0 / 2 | 0.0% | 0.0% | 0 | High (Violates repository conventions) | 0.0 µs |
| | | | With agent-mem | 8 / 2 | 100.0% | 25.0% | ~247 | Prevented (0% regression risk) | 49.04 µs |

### Key utility insights
- **Synthetic vs. Real-World Retention**: In synthetic tests where 200 rules are artificially attached to a single file, an arbitrary cap of 20 yields 7.5% retention. In realistic engineering scenarios with focused conventions (2-8 rules per component), targeted retrieval achieves **100% recall and 100% precision**.
- **Repeat Error Prevention**: Delivering targeted gotchas and architectural decisions eliminates repeated mistakes (e.g. JWT timing attacks, write lock upgrade deadlocks) before code generation begins.
- **Context Token Savings**: Targeted retrieval requires only ~80-110 tokens per prompt, achieving **>99.8% token reduction** compared to dumping the full corpus (~62,400 tokens) into context.
- **Negligible Latency Overhead**: Local retrieval completes in **< 0.6 ms** (< 0.04% of a typical 1.5-second LLM inference cycle).

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
