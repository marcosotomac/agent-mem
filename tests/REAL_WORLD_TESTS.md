# Real-world validation suite

`real_world_scenarios_test.rs` exercises complete product workflows rather than isolated methods.

| Scenario | Failure it detects | Main invariant |
|---|---|---|
| Clone and conflicting Git branches | Lost shared rules, ambiguous merge values, broken multiline encoding | Default sync rejects same-key contradictions; explicit acceptance preserves a searchable winner and an auditable alternative |
| Corruption and conflict storm | Partial imports, stale FTS rows, unbounded audit history | Failed sync is atomic; explicit conflict acceptance preserves the alternative; sessions stay capped |
| Large monorepo with generic filenames | `mod.rs` basename collisions and ignored topic filters | Exact path and topic matches outrank unrelated services |
| MCP context under large incident payloads | A stack trace starving critical or global rules | Output stays within 16 KiB and retains critical, related, session, and global context |
| Concurrent writers, readers, and exports | WAL lock failures, torn exports, missing relations | Final text snapshot imports into an equivalent replica |
| Batch and no-op performance | Transaction and synchronization regressions | Batch beats individual commits; no-op sync beats full reconciliation |
| Noisy daily-language retrieval | Zero-result strict searches and distractor-heavy ranking | Recall@5, MRR, and returned payload are measured against fixed qrels |
| 100k indexed routing | Linear anchor scans, unbounded candidates, quadratic initial FTS writes | Exact rules rank first; p50/p95/p99 and payload reduction stay bounded |

Run the suite with visible timing evidence:

```bash
cargo test --test real_world_scenarios_test -- --nocapture
cargo test --test retrieval_quality_test -- --nocapture
```

Run the explicit release-scale benchmark (ignored by the normal test suite):

```bash
cargo test --release --test indexed_routing_scale_test -- --ignored --nocapture
```

Run it repeatedly when changing storage, synchronization, context selection, or MCP formatting:

```bash
for run in 1 2 3 4 5; do
  cargo test --test real_world_scenarios_test
done
```

The performance assertions compare equivalent work in the same process. They deliberately avoid fragile universal microsecond limits; release benchmarks remain the source for absolute latency percentiles.
