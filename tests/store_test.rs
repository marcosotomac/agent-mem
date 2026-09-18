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
fn test_fts_search_and_safe_query_sanitization() {
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
    assert_eq!(results[0].key, "db-rule");

    // Search keyword in key
    let results_key = store.find("auth").unwrap();
    assert_eq!(results_key.len(), 1);
    assert_eq!(results_key[0].key, "auth-rule");

    // Search with special characters (must not crash FTS5 parser)
    let safe_search = store
        .find("SQLite: \"WAL\" mode (sub-millisecond)")
        .unwrap();
    assert!(!safe_search.is_empty());
    assert_eq!(safe_search[0].key, "db-rule");

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
    assert_eq!(found[0].key, "ghost-key");

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
    assert_eq!(stemmed[0].key, "arch-rule");

    // 2. Exact token: "wal" must NOT match "wallet" because automatic * wildcard was removed
    let no_wildcard_pollution = store.find("wal").unwrap();
    assert!(no_wildcard_pollution.is_empty());

    // 3. Explicit prefix search: user explicitly requested wildcard "wal*"
    let explicit_wildcard = store.find("wal*").unwrap();
    assert_eq!(explicit_wildcard.len(), 1);
    assert_eq!(explicit_wildcard[0].key, "wallet-rule");
}

#[test]
fn test_fts_zero_hit_fallback_recovers_noisy_queries() {
    let mut store = Store::open_in_memory().expect("open in memory db");
    store
        .set(
            "decision/payments/idempotency",
            "Require an Idempotency-Key header for every payment write",
        )
        .unwrap();
    store
        .set(
            "decision/payments/retries",
            "Retry payment reads with exponential backoff",
        )
        .unwrap();
    store
        .set(
            "decision/orders/deduplication",
            "Deduplicate order events by immutable event identifier",
        )
        .unwrap();

    // The strict AND query has no hit because the stored rule does not contain
    // "prevent" or "duplicate". The zero-hit OR fallback must still rank the
    // rule containing the two discriminative terms first.
    let results = store.find("prevent duplicate payment idempotency").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].key, "decision/payments/idempotency");
}

#[test]
fn test_route_index_tracks_anchor_updates_without_stale_matches() {
    let mut store = Store::open_in_memory().expect("open in memory db");
    store
        .set_with_anchor(
            "decision/auth",
            "Validate JWT issuer",
            Some("services/api/src/auth.rs:10"),
        )
        .unwrap();

    let (initial, _, _) = store
        .context_for_files(&["services/api/src/auth.rs".into()], None, 10)
        .unwrap();
    assert!(initial.iter().any(|rule| rule.key == "decision/auth"));

    store
        .set_with_anchor(
            "decision/auth",
            "Validate JWT issuer in the worker",
            Some("services/worker/src/token_validation.rs:20"),
        )
        .unwrap();

    let (old_path, _, _) = store
        .context_for_files(&["services/api/src/auth.rs".into()], None, 10)
        .unwrap();
    assert!(!old_path.iter().any(|rule| rule.key == "decision/auth"));
    let (new_path, _, _) = store
        .context_for_files(
            &["services/worker/src/token_validation.rs".into()],
            None,
            10,
        )
        .unwrap();
    assert!(new_path.iter().any(|rule| rule.key == "decision/auth"));
}

#[test]
fn test_v3_migration_backfills_route_index() {
    let dir =
        std::env::temp_dir().join(format!("agent_mem_route_migration_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");

    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor(
                "decision/api",
                "Keep handlers idempotent",
                Some("services/api/src/routes.rs:12"),
            )
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP TABLE memory_routes; PRAGMA user_version = 3;")
            .unwrap();
    }

    let store = Store::open(&db_path, false).expect("migrate v3 database");
    let (rules, _, _) = store
        .context_for_files(&["services/api/src/routes.rs".into()], None, 10)
        .unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].key, "decision/api");
    drop(store);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    let route_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_routes;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    assert!(route_count > 0);

    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
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
    assert_eq!(entry.val, "RS256 validation required for all requests");
    assert_eq!(entry.anchor.as_deref(), Some("src/auth/jwt.rs:42"));

    // 2. dump returns anchor
    let all = store.dump().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].2.as_deref(), Some("src/auth/jwt.rs:42"));

    // 3. BM25 / FTS matches on anchor path
    let by_anchor = store.find("auth/jwt.rs").unwrap();
    assert_eq!(by_anchor.len(), 1);
    assert_eq!(by_anchor[0].key, "jwt-auth");
    assert_eq!(by_anchor[0].anchor.as_deref(), Some("src/auth/jwt.rs:42"));
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
    assert_eq!(rules[0].key, "architecture/db");
    assert_eq!(rules[0].val, "Use SQLite WAL mode without rowid");
    assert_eq!(rules[0].anchor, None);

    assert_eq!(rules[1].key, "auth/jwt");
    assert_eq!(rules[1].val, "RS256 with 15-minute expiration");
    assert_eq!(rules[1].anchor.as_deref(), Some("src/auth.rs:42"));

    assert_eq!(rules[2].key, "conventions/naming");
    assert_eq!(rules[2].val, "Use snake_case for functions");
    assert_eq!(rules[2].anchor, None);

    assert_eq!(rules[3].key, "comments/ignored");
    assert_eq!(
        rules[3].val,
        "// this line should not be ignored as a rule if key=val, but comments start with //"
    );
    assert_eq!(rules[3].anchor, None);

    assert_eq!(rules[4].key, "api/streaming");
    assert_eq!(rules[4].val, "Prefer HTTP/2 streaming");
    assert_eq!(rules[4].anchor.as_deref(), Some("src/api.rs:10"));

    // Verify deterministic export from Store
    let mut store = Store::open_in_memory().unwrap();
    for r in rules {
        store
            .set_with_anchor(&r.key, &r.val, r.anchor.as_deref())
            .unwrap();
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
    assert_eq!(search_res[0].key, "lint/clippy");

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
[rel] feature/auth -> depends_on -> arch/db
[rel] feature/auth -> depends_on -> arch/db
"#;
    std::fs::write(&rules_path, conflict_content).unwrap();

    // Sync must not crash with UNIQUE constraint error
    let report = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report.total, 2); // arch/db and feature/auth (last one wins)
    assert_eq!(store.get_all_relations().unwrap().len(), 1);
    store.sync_with_file(&rules_path).unwrap();
    assert_eq!(store.get_all_relations().unwrap().len(), 1);
    assert_eq!(
        store.get("feature/auth").unwrap().as_deref(),
        Some("Use Passkeys WebAuthn")
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_set_with_relation_rolls_back_on_relation_error() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_relation_rollback_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");

    drop(Store::open(&db_path, true).unwrap());
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_relation_insert
         BEFORE INSERT ON relations
         BEGIN
             SELECT RAISE(ABORT, 'forced relation failure');
         END;",
    )
    .unwrap();
    drop(conn);

    let mut store = Store::open(&db_path, true).unwrap();
    let result = store.set_entry_with_relation(
        "decision/atomic",
        "memory and edge commit together",
        None,
        Some("decision"),
        Some(("depends_on", "architecture/storage")),
    );
    assert!(result.is_err());
    assert!(store.get_entry("decision/atomic").unwrap().is_none());
    assert!(store.get_all_relations().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_relation_read_errors_are_never_silenced() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_relation_errors_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");

    let mut store = Store::open(&db_path, true).unwrap();
    store
        .set_with_anchor("anchored", "matched rule", Some("src/matched.rs:1"))
        .unwrap();
    store.set("universal", "always relevant").unwrap();
    drop(store);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("DROP TABLE relations;", []).unwrap();
    drop(conn);

    let store = Store::open(&db_path, false).unwrap();
    assert!(store.context_filtered(None, None, 10).is_err());
    assert!(
        store
            .context_for_files(&["src/matched.rs".into()], None, 10)
            .is_err()
    );
    assert!(
        store
            .context_for_files(&["src/unmatched.rs".into()], None, 10)
            .is_err()
    );
    assert!(store.export_rules_text().is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_export_replaces_file_without_leaving_temporary_artifacts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_atomic_export_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let rules_path = temp_dir.join(".agent-rules");
    std::fs::write(&rules_path, "old partial-prone content").unwrap();

    let mut store = Store::open_in_memory().unwrap();
    store
        .set("architecture/atomic", "replace via rename")
        .unwrap();
    store.export_to_file(&rules_path).unwrap();

    let exported = std::fs::read_to_string(&rules_path).unwrap();
    assert!(exported.contains("architecture/atomic = replace via rename"));
    assert!(!exported.contains("old partial-prone content"));
    let leftovers: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("..agent-rules.tmp-")
        })
        .collect();
    assert!(leftovers.is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_failed_migration_rolls_back_schema_and_version() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_migration_rollback_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("legacy.db");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE memories (
             key TEXT PRIMARY KEY,
             val TEXT NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE relations (wrong_column TEXT);
         PRAGMA user_version = 0;",
    )
    .unwrap();
    drop(conn);

    assert!(Store::open(&db_path, true).is_err());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    let anchor_columns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'anchor';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 0);
    assert_eq!(anchor_columns, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sync_roundtrip_preserves_ambiguous_content_and_metadata() {
    let dir = std::env::temp_dir().join(format!("agent_mem_escaped_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".agent-rules");
    let mut store = Store::open_in_memory().unwrap();
    let values = [
        "First line\nSecond line",
        "Deploy @ staging",
        "Literal (@ src/example.rs:10)",
        "Pass --reason: unchanged",
        "Pass --reason unchanged",
        "CRLF\r\nTabs\tand Unicode: español 🦀",
        r#"Quotes "hello" and literal \n stay intact"#,
        "Text\ninjected/key = must remain inside the value",
    ];
    for (i, val) in values.iter().enumerate() {
        store.set(&format!("case/{i}"), val).unwrap();
    }
    for byte in (0u8..32).chain(std::iter::once(127)) {
        store
            .set(
                &format!("control/{byte}"),
                &format!("before{}after", char::from(byte)),
            )
            .unwrap();
    }
    for key in [
        "#comment",
        "[decision] literal",
        "key=with=equals",
        "key\nnewline",
        "a -> b",
    ] {
        store
            .set_entry(key, "literal", Some("src/a @ b.rs:1"), Some("CustomKind"))
            .unwrap();
    }
    store
        .set_entry("decision/override", "explicit rule", None, Some("rule"))
        .unwrap();
    store
        .archive("case/0", Some("Replaced\nUse @ next --reason: literal"))
        .unwrap();
    store.relate("a -> b", "relates_to", "case/0").unwrap();
    store.relate("case/1", "depends_on", "case/2").unwrap();

    let exported = store.export_rules_text().unwrap();
    assert!(exported.contains("[rule-json-v1]"));
    assert!(exported.contains("[rel-json-v1]"));
    assert!(exported.contains("[rule] decision/override = explicit rule"));
    std::fs::write(&path, &exported).unwrap();
    let mut imported = Store::open_in_memory().unwrap();
    imported.sync_with_file(&path).unwrap();
    assert_eq!(imported.export_rules_text().unwrap(), exported);
    for original in store.dump_all().unwrap() {
        let restored = imported.get_entry(&original.key).unwrap().unwrap();
        assert_eq!(restored.val, original.val);
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.anchor, original.anchor);
        assert_eq!(restored.archive_reason, original.archive_reason);
        assert_eq!(restored.is_archived(), original.is_archived());
    }
    assert_eq!(imported.get_all_relations().unwrap().len(), 2);
    assert_eq!(imported.get("injected/key").unwrap(), None);
    assert!(!imported.find("Second").unwrap().is_empty());
    // Existing simple rules stay compact and need no format migration.
    assert!(exported.contains("case/6 = Quotes"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_sync_rejects_malformed_encoded_records_without_mutation() {
    let dir = std::env::temp_dir().join(format!("agent_mem_bad_encoding_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".agent-rules");
    let mut store = Store::open_in_memory().unwrap();
    store.set("preserved", "original memory").unwrap();
    store
        .relate("preserved", "relates_to", "preserved")
        .unwrap();
    let before = store.export_rules_text().unwrap();
    for bad in [
        "[rule-json-v1] {",
        "[rule-json-v1]",
        "[rule-json-v1] {}",
        "[rel-json-v1] [\"a\"]",
    ] {
        std::fs::write(&path, format!("new = must not import\n{bad}\n")).unwrap();
        let error = store.sync_with_file(&path).unwrap_err().to_string();
        assert!(error.contains("line 2"), "{error}");
        assert_eq!(store.export_rules_text().unwrap(), before);
        assert_eq!(store.find("original").unwrap().len(), 1);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_export_uses_one_snapshot_during_concurrent_archiving() {
    use std::sync::{Arc, Barrier};
    let dir =
        std::env::temp_dir().join(format!("agent_mem_export_snapshot_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mem.db");
    let mut store = Store::open(&path, true).unwrap();
    for i in 0..500 {
        store
            .set(&format!("rule/{i:04}"), "preserve every rule")
            .unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);
    let writer = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        writer_barrier.wait();
        for i in 0..200 {
            conn.execute(
                "UPDATE memories SET archived_at = ?1",
                [if i % 2 == 0 { Some(1i64) } else { None }],
            )
            .unwrap();
            std::thread::yield_now();
        }
    });
    barrier.wait();
    for _ in 0..100 {
        let text = store.export_rules_text().unwrap();
        let rules = Store::parse_rules_text(&text);
        let keys: std::collections::BTreeSet<_> = rules.iter().map(|r| &r.key).collect();
        assert_eq!(rules.len(), 500);
        assert_eq!(keys.len(), 500);
        // Every atomic writer update archives or activates the entire set.
        assert!(
            rules
                .iter()
                .all(|r| r.is_archived() == rules[0].is_archived())
        );
    }
    writer.join().unwrap();
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_sync_preserves_historic_timestamps_for_unmodified_rules() {
    let dir =
        std::env::temp_dir().join(format!("agent_mem_sync_timestamps_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let rules_path = dir.join(".agent-rules");

    let mut store = Store::open(&db_path, true).unwrap();

    // Seed rules with historic timestamps directly in SQLite
    let historic_time = 1_600_000_000i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind)
             VALUES ('rule/preserved', 'value unchanged', ?1, 'src/lib.rs:1', NULL, NULL, 'rule'),
                    ('rule/modified', 'old value', ?1, NULL, NULL, NULL, 'rule');",
            rusqlite::params![historic_time],
        )
        .unwrap();
    }

    // Create file where rule/preserved is untouched and rule/modified has a new value
    let file_content =
        "rule/preserved = value unchanged (@ src/lib.rs:1)\nrule/modified = new upgraded value\n";
    std::fs::write(&rules_path, file_content).unwrap();

    let report = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.conflicts_resolved, 0);

    // Verify timestamps in SQLite
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let preserved_updated: i64 = conn
        .query_row(
            "SELECT updated_at FROM memories WHERE key = 'rule/preserved';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        preserved_updated, historic_time,
        "Unmodified rule must retain historic timestamp"
    );

    let modified_updated: i64 = conn
        .query_row(
            "SELECT updated_at FROM memories WHERE key = 'rule/modified';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        modified_updated > historic_time,
        "Modified rule must receive a fresh timestamp"
    );

    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_sync_detects_and_logs_conflicts_with_provenance() {
    let dir = std::env::temp_dir().join(format!("agent_mem_sync_conflicts_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let rules_path = dir.join(".agent-rules");

    let mut store = Store::open(&db_path, true).unwrap();

    // .agent-rules containing conflicting definitions for the same key
    let file_content = r#"
# Branch merge with conflicting rule definitions
gotcha/auth = validate JWT issuer before expiration (@ src/auth.rs:12)
gotcha/auth = use constant-time comparison for tokens (@ src/crypto.rs:45)
"#;
    std::fs::write(&rules_path, file_content).unwrap();

    let report = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(
        report.conflicts_resolved, 1,
        "Must detect 1 conflict resolution"
    );

    // The winning value should be the last definition
    let stored = store
        .get_entry("gotcha/auth")
        .unwrap()
        .expect("Rule must exist");
    assert_eq!(stored.val, "use constant-time comparison for tokens");
    assert_eq!(stored.anchor.as_deref(), Some("src/crypto.rs:45"));

    // Verify session audit log provenance
    let sessions = store.session_list(10).unwrap();
    assert!(
        sessions.iter().any(|(_, summary)| {
            summary.contains("[conflict-resolved] 'gotcha/auth'")
                && summary.contains("validate JWT issuer")
                && summary.contains("use constant-time comparison")
        }),
        "Session audit log must record conflict resolution with full provenance"
    );

    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_store_set_batch_atomic_transaction() {
    let dir = std::env::temp_dir().join(format!("agent_mem_batch_set_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let mut store = Store::open(&db_path, true).unwrap();

    let rules = [
        agent_mem::BatchRule {
            key: "auth/jwt",
            val: "Use RS256 with key rotation",
            anchor: Some("src/auth.rs:10"),
            kind: Some("decision"),
            relation: Some(("relates_to", "crypto/keys")),
        },
        agent_mem::BatchRule {
            key: "crypto/keys",
            val: "Rotate RSA keys every 90 days",
            anchor: Some("src/crypto.rs:25"),
            kind: Some("rule"),
            relation: None,
        },
        agent_mem::BatchRule {
            key: "db/wal",
            val: "Enable WAL mode",
            anchor: None,
            kind: Some("rule"),
            relation: None,
        },
    ];

    let count = store.set_batch(&rules).unwrap();
    assert_eq!(count, 3);

    // Verify all memories, FTS, and relations were saved
    assert_eq!(
        store.get("auth/jwt").unwrap(),
        Some("Use RS256 with key rotation".into())
    );
    assert_eq!(
        store.get("crypto/keys").unwrap(),
        Some("Rotate RSA keys every 90 days".into())
    );
    assert_eq!(store.get("db/wal").unwrap(), Some("Enable WAL mode".into()));

    let relations = store.get_relations("auth/jwt").unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].0, "relates_to");
    assert_eq!(relations[0].1, "crypto/keys");

    let fts_hits = store.find("rotation").unwrap();
    assert_eq!(fts_hits.len(), 2);

    // Verify atomic rollback on invalid entry in batch
    let invalid_rules = [
        agent_mem::BatchRule {
            key: "valid/one",
            val: "Valid value",
            anchor: None,
            kind: None,
            relation: None,
        },
        agent_mem::BatchRule {
            key: "", // invalid empty key!
            val: "Should trigger rollback",
            anchor: None,
            kind: None,
            relation: None,
        },
    ];
    let err = store.set_batch(&invalid_rules);
    assert!(err.is_err());
    assert_eq!(
        store.get("valid/one").unwrap(),
        None,
        "Rollback must ensure no partial writes"
    );

    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_sync_noop_detection_and_invalidation() {
    let dir = std::env::temp_dir().join(format!("agent_mem_sync_noop_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let rules_path = dir.join(".agent-rules");
    let mut store = Store::open(&db_path, true).unwrap();

    let initial_content = "rule/one = first rule\nrule/two = second rule\n";
    std::fs::write(&rules_path, initial_content).unwrap();

    // First sync imports rules
    let report1 = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report1.total, 2);

    // Second sync without modifications hits no-op path
    let report2 = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report2.total, 2);

    // File change invalidates no-op cache and triggers sync
    std::fs::write(
        &rules_path,
        "rule/one = first rule\nrule/two = updated second rule\nrule/three = third rule\n",
    )
    .unwrap();
    let report3 = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report3.total, 3);
    assert_eq!(
        store.get("rule/two").unwrap(),
        Some("updated second rule".into())
    );

    // DB modification invalidates cache
    store.set("rule/four", "fourth rule").unwrap();
    // Subsequent sync reconciles to match the file (which only has three rules)
    let report4 = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report4.total, 3);
    assert_eq!(store.get("rule/four").unwrap(), None);

    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_write_limits_reject_oversized_values_before_mutation() {
    let mut store = Store::open_in_memory().unwrap();
    let oversized = "x".repeat(agent_mem::store::MAX_VALUE_BYTES + 1);

    let error = store.set("too-large", &oversized).unwrap_err();

    assert!(error.to_string().contains("65536-byte limit"));
    assert_eq!(store.get("too-large").unwrap(), None);
}

#[test]
fn test_sync_rejects_oversized_rules_file_before_reading_it() {
    let dir = std::env::temp_dir().join(format!("agent_mem_sync_limit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let rules_path = dir.join(".agent-rules");
    let file = std::fs::File::create(&rules_path).unwrap();
    file.set_len(32 * 1024 * 1024 + 1).unwrap();
    let mut store = Store::open(&dir.join("mem.db"), true).unwrap();

    let error = store.sync_with_file(&rules_path).unwrap_err();

    assert!(error.to_string().contains("33554432-byte import limit"));
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}
