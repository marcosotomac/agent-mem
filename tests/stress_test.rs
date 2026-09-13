use agent_mem::store::Store;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Instant;

#[test]
fn test_high_concurrency_wal_stress() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_concurrency_stress_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = Arc::new(temp_dir.join("stress.db"));

    // Pre-initialize store
    {
        let _ = Store::open(&db_path, true).unwrap();
    }

    let write_threads = 10;
    let read_threads = 10;
    let ops_per_thread = 50;

    let successful_writes = Arc::new(AtomicUsize::new(0));
    let successful_reads = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();

    let start_time = Instant::now();

    // Spawn writer threads
    for t_id in 0..write_threads {
        let db_path = Arc::clone(&db_path);
        let success = Arc::clone(&successful_writes);
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db_path, true).expect("open writer store");
            for i in 0..ops_per_thread {
                let key = format!("worker_{}/rule_{}", t_id, i);
                let val = format!(
                    "Value for worker {} iteration {} with high concurrency payload",
                    t_id, i
                );
                let anchor = format!("src/worker_{}.rs:{}", t_id, i);

                store
                    .set_with_anchor(&key, &val, Some(&anchor))
                    .expect("set memory");
                if i % 10 == 0 {
                    store
                        .session_add(&format!("Worker {} checkpoint {}", t_id, i))
                        .expect("session add");
                }
                success.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    // Spawn reader threads concurrently reading and searching
    for _t_id in 0..read_threads {
        let db_path = Arc::clone(&db_path);
        let success = Arc::clone(&successful_reads);
        handles.push(thread::spawn(move || {
            let store = Store::open(&db_path, false).expect("open reader store");
            for i in 0..ops_per_thread {
                let search_target = format!("worker_{}", i % write_threads);
                let _ = store.find(&search_target);
                let _ = store.get(&format!("worker_{}/rule_{}", i % write_threads, i));
                let _ = store.context();
                success.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread join succeeded without panic");
    }

    let elapsed = start_time.elapsed();
    assert_eq!(
        successful_writes.load(Ordering::SeqCst),
        write_threads * ops_per_thread
    );
    assert_eq!(
        successful_reads.load(Ordering::SeqCst),
        read_threads * ops_per_thread
    );

    // Verify consistency
    let store = Store::open(&db_path, false).unwrap();
    let dump = store.dump().unwrap();
    assert_eq!(dump.len(), write_threads * ops_per_thread);

    // Verify sessions ring buffer stayed capped at 20
    let sessions = store.session_list(100).unwrap();
    assert!(sessions.len() <= 20);

    println!(
        "Concurrency stress: completed {} writes and {} reads concurrently in {:?}",
        write_threads * ops_per_thread,
        read_threads * ops_per_thread,
        elapsed
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_bulk_volume_and_fts_bm25_speed() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_volume_stress_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("volume.db");

    let mut store = Store::open(&db_path, true).unwrap();
    let total_records = 1000;

    let insert_start = Instant::now();
    for i in 0..total_records {
        let key = format!("arch/service_{}/component_{}", i / 10, i % 10);
        let val = if i == 777 {
            "Special needle in haystack: Quantum resilience token protocol with zero overhead"
        } else {
            "Standard microservice event bus communication rule using protobuf definitions"
        };
        let anchor = format!("services/srv_{}/src/lib.rs:{}", i / 10, i);
        store.set_with_anchor(&key, val, Some(&anchor)).unwrap();
    }
    let insert_duration = insert_start.elapsed();
    println!(
        "Inserted {} indexed records in {:?}",
        total_records, insert_duration
    );

    // Search needle via FTS5 BM25
    let search_start = Instant::now();
    let results = store.find("Quantum resilience needle").unwrap();
    let search_duration = search_start.elapsed();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "arch/service_77/component_7");
    assert!(results[0].val.contains("Quantum resilience token protocol"));
    assert_eq!(
        results[0].anchor.as_deref(),
        Some("services/srv_77/src/lib.rs:777")
    );

    println!(
        "BM25 search across 1000 records completed in {:?}",
        search_duration
    );
    assert!(search_duration.as_millis() < 50);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_fuzz_unicode_and_extreme_inputs() {
    let mut store = Store::open_in_memory().unwrap();

    let emoji_key = "arquitectura/seguridad/🔑-token";
    let emoji_val = "Usar JWT con clave asimétrica Ed25519 🛡️ y rotación cada 15m ⚡";
    let emoji_anchor = "src/seguridad/autenticación_v2.rs:120";

    store
        .set_with_anchor(emoji_key, emoji_val, Some(emoji_anchor))
        .unwrap();

    assert_eq!(store.get(emoji_key).unwrap().as_deref(), Some(emoji_val));
    let search_emoji = store.find("Ed25519 🛡️").unwrap();
    assert_eq!(search_emoji.len(), 1);
    assert_eq!(search_emoji[0].key, emoji_key);

    let large_val = "A".repeat(64 * 1024);
    store.set("huge_blob", &large_val).unwrap();
    assert_eq!(store.get("huge_blob").unwrap().unwrap().len(), 64 * 1024);

    let dirty_input = format!(
        "# messy comments\n\n\
        // another comment\n\
        malformed_line_without_equals_or_colon\n\
        = empty key should be skipped\n\
        empty_val = \n\
        key_with_equals = value with = internal = signs = and symbols @ src/test.rs:10\n\
        {} = {}\n\
        strange/anchor = value (@ nested (parenthesis) path/to/file.rs:99)\n",
        emoji_key, emoji_val
    );

    let parsed = Store::parse_rules_text(&dirty_input);
    assert!(
        parsed.iter().any(|r| r.key == "key_with_equals"
            && r.val == "value with = internal = signs = and symbols")
    );
    assert!(parsed.iter().any(|r| r.key == emoji_key));
    assert!(parsed.iter().any(|r| r.key == "strange/anchor"
        && r.anchor.as_deref() == Some("nested (parenthesis) path/to/file.rs:99")));
}
