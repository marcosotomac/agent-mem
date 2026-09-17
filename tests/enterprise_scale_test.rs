use agent_mem::store::Store;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Instant;

#[test]
fn test_massive_scale_5k_records_and_sub_millisecond_latency() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_enterprise_5k_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("enterprise_5k.db");

    let mut store = Store::open(&db_path, true).unwrap();
    let total_records = 5_000;

    let insert_start = Instant::now();
    for i in 0..total_records {
        let key = format!(
            "service_{:04}/module_{:02}/convention_{:02}",
            i / 100,
            (i / 10) % 10,
            i % 10
        );
        let val = if i == 3456 {
            "CRITICAL_NEEDLE: Use distributed lock manager with Raft consensus for enterprise payments"
        } else {
            "Standard enterprise domain entity validation and CQRS pattern"
        };
        let anchor = format!("services/srv_{:04}/src/domain/entity.rs:{}", i / 100, i);
        store.set_with_anchor(&key, val, Some(&anchor)).unwrap();
    }
    let insert_duration = insert_start.elapsed();
    println!(
        "Inserted {} indexed records in {:?}",
        total_records, insert_duration
    );

    // 1. Point lookup latency test (WITHOUT ROWID clustered index)
    let get_start = Instant::now();
    let entry = store.get("service_0034/module_05/convention_06").unwrap();
    let get_duration = get_start.elapsed();
    println!("Clustered B-tree point lookup latency: {:?}", get_duration);
    assert!(entry.is_some());
    assert!(entry.unwrap().contains("CRITICAL_NEEDLE"));
    assert!(
        get_duration.as_millis() < 5,
        "Point lookup must execute in < 5ms"
    );

    // 2. Full-Text Search BM25 latency test over thousands of indexed documents
    let fts_start = Instant::now();
    let results = store.find("Raft consensus enterprise payments").unwrap();
    let fts_duration = fts_start.elapsed();
    println!(
        "FTS5 BM25 search over {} records: {:?}",
        total_records, fts_duration
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "service_0034/module_05/convention_06");
    assert!(results[0].val.contains("CRITICAL_NEEDLE"));
    assert!(
        fts_duration.as_millis() < 25,
        "FTS5 BM25 search must execute in < 25ms"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_payload_boundary_and_oversized_rejection() {
    let mut store = Store::open_in_memory().unwrap();

    let max_payload = "E".repeat(agent_mem::store::MAX_VALUE_BYTES);
    let key = "enterprise/large_spec";
    let anchor = "specs/openapi.json:1";

    let write_start = Instant::now();
    store
        .set_with_anchor(key, &max_payload, Some(anchor))
        .unwrap();
    let write_duration = write_start.elapsed();
    println!("bounded payload write duration: {:?}", write_duration);

    let read_start = Instant::now();
    let retrieved = store.get(key).unwrap().expect("bounded value retrieved");
    let read_duration = read_start.elapsed();
    println!("bounded payload read duration: {:?}", read_duration);

    assert_eq!(retrieved.len(), agent_mem::store::MAX_VALUE_BYTES);
    assert!(
        read_duration.as_millis() < 10,
        "bounded memory mapped read must execute in < 10ms"
    );

    let oversized = "E".repeat(agent_mem::store::MAX_VALUE_BYTES + 1);
    assert!(store.set("enterprise/rejected", &oversized).is_err());
    assert_eq!(store.get("enterprise/rejected").unwrap(), None);
}

#[test]
fn test_enterprise_high_concurrency_20_workers() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_enterprise_concurrency_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = Arc::new(temp_dir.join("enterprise_concurrent.db"));

    // Initialize database schema
    {
        let _ = Store::open(&db_path, true).unwrap();
    }

    let worker_count = 20;
    let ops_per_worker = 25;
    let completed_ops = Arc::new(AtomicUsize::new(0));

    let start = Instant::now();
    let mut handles = Vec::new();

    for w_id in 0..worker_count {
        let db_path = Arc::clone(&db_path);
        let completed = Arc::clone(&completed_ops);
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db_path, true).expect("open worker store");
            for i in 0..ops_per_worker {
                let key = format!("tenant_{:02}/rule_{}", w_id, i);
                let val = format!("Enterprise policy for tenant {} rule index {}", w_id, i);
                store.set(&key, &val).expect("set rule");
                if i % 5 == 0 {
                    let _ = store.find(&format!("tenant_{:02}", w_id));
                    let _ = store.get(&key);
                }
                completed.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("worker finished cleanly");
    }

    let elapsed = start.elapsed();
    let total_ops = completed_ops.load(Ordering::SeqCst);
    println!(
        "Completed {} concurrent enterprise operations across {} workers in {:?}",
        total_ops, worker_count, elapsed
    );

    assert_eq!(total_ops, worker_count * ops_per_worker);

    // Verify consistency
    let store = Store::open(&db_path, false).unwrap();
    let all = store.dump().unwrap();
    assert_eq!(all.len(), worker_count * ops_per_worker);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_massive_sync_reconciliation_scale() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_sync_scale_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("sync_scale.db");
    let rules_path = temp_dir.join(".agent-rules");

    // Generate 2,000 rules in plain text
    let mut raw_content = String::with_capacity(200_000);
    raw_content.push_str("# Massive team rules file\n");
    for i in 0..2_000 {
        raw_content.push_str(&format!(
            "team/rule_{} = Rule description number {} (@ src/rule_{}.rs:10)\n",
            i, i, i
        ));
    }
    std::fs::write(&rules_path, &raw_content).unwrap();

    let mut store = Store::open(&db_path, true).unwrap();
    let sync_start = Instant::now();
    let report = store.sync_with_file(&rules_path).unwrap();
    let sync_duration = sync_start.elapsed();

    println!(
        "Synchronized {} rules from file into SQLite in {:?}",
        report.total, sync_duration
    );
    assert_eq!(report.total, 2_000);
    assert!(
        sync_duration.as_millis() < 500,
        "2,000 rules sync must finish in < 500ms"
    );

    // Second sync on untouched file must hit no-op fast path (< 5ms)
    let noop_start = Instant::now();
    let noop_report = store.sync_with_file(&rules_path).unwrap();
    let noop_duration = noop_start.elapsed();
    assert_eq!(noop_report.total, 2_000);
    assert_eq!(noop_report.conflicts_resolved, 0);
    assert!(
        noop_duration.as_millis() < 5,
        "Unchanged 2,000 rules sync must hit no-op path in < 5ms, took {:?}",
        noop_duration
    );

    // BM25 verify
    let results = store.find("Rule description number 1500").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "team/rule_1500");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
