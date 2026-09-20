use agent_mem::store::{BatchRule, Store};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

#[test]
#[ignore = "100k-record release benchmark; run explicitly with --ignored"]
fn indexed_context_remains_bounded_at_100k_records() {
    let dir = std::env::temp_dir().join(format!("agent_mem_indexed_scale_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let mut store = Store::open(&db_path, true).unwrap();

    let total = std::env::var("AGENT_MEM_SCALE_RECORDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100_000usize);
    assert!(total >= 1_000);
    let seed_started = Instant::now();
    let owned: Vec<(String, String, String)> = (0..total)
        .map(|index| {
            let service = index / 1_000;
            let module = (index / 10) % 100;
            let key = format!("service-{service:03}/module-{module:02}/rule-{index:06}");
            let value = format!(
                "Operational convention {index} for retries, validation, observability, and safe deployment"
            );
            let anchor = format!(
                "services/service-{service:03}/src/domain/module-{module:02}.rs:{}",
                index % 400 + 1
            );
            (key, value, anchor)
        })
        .collect();
    let full_payload_bytes = owned
        .iter()
        .map(|(key, value, _)| key.len() + value.len())
        .sum::<usize>();
    let batch: Vec<BatchRule<'_>> = owned
        .iter()
        .map(|(key, value, anchor)| BatchRule {
            key,
            val: value,
            anchor: Some(anchor),
            kind: Some("rule"),
            relation: None,
        })
        .collect();
    assert_eq!(store.set_batch(&batch).unwrap(), batch.len());
    let seed_seconds = seed_started.elapsed().as_secs_f64();

    let target_service = ((total - 1) / 1_000).min(73);
    let target = format!("services/service-{target_service:03}/src/domain/module-17.rs");
    let expected_prefix = format!("service-{target_service:03}/module-17/");
    let (warm, _, _) = store
        .context_for_files(std::slice::from_ref(&target), None, 20)
        .unwrap();
    assert_eq!(warm.len(), 20);
    assert!(
        warm.iter()
            .take(10)
            .all(|rule| rule.key.starts_with(&expected_prefix)),
        "all ten exact-path rules must rank ahead of sibling routes"
    );

    let mut timings_us = Vec::with_capacity(1_000);
    let mut selected_payload_bytes = 0usize;
    for _ in 0..1_000 {
        let started = Instant::now();
        let (rules, _, _) = store
            .context_for_files(std::slice::from_ref(&target), None, 20)
            .unwrap();
        timings_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        selected_payload_bytes = rules
            .iter()
            .map(|rule| rule.key.len() + rule.val.len())
            .sum();
    }
    timings_us.sort_by(f64::total_cmp);
    let p50_us = percentile(&timings_us, 50);
    let p95_us = percentile(&timings_us, 95);
    let p99_us = percentile(&timings_us, 99);
    let payload_reduction =
        100.0 * (1.0 - selected_payload_bytes as f64 / full_payload_bytes as f64);
    let db_bytes = std::fs::metadata(&db_path).unwrap().len();

    println!(
        "{total} indexed routes: seed={seed_seconds:.2}s p50={p50_us:.1}us p95={p95_us:.1}us p99={p99_us:.1}us db={db_bytes}B payload_reduction={payload_reduction:.4}%"
    );
    assert!(
        p95_us < 10_000.0,
        "indexed context p95 must stay below 10ms, got {p95_us:.1}us"
    );
    let minimum_reduction = 100.0 * (1.0 - 25.0 / total as f64);
    assert!(
        payload_reduction > minimum_reduction,
        "payload reduction must exceed {minimum_reduction:.4}%"
    );

    let rules_path = dir.join(".agent-rules");
    let export_started = Instant::now();
    store.export_to_file(&rules_path).unwrap();
    let export_seconds = export_started.elapsed().as_secs_f64();
    let export_bytes = std::fs::metadata(&rules_path).unwrap().len();
    assert!(export_bytes < 32 * 1024 * 1024);

    // Exercise independent SQLite connections: two writers, two readers and
    // one exporter all start together. Writers update existing records so the
    // final cardinality is an exact, useful data-loss invariant.
    drop(store);
    let db_path = Arc::new(db_path);
    let rules_path = Arc::new(rules_path);
    let barrier = Arc::new(Barrier::new(5));
    let mut handles = Vec::new();
    for writer in 0..2usize {
        let db_path = Arc::clone(&db_path);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db_path, false).unwrap();
            let owned: Vec<(String, String, String)> = (0..128usize)
                .map(|offset| {
                    let index = writer * 128 + offset;
                    (
                        format!(
                            "service-000/module-{:02}/rule-{index:06}",
                            (index / 10) % 100
                        ),
                        format!("Concurrent verified update from writer {writer}, record {index}"),
                        format!(
                            "services/service-000/src/domain/module-{:02}.rs:{}",
                            (index / 10) % 100,
                            index % 400 + 1
                        ),
                    )
                })
                .collect();
            let batch: Vec<BatchRule<'_>> = owned
                .iter()
                .map(|(key, value, anchor)| BatchRule {
                    key,
                    val: value,
                    anchor: Some(anchor),
                    kind: Some("rule"),
                    relation: None,
                })
                .collect();
            barrier.wait();
            store.set_batch(&batch).unwrap();
        }));
    }
    for _ in 0..2 {
        let db_path = Arc::clone(&db_path);
        let barrier = Arc::clone(&barrier);
        let target = target.clone();
        handles.push(thread::spawn(move || {
            let store = Store::open(&db_path, false).unwrap();
            barrier.wait();
            for _ in 0..25 {
                assert_eq!(
                    store
                        .context_for_files(std::slice::from_ref(&target), None, 20)
                        .unwrap()
                        .0
                        .len(),
                    20
                );
            }
        }));
    }
    {
        let db_path = Arc::clone(&db_path);
        let rules_path = Arc::clone(&rules_path);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let store = Store::open(&db_path, false).unwrap();
            barrier.wait();
            for _ in 0..3 {
                store.export_to_file(&rules_path).unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let store = Store::open(&db_path, false).unwrap();
    assert_eq!(store.dump_all().unwrap().len(), total);
    assert_eq!(
        store
            .get("service-000/module-00/rule-000000")
            .unwrap()
            .as_deref(),
        Some("Concurrent verified update from writer 0, record 0")
    );
    store.export_to_file(&rules_path).unwrap();
    let replica_path = dir.join("replica.db");
    let mut replica = Store::open(&replica_path, true).unwrap();
    replica.sync_with_file(&rules_path).unwrap();
    assert_eq!(replica.dump_all().unwrap().len(), total);
    assert_eq!(
        replica.export_rules_text().unwrap(),
        store.export_rules_text().unwrap()
    );
    let temp_artifacts = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
        .count();
    assert_eq!(temp_artifacts, 0);

    println!(
        "100k durability: export={export_seconds:.2}s export_bytes={export_bytes} concurrent_writers=2 concurrent_readers=2 concurrent_exporters=1 replica_records={total}"
    );

    drop(replica);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}
