use agent_mem::store::{BatchRule, Store};
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

    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}
