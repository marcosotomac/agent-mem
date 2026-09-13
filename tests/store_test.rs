use agent_mem::store::Store;

#[test]
fn test_crud_lifecycle() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    // Initially empty
    assert_eq!(store.get("arch").unwrap(), None);

    // Set rule
    store.set("arch", "Clean Architecture with Rust").unwrap();
    assert_eq!(store.get("arch").unwrap().as_deref(), Some("Clean Architecture with Rust"));

    // Upsert rule
    store.set("arch", "Hexagonal Architecture with Rust").unwrap();
    assert_eq!(store.get("arch").unwrap().as_deref(), Some("Hexagonal Architecture with Rust"));

    // Add another key
    store.set("tests", "Always write unit and integration tests").unwrap();

    // Dump
    let dumped = store.dump().unwrap();
    assert_eq!(dumped.len(), 2);
    assert_eq!(dumped[0].0, "arch");
    assert_eq!(dumped[1].0, "tests");

    // Delete
    let deleted = store.del("arch").unwrap();
    assert!(deleted);
    assert_eq!(store.get("arch").unwrap(), None);

    // Delete non-existent
    let deleted_again = store.del("arch").unwrap();
    assert!(!deleted_again);
}

#[test]
fn test_fts_and_fallback() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store.set("db-rule", "Use SQLite in WAL mode for sub-millisecond reads").unwrap();
    store.set("auth-rule", "JWT tokens must be verified with RS256").unwrap();
    store.set("api-rule", "Prefer HTTP/2 streaming over polling").unwrap();

    // Search keyword in value
    let results = store.find("SQLite WAL").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "db-rule");

    // Search keyword in key
    let results_key = store.find("auth").unwrap();
    assert_eq!(results_key.len(), 1);
    assert_eq!(results_key[0].0, "auth-rule");

    // Search with special characters (must not crash FTS5 parser)
    let safe_search = store.find("SQLite: \"WAL\" mode (sub-millisecond)").unwrap();
    assert!(!safe_search.is_empty());
    assert_eq!(safe_search[0].0, "db-rule");

    // Search non-existent
    let empty = store.find("nonexistentquerythatshouldnevermatch").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_no_phantom_results_after_delete() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store.set("ghost-key", "this should disappear completely").unwrap();
    let found = store.find("disappear").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "ghost-key");

    let deleted = store.del("ghost-key").unwrap();
    assert!(deleted);

    // Verify neither get nor find returns the deleted key
    assert_eq!(store.get("ghost-key").unwrap(), None);
    let phantom_search = store.find("disappear").unwrap();
    assert!(phantom_search.is_empty());
}

#[test]
fn test_session_lifecycle() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    let s1 = store.session_add("Initial refactor completed").unwrap();
    let s2 = store.session_add("Added unit tests").unwrap();
    let s3 = store.session_add("Verified performance").unwrap();

    assert!(s1 < s2);
    assert!(s2 < s3);

    let list = store.session_list(2).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].0, s3);
    assert_eq!(list[0].1, "Verified performance");
    assert_eq!(list[1].0, s2);
    assert_eq!(list[1].1, "Added unit tests");
}

#[test]
fn test_context_export() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store.set("convention", "Conventional commits only").unwrap();
    store.session_add("Setup project structure").unwrap();

    let (rules, sessions) = store.context().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].0, "convention");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].1, "Setup project structure");
}

#[test]
fn test_fts_porter_stemming_and_no_wildcard_pollution() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store
        .set("arch-rule", "Clean architectural design patterns")
        .unwrap();
    store.set("wallet-rule", "Keep keys in your secure wallet").unwrap();

    // 1. Porter stemming: "architecture" should stem to match "architectural"
    let stemmed = store.find("architecture").unwrap();
    assert_eq!(stemmed.len(), 1);
    assert_eq!(stemmed[0].0, "arch-rule");

    // 2. Exact token: "wal" must NOT match "wallet" because automatic * wildcard was removed
    let no_wildcard_pollution = store.find("wal").unwrap();
    assert!(no_wildcard_pollution.is_empty());

    // 3. Explicit prefix search: user explicitly requested wildcard "wal*"
    let explicit_wildcard = store.find("wal*").unwrap();
    assert_eq!(explicit_wildcard.len(), 1);
    assert_eq!(explicit_wildcard[0].0, "wallet-rule");
}

#[test]
fn test_zero_byte_database_resilience() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_zero_byte_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");

    // Create a 0-byte file (simulate crash, touch, or power loss)
    std::fs::File::create(&db_path).unwrap();
    assert_eq!(db_path.metadata().unwrap().len(), 0);

    // 1. Read open on 0-byte database must return NotInitialized instead of panicking with "no such table"
    let read_res = Store::open(&db_path, false);
    assert!(matches!(read_res, Err(agent_mem::error::Error::NotInitialized)));

    // 2. Write open on 0-byte database must recover and initialize schema
    let mut write_store = Store::open(&db_path, true).expect("write open should initialize 0-byte db");
    write_store.set("recovered", "schema initialized successfully").unwrap();
    assert_eq!(
        write_store.get("recovered").unwrap().as_deref(),
        Some("schema initialized successfully")
    );

    // 3. Subsequent read open succeeds
    let read_store = Store::open(&db_path, false).expect("read open should now succeed");
    assert_eq!(
        read_store.get("recovered").unwrap().as_deref(),
        Some("schema initialized successfully")
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_anchor_storage_and_search() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store
        .set_with_anchor(
            "jwt-auth",
            "RS256 validation required for all requests",
            Some("src/auth/jwt.rs:42"),
        )
        .unwrap();

    // 1. get_entry returns value and anchor
    let entry = store.get_entry("jwt-auth").unwrap().expect("entry found");
    assert_eq!(entry.0, "RS256 validation required for all requests");
    assert_eq!(entry.1.as_deref(), Some("src/auth/jwt.rs:42"));

    // 2. dump returns anchor
    let all = store.dump().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].2.as_deref(), Some("src/auth/jwt.rs:42"));

    // 3. BM25 / FTS matches on anchor path
    let by_anchor = store.find("auth/jwt.rs").unwrap();
    assert_eq!(by_anchor.len(), 1);
    assert_eq!(by_anchor[0].0, "jwt-auth");
    assert_eq!(by_anchor[0].2.as_deref(), Some("src/auth/jwt.rs:42"));
}

#[test]
fn test_session_ring_buffer_pruning() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    // Add 25 sessions
    for i in 1..=25 {
        store.session_add(&format!("Session step {}", i)).unwrap();
    }

    // Must strictly keep only the latest 20 sessions
    let list = store.session_list(50).unwrap();
    assert_eq!(list.len(), 20, "ring buffer must cap sessions at 20");
    assert_eq!(list[0].1, "Session step 25");
    assert_eq!(list[19].1, "Session step 6");
}
