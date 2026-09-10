use agent_mem::store::Store;

#[test]
fn test_crud_lifecycle() {
    let store = Store::open_in_memory().expect("open in memory db");

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
    let store = Store::open_in_memory().expect("open in memory db");

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
fn test_session_lifecycle() {
    let store = Store::open_in_memory().expect("open in memory db");

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
    let store = Store::open_in_memory().expect("open in memory db");

    store.set("convention", "Conventional commits only").unwrap();
    store.session_add("Setup project structure").unwrap();

    let (rules, sessions) = store.context().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].0, "convention");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].1, "Setup project structure");
}
