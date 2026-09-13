use agent_mem::store::Store;

#[test]
fn test_crud_lifecycle() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    // Initially empty
    assert_eq!(store.get("arch").unwrap(), None);

    // Set rule
    store.set("arch", "Clean Architecture with Rust").unwrap();
    assert_eq!(
        store.get("arch").unwrap().as_deref(),
        Some("Clean Architecture with Rust")
    );

    // Upsert rule
    store
        .set("arch", "Hexagonal Architecture with Rust")
        .unwrap();
    assert_eq!(
        store.get("arch").unwrap().as_deref(),
        Some("Hexagonal Architecture with Rust")
    );

    // Add another key
    store
        .set("tests", "Always write unit and integration tests")
        .unwrap();

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

    store
        .set(
            "db-rule",
            "Use SQLite in WAL mode for sub-millisecond reads",
        )
        .unwrap();
    store
        .set("auth-rule", "JWT tokens must be verified with RS256")
        .unwrap();
    store
        .set("api-rule", "Prefer HTTP/2 streaming over polling")
        .unwrap();

    // Search keyword in value
    let results = store.find("SQLite WAL").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "db-rule");

    // Search keyword in key
    let results_key = store.find("auth").unwrap();
    assert_eq!(results_key.len(), 1);
    assert_eq!(results_key[0].0, "auth-rule");

    // Search with special characters (must not crash FTS5 parser)
    let safe_search = store
        .find("SQLite: \"WAL\" mode (sub-millisecond)")
        .unwrap();
    assert!(!safe_search.is_empty());
    assert_eq!(safe_search[0].0, "db-rule");

    // Search non-existent
    let empty = store.find("nonexistentquerythatshouldnevermatch").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_no_phantom_results_after_delete() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    store
        .set("ghost-key", "this should disappear completely")
        .unwrap();
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

    store
        .set("convention", "Conventional commits only")
        .unwrap();
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
    store
        .set("wallet-rule", "Keep keys in your secure wallet")
        .unwrap();

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
    assert!(matches!(
        read_res,
        Err(agent_mem::error::Error::NotInitialized)
    ));

    // 2. Write open on 0-byte database must recover and initialize schema
    let mut write_store =
        Store::open(&db_path, true).expect("write open should initialize 0-byte db");
    write_store
        .set("recovered", "schema initialized successfully")
        .unwrap();
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

#[test]
fn test_empty_input_exceptions() {
    let mut store = Store::open_in_memory().expect("open in memory db");

    // 1. Empty key in set
    let res_empty_key = store.set("   ", "valid value");
    assert!(matches!(
        res_empty_key,
        Err(agent_mem::error::Error::Usage(_))
    ));

    // 2. Empty val in set
    let res_empty_val = store.set("valid_key", "   ");
    assert!(matches!(
        res_empty_val,
        Err(agent_mem::error::Error::Usage(_))
    ));

    // 3. Empty summary in session_add
    let res_empty_session = store.session_add("   ");
    assert!(matches!(
        res_empty_session,
        Err(agent_mem::error::Error::Usage(_))
    ));

    // 4. Empty query in find returns empty list safely
    let res_empty_find = store.find("   ").unwrap();
    assert!(res_empty_find.is_empty());

    // 5. Empty key in get returns None safely
    assert_eq!(store.get("  ").unwrap(), None);

    // 6. Empty key in del returns false safely
    assert!(!store.del("  ").unwrap());
}

#[test]
fn test_parse_and_export_rules_text() {
    let input = r#"
# Project team rules
# Auto-generated by agent-mem

architecture/db = Use SQLite WAL mode without rowid
auth/jwt = RS256 with 15-minute expiration (@ src/auth.rs:42)
conventions/naming = Use snake_case for functions
comments/ignored = // this line should not be ignored as a rule if key=val, but comments start with //
// full comment line
; ini style comment

api/streaming: Prefer HTTP/2 streaming (@ src/api.rs:10)
"#;

    let rules = Store::parse_rules_text(input);
    assert_eq!(rules.len(), 5);
    assert_eq!(
        rules[0],
        (
            "architecture/db".into(),
            "Use SQLite WAL mode without rowid".into(),
            None
        )
    );
    assert_eq!(
        rules[1],
        (
            "auth/jwt".into(),
            "RS256 with 15-minute expiration".into(),
            Some("src/auth.rs:42".into())
        )
    );
    assert_eq!(
        rules[2],
        (
            "conventions/naming".into(),
            "Use snake_case for functions".into(),
            None
        )
    );
    assert_eq!(
        rules[3],
        (
            "comments/ignored".into(),
            "// this line should not be ignored as a rule if key=val, but comments start with //"
                .into(),
            None
        )
    );
    assert_eq!(
        rules[4],
        (
            "api/streaming".into(),
            "Prefer HTTP/2 streaming".into(),
            Some("src/api.rs:10".into())
        )
    );

    // Verify deterministic export from Store
    let mut store = Store::open_in_memory().unwrap();
    for (k, v, a) in rules {
        store.set_with_anchor(&k, &v, a.as_deref()).unwrap();
    }
    let exported = store.export_rules_text().unwrap();
    assert!(exported.contains("api/streaming = Prefer HTTP/2 streaming (@ src/api.rs:10)"));
    assert!(exported.contains("architecture/db = Use SQLite WAL mode without rowid"));
    assert!(exported.contains("auth/jwt = RS256 with 15-minute expiration (@ src/auth.rs:42)"));
    assert!(exported.starts_with("# .agent-rules"));
}

#[test]
fn test_sync_with_file_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_sync_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");
    let rules_path = temp_dir.join(".agent-rules");

    let mut store = Store::open(&db_path, true).unwrap();
    store
        .set_with_anchor("arch/db", "SQLite WAL", Some("src/main.rs:1"))
        .unwrap();
    store.set("auth/type", "OAuth2").unwrap();

    // 1. Sync when file doesn't exist -> creates file
    let report1 = store.sync_with_file(&rules_path).unwrap();
    assert!(report1.file_created);
    assert_eq!(report1.total, 2);
    assert!(rules_path.exists());

    let file_content = std::fs::read_to_string(&rules_path).unwrap();
    assert!(file_content.contains("arch/db = SQLite WAL (@ src/main.rs:1)"));
    assert!(file_content.contains("auth/type = OAuth2"));

    // 2. Teammate edits file (deletes auth/type, adds lint/clippy)
    let new_team_rules = r#"
# Updated by teammate in Git
arch/db = SQLite WAL (@ src/main.rs:1)
lint/clippy = Run cargo clippy with -D warnings
"#;
    std::fs::write(&rules_path, new_team_rules).unwrap();

    // 3. Sync reconciles SQLite with teammate changes
    let report2 = store.sync_with_file(&rules_path).unwrap();
    assert!(!report2.file_created);
    assert_eq!(report2.imported, 2);
    assert_eq!(report2.total, 2);

    // Verify auth/type was removed (no zombie memory!)
    assert_eq!(store.get("auth/type").unwrap(), None);
    // Verify lint/clippy was added
    assert_eq!(
        store.get("lint/clippy").unwrap().as_deref(),
        Some("Run cargo clippy with -D warnings")
    );
    // Verify FTS5 search reflects the sync immediately
    let search_res = store.find("clippy").unwrap();
    assert_eq!(search_res.len(), 1);
    assert_eq!(search_res[0].0, "lint/clippy");

    // 4. sync_export explicitly rewrites file from SQLite
    store.set("cache/ttl", "3600s").unwrap();
    let report3 = store.sync_export(&rules_path).unwrap();
    assert!(report3.file_updated);
    assert_eq!(report3.total, 3);
    let exported_content = std::fs::read_to_string(&rules_path).unwrap();
    assert!(exported_content.contains("cache/ttl = 3600s"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sync_with_duplicate_keys_and_conflict_markers() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_conflict_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");
    let rules_path = temp_dir.join(".agent-rules");

    let mut store = Store::open(&db_path, true).unwrap();

    // Simulate git union merge conflict with duplicate keys and markers
    let conflict_content = r#"
# Git union merge output
arch/db = SQLite WAL
<<<<<<< HEAD
feature/auth = Use OAuth2 PKCE (@ src/auth.rs:10)
=======
feature/auth = Use Passkeys WebAuthn (@ src/webauthn.rs:20)
>>>>>>> branch-b
"#;
    std::fs::write(&rules_path, conflict_content).unwrap();

    // Sync must not crash with UNIQUE constraint error
    let report = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report.total, 2); // arch/db and feature/auth (last one wins)
    assert_eq!(
        store.get("feature/auth").unwrap().as_deref(),
        Some("Use Passkeys WebAuthn")
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

