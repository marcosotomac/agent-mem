use agent_mem::mcp::{JsonRpcRequest, McpServer};
use agent_mem::store::Store;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64) * (pct / 100.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn format_us(us: f64) -> String {
    if us < 1.0 {
        format!("{:.2} µs ({:.0} ns)", us, us * 1000.0)
    } else if us < 1000.0 {
        format!("{:.2} µs", us)
    } else {
        format!("{:.2} ms", us / 1000.0)
    }
}

fn main() {
    println!("\n============================================================");
    println!("     agent-mem: Local Memory Benchmark Suite   ");
    println!("============================================================\n");

    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_bench_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).expect("failed to create temp bench dir");
    let db_path = temp_dir.join(".agent-mem").join("mem.db");

    println!("[1/6] Seeding 2,000 realistic engineering rules & knowledge graph...");
    let mut store = Store::open(&db_path, true).expect("failed to open store");

    let topics = [
        "auth", "db", "cache", "api", "infra", "security", "testing", "perf", "queue", "log",
    ];
    let kinds = ["rule", "decision", "gotcha", "pattern"];
    let files = [
        "src/auth/jwt.rs",
        "src/db/pool.rs",
        "src/cache/redis.rs",
        "src/api/routes.rs",
        "src/infra/k8s.rs",
        "src/security/cors.rs",
        "src/tests/e2e.rs",
        "src/perf/mmap.rs",
        "src/queue/kafka.rs",
        "src/log/tracing.rs",
    ];

    let seed_start = Instant::now();
    for i in 0..2000 {
        let topic = topics[i % topics.len()];
        let kind = kinds[i % kinds.len()];
        let file = files[i % files.len()];
        let key = format!("{}/{}_{:04}", topic, kind, i);
        let val = format!(
            "Production convention #{}: Always enforce zero-allocation buffers, WAL mode, and strict isolation for {}",
            i, topic
        );
        let anchor = format!("{}:{}", file, (i % 200) + 1);

        store
            .set_entry(&key, &val, Some(&anchor), Some(kind))
            .expect("seed set");

        // Link with prior node to create graph topology
        if i > 0 && i % 3 == 0 {
            let prev_key = format!(
                "{}/{}_{:04}",
                topics[(i - 1) % topics.len()],
                kinds[(i - 1) % kinds.len()],
                i - 1
            );
            let rel = match i % 4 {
                0 => "relates_to",
                1 => "mitigates",
                2 => "depends_on",
                _ => "supersedes",
            };
            assert!(store.get_entry(&prev_key).expect("target lookup").is_some());
            store.relate(&key, rel, &prev_key).expect("seed relation");
        }
    }
    let seed_duration = seed_start.elapsed();
    println!(
        "      Inserted 2,000 entities + graph edges in {:.2} ms ({:.0} ops/sec)\n",
        seed_duration.as_secs_f64() * 1000.0,
        2000.0 / seed_duration.as_secs_f64()
    );

    // ---------------------------------------------------------
    // Benchmark 1: Point Lookup Latency (Clustered B-tree)
    // ---------------------------------------------------------
    println!("[2/6] Benchmarking Clustered B-Tree Point Lookups (5,000 iterations)...");
    let mut lookup_times = Vec::with_capacity(5000);
    for i in 0..5000 {
        let topic = topics[i % topics.len()];
        let kind = kinds[i % kinds.len()];
        let key = format!("{}/{}_{:04}", topic, kind, i % 2000);

        let t0 = Instant::now();
        let record = store.get_entry(&key).expect("lookup");
        let elapsed = t0.elapsed().as_secs_f64() * 1_000_000.0; // microseconds
        lookup_times.push(elapsed);
        assert!(record.is_some());
    }
    lookup_times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let lookup_min = lookup_times[0];
    let lookup_p50 = percentile(&lookup_times, 50.0);
    let lookup_p95 = percentile(&lookup_times, 95.0);
    let lookup_p99 = percentile(&lookup_times, 99.0);
    let lookup_max = lookup_times[lookup_times.len() - 1];
    let lookup_mean = lookup_times.iter().sum::<f64>() / (lookup_times.len() as f64);
    let lookup_ops = 1_000_000.0 / lookup_mean;

    println!("      Min:    {}", format_us(lookup_min));
    println!("      p50:    {}", format_us(lookup_p50));
    println!("      p95:    {}", format_us(lookup_p95));
    println!("      p99:    {}", format_us(lookup_p99));
    println!("      Max:    {}", format_us(lookup_max));
    println!("      Mean:   {}", format_us(lookup_mean));
    println!("      Throughput: {:.0} lookups/sec\n", lookup_ops);

    // ---------------------------------------------------------
    // Benchmark 2: Full-Text Search (FTS5 BM25)
    // ---------------------------------------------------------
    println!("[3/6] Benchmarking FTS5 BM25 Search (1,000 iterations)...");
    let search_queries = [
        "zero-allocation buffers",
        "WAL mode isolation",
        "production convention auth",
        "strict isolation security",
        "redis cache zero-allocation",
    ];
    let mut fts_times = Vec::with_capacity(1000);
    for i in 0..1000 {
        let query = search_queries[i % search_queries.len()];
        let t0 = Instant::now();
        let hits = store.find(query).expect("fts find");
        let elapsed = t0.elapsed().as_secs_f64() * 1_000_000.0;
        fts_times.push(elapsed);
        assert!(!hits.is_empty());
    }
    fts_times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let fts_min = fts_times[0];
    let fts_p50 = percentile(&fts_times, 50.0);
    let fts_p95 = percentile(&fts_times, 95.0);
    let fts_p99 = percentile(&fts_times, 99.0);
    let fts_mean = fts_times.iter().sum::<f64>() / (fts_times.len() as f64);
    let fts_ops = 1_000_000.0 / fts_mean;

    println!("      Min:    {}", format_us(fts_min));
    println!("      p50:    {}", format_us(fts_p50));
    println!("      p95:    {}", format_us(fts_p95));
    println!("      p99:    {}", format_us(fts_p99));
    println!("      Mean:   {}", format_us(fts_mean));
    println!("      Throughput: {:.0} queries/sec\n", fts_ops);

    // ---------------------------------------------------------
    // Benchmark 3: Indexed outgoing edge lookup (nonempty results)
    // ---------------------------------------------------------
    println!("[4/6] Benchmarking Outgoing Edge Lookup (1,000 iterations)...");
    let mut graph_times = Vec::with_capacity(1000);
    for i in 0..1000 {
        let index = (i % 666 + 1) * 3;
        let key = format!(
            "{}/{}_{:04}",
            topics[index % topics.len()],
            kinds[index % kinds.len()],
            index
        );
        let t0 = Instant::now();
        let relations = store.get_relations(&key).expect("edge lookup");
        let elapsed = t0.elapsed().as_secs_f64() * 1_000_000.0;
        graph_times.push(elapsed);
        assert_eq!(relations.len(), 1);
    }
    graph_times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let graph_p50 = percentile(&graph_times, 50.0);
    let graph_p95 = percentile(&graph_times, 95.0);
    let graph_p99 = percentile(&graph_times, 99.0);
    let graph_mean = graph_times.iter().sum::<f64>() / (graph_times.len() as f64);
    let graph_ops = 1_000_000.0 / graph_mean;

    println!("      p50:    {}", format_us(graph_p50));
    println!("      p95:    {}", format_us(graph_p95));
    println!("      p99:    {}", format_us(graph_p99));
    println!("      Mean:   {}", format_us(graph_mean));
    println!("      Throughput: {:.0} traversals/sec\n", graph_ops);

    // ---------------------------------------------------------
    // Benchmark 4: Warm in-process MCP dispatch; excludes stdio and process startup.
    // ---------------------------------------------------------
    let mcp = McpServer::with_paths(temp_dir.clone(), temp_dir.join("global.db"));
    println!("[5/6] Benchmarking MCP mem_find Dispatch (1,000 iterations)...");
    let find_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_find",
            "arguments": { "query": "zero-allocation buffers", "scope": "project" }
        }),
    };
    // Warm up connection and prepared statement caches outside the timed loop.
    mcp.handle_request(&find_req).expect("warmup");
    let mut mcp_times = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let t0 = Instant::now();
        let response = mcp.handle_request(&find_req).expect("mcp response");
        let elapsed = t0.elapsed().as_secs_f64() * 1_000_000.0;
        mcp_times.push(elapsed);
        assert!(response.error.is_none());
        let result = response.result.expect("tool result");
        assert_ne!(result.get("isError").and_then(|v| v.as_bool()), Some(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("search text")
                .contains("Production convention")
        );
    }
    mcp_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mcp_p50 = percentile(&mcp_times, 50.0);
    let mcp_p95 = percentile(&mcp_times, 95.0);
    let mcp_p99 = percentile(&mcp_times, 99.0);
    println!("      p50:    {}", format_us(mcp_p50));
    println!("      p95:    {}", format_us(mcp_p95));
    println!("      p99:    {}\n", format_us(mcp_p99));

    // ---------------------------------------------------------
    // Benchmark 5: Token Budget & Context Filtering Efficiency
    // ---------------------------------------------------------
    println!("[6/6] Auditing Token Budget & Context Reduction...");
    let list_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/list".into(),
        params: json!({}),
    };
    let resp = mcp.handle_request(&list_req).expect("mcp response");
    let schema_chars = serde_json::to_string(&resp.result).expect("json").len();
    let schema_tokens = schema_chars / 4; // ~4 chars per token rule of thumb

    // Build the relevance oracle independently of the retrieval query: exact file
    // matches plus the seeded outgoing one-hop neighbors, with no result cap.
    let all_rules = store.dump_all().expect("full rules");
    let direct: BTreeSet<_> = all_rules
        .iter()
        .filter(|r| r.anchor.as_deref().and_then(|a| a.split(':').next()) == Some(files[0]))
        .map(|r| r.key.clone())
        .collect();
    let mut expected = direct.clone();
    for rel in store.get_all_relations().expect("all relations") {
        if direct.contains(&rel.source_key) {
            expected.insert(rel.target_key);
        }
    }
    let (anchor_rules, _, _) = store
        .context_filtered(Some(files[0]), None, all_rules.len())
        .expect("untruncated anchor context");
    let actual: BTreeSet<_> = anchor_rules.iter().map(|r| r.key.clone()).collect();
    assert_eq!(actual, expected, "filtering must retain all relevant rules");
    let (capped_rules, _, _) = store
        .context_filtered(Some(files[0]), None, 20)
        .expect("capped anchor context");
    let rule_bytes = |rules: &[agent_mem::store::RuleRecord]| -> usize {
        rules.iter().map(|r| r.key.len() + r.val.len()).sum()
    };
    let full_bytes = rule_bytes(&all_rules);
    let filtered_bytes = rule_bytes(&anchor_rules);
    let capped_bytes = rule_bytes(&capped_rules);
    let filtering_reduction = 100.0 * (1.0 - filtered_bytes as f64 / full_bytes as f64);
    let capped_reduction = 100.0 * (1.0 - capped_bytes as f64 / full_bytes as f64);
    let capped_recall = 100.0 * capped_rules.len() as f64 / expected.len() as f64;
    println!(
        "      Relevant rules: {} ({} directly anchored)",
        expected.len(),
        direct.len()
    );
    println!(
        "      Filtering only: {filtering_reduction:.1}% fewer key/value bytes; 100% relevant rules retained"
    );
    println!(
        "      With limit=20: {capped_reduction:.1}% fewer bytes; {capped_recall:.1}% relevant rules retained"
    );

    // Measure the user-facing selective read and write/export paths as well as
    // database primitives. Same corpus and warm connections for every iteration.
    let mut context_times = Vec::with_capacity(200);
    let mut serialization_times = Vec::with_capacity(100);
    let mut export_times = Vec::with_capacity(100);
    let mut write_times = Vec::with_capacity(100);
    let rules_path = temp_dir.join(".agent-rules");
    for _ in 0..200 {
        let start = Instant::now();
        let (rules, _, _) = store
            .context_filtered(Some(files[0]), None, 20)
            .expect("context");
        context_times.push(start.elapsed().as_secs_f64() * 1_000_000.0);
        assert_eq!(rules.len(), 20);
    }
    for _ in 0..100 {
        let start = Instant::now();
        let text = store.export_rules_text().expect("serialize");
        serialization_times.push(start.elapsed().as_secs_f64() * 1_000_000.0);
        assert!(text.contains("auth/rule_0000"));
    }
    for _ in 0..100 {
        let start = Instant::now();
        store.export_to_file(&rules_path).expect("export");
        export_times.push(start.elapsed().as_secs_f64() * 1_000_000.0);
    }
    for i in 0..100 {
        let start = Instant::now();
        store
            .set_entry(
                "bench/write",
                if i % 2 == 0 {
                    "First line\nSecond line"
                } else {
                    "Deploy @ staging"
                },
                None,
                None,
            )
            .expect("write");
        store.export_to_file(&rules_path).expect("write export");
        write_times.push(start.elapsed().as_secs_f64() * 1_000_000.0);
    }
    context_times.sort_by(f64::total_cmp);
    serialization_times.sort_by(f64::total_cmp);
    export_times.sort_by(f64::total_cmp);
    write_times.sort_by(f64::total_cmp);

    let mut table =
        String::from("| Operation | Samples | p50 | p95 | p99 |\n|---|---:|---:|---:|---:|\n");
    for (label, times) in [
        ("Point lookup", &lookup_times),
        ("FTS5 search", &fts_times),
        ("Outgoing edge lookup (one edge)", &graph_times),
        ("MCP search (warm, in-process)", &mcp_times),
        (
            "Anchor context (limit 20, including relations/sessions)",
            &context_times,
        ),
        ("Export serialization (no file I/O)", &serialization_times),
        ("Full export (serialization + atomic rename)", &export_times),
        ("Set + full export", &write_times),
    ] {
        table.push_str(&format!(
            "| {label} | {} | {} | {} | {} |\n",
            times.len(),
            format_us(percentile(times, 50.0)),
            format_us(percentile(times, 95.0)),
            format_us(percentile(times, 99.0))
        ));
    }
    let profile = if cfg!(debug_assertions) {
        "debug (not a release performance result)"
    } else {
        "release/bench"
    };
    let report = format!(
        r#"# agent-mem benchmark

Version: {}. Platform: {}/{}. Profile: {}.

## Method

Synthetic corpus: 2,000 memories across 10 files and 666 directed edges with existing endpoints.
Point reads assert a hit; edge lookups assert one edge; FTS and MCP calls assert successful, nonempty results.
MCP is measured in-process after warmup, excluding process startup and stdio transport.
Export includes serialization and atomic file replacement; writes use SQLite WAL with synchronous=NORMAL.
These measurements describe this machine and workload, not a universal latency guarantee.

{}
## Context efficiency and retrieval quality

Payload metric: UTF-8 bytes of rule keys and values only; excludes formatting, anchors, relations and sessions.
This is not a tokenizer measurement. MCP schema: {} bytes (~{} tokens using bytes/4).

| Selection | Rules returned | Key/value bytes | Reduction vs full corpus | Relevant rules retained |
|---|---:|---:|---:|---:|
| Full corpus | {} | {} | 0% | 100% |
| Anchor + outgoing one hop, without truncation | {} | {} | {:.1}% | 100% |
| Same filter, limit 20 | {} | {} | {:.1}% | {:.1}% |

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
"#,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        profile,
        table,
        schema_chars,
        schema_tokens,
        all_rules.len(),
        full_bytes,
        anchor_rules.len(),
        filtered_bytes,
        filtering_reduction,
        capped_rules.len(),
        capped_bytes,
        capped_reduction,
        capped_recall
    );
    println!("\n{report}");
    let report_path = std::env::var_os("AGENT_MEM_BENCH_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/benchmark.md"));
    if let Some(parent) = report_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).expect("report directory");
    }
    fs::write(&report_path, report).expect("write benchmark report");
    println!("Benchmark report generated at {}", report_path.display());
    fs::remove_dir_all(&temp_dir).expect("remove benchmark fixture");
}
