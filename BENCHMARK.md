# agent-mem benchmark

Version: 1.3.0. Platform: macos/aarch64. Profile: release/bench.

## Method

Synthetic corpus: 2,000 memories across 10 files and 666 directed edges with existing endpoints.
Point reads assert a hit; edge lookups assert one edge; FTS and MCP calls assert successful, nonempty results.
MCP is measured in-process after warmup, excluding process startup and stdio transport.
Export includes serialization and atomic file replacement; writes use SQLite WAL with synchronous=NORMAL.
These measurements describe this machine and workload, not a universal latency guarantee.

| Operation | Samples | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Point lookup | 5000 | 1.29 µs | 1.42 µs | 2.08 µs |
| FTS5 search | 1000 | 315.42 µs | 1.27 ms | 2.31 ms |
| Outgoing edge lookup (one edge) | 1000 | 1.21 µs | 1.33 µs | 1.96 µs |
| MCP search (warm, in-process) | 1000 | 1.15 ms | 1.26 ms | 1.73 ms |
| Anchor context (limit 20, including relations/sessions) | 200 | 398.96 µs | 410.42 µs | 416.25 µs |
| Export serialization (no file I/O) | 100 | 774.83 µs | 814.62 µs | 837.00 µs |
| Full export (serialization + atomic rename) | 100 | 1.13 ms | 1.57 ms | 1.83 ms |
| Set + full export | 100 | 1.47 ms | 1.95 ms | 3.39 ms |

## Context efficiency and retrieval quality

Payload metric: UTF-8 bytes of rule keys and values only; excludes formatting, anchors, relations and sessions.
This is not a tokenizer measurement. MCP schema: 1366 bytes (~341 tokens using bytes/4).

| Selection | Rules returned | Key/value bytes | Reduction vs full corpus | Relevant rules retained |
|---|---:|---:|---:|---:|
| Full corpus | 2000 | 249790 | 0% | 100% |
| Anchor + outgoing one hop, without truncation | 266 | 32604 | 86.9% | 100% |
| Same filter, limit 20 | 20 | 2453 | 99.0% | 7.5% |

The untruncated result is checked against an independent set of exact file matches and their outgoing neighbors.
Savings from the result cap must not be attributed to filtering; the cap can omit relevant rules.

### Indexed routing at 100,000 memories

The separate ignored release-scale gate seeds 100,000 anchored memories through one atomic batch,
checks that all exact-path rules outrank sibling candidates, and runs 1,000 selective context calls.
On the same macOS/arm64 machine, schema v7 measured 3.19 s initial ingest, 1.767 ms p50,
1.853 ms p95, 2.109 ms p99, a 123,539,456-byte database, and 99.9800% fewer key/value
bytes than the full corpus. A full 100,000-record export took 0.07 s and produced
17,762,027 bytes. Two writers, two readers, and one exporter then ran concurrently;
the exported replica retained all 100,000 records and left no temporary artifacts.
These figures describe this fixture and machine.
The slower initial ingest reflects the per-write generation and revision triggers used to
invalidate stale sync fingerprints and preserve local history.

The fixed 20-slot graph-context test contains 40 exact-path distractors, five
decision-changing related memories, and two repository-wide rules. Direct-only
packing retained 0/7 declared qrels; diversity reservation retains 7/7 under the
same 20-record budget. This controlled qrel result does not imply end-to-end task success.

The deterministic noisy-query eval contains 20 fixed queries, 10 independently declared qrels,
and 600 vocabulary-overlapping distractors. It gates Recall@5, mean reciprocal rank, and returned
payload size rather than relying on anecdotal examples.

## End-to-end agent evaluation

This microbenchmark does not claim improvements in agent success, token use, or
repeated-error rate. Those outcomes require model executions on real tasks. The
reproducible matrix and its evidence contract live in [`evals/`](evals/README.md);
comparisons are published only from retained run artifacts.

## Comparisons

No competitor latency, memory usage or token estimates are reported. A comparative benchmark must first
use the same corpus, queries, transport, limits and correctness checks for both engines.

## Reproduce

```bash
cargo bench --bench bench_suite
cargo test --release --test indexed_routing_scale_test -- --ignored --nocapture
cargo test --test retrieval_quality_test -- --nocapture
# Explicitly refresh the tracked report:
AGENT_MEM_BENCH_REPORT=BENCHMARK.md cargo bench --bench bench_suite
```

The default report goes to target/benchmark.md, so tests do not rewrite tracked documentation.
