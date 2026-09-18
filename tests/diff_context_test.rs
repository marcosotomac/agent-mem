use agent_mem::cli::{Command, execute_command, parse_args};
use agent_mem::store::Store;
use std::fs;
use std::process::Command as SysCommand;

#[test]
fn test_store_context_for_files_and_hypergraph_expansion() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_diff_context_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("mem.db");
    let mut store = Store::open(&db_path, true).unwrap();

    // 1. Seed memories
    // A. Anchored to src/auth/jwt.rs
    store
        .set_with_anchor(
            "architecture/auth",
            "JWT RS256 with key rotation",
            Some("src/auth/jwt.rs:42"),
        )
        .unwrap();

    // B. Anchored to src/db/pool.rs (unrelated)
    store
        .set_with_anchor(
            "architecture/db",
            "Postgres 16 connection pooling with r2d2",
            Some("src/db/pool.rs:15"),
        )
        .unwrap();

    // C. 1-hop related to architecture/auth (gotcha)
    store
        .set_with_anchor(
            "gotcha/auth-header",
            "Authorization header must be Bearer prefix",
            None,
        )
        .unwrap();
    store
        .relate("architecture/auth", "mitigates", "gotcha/auth-header")
        .unwrap();

    // D. Universal rule (no anchor)
    store
        .set_with_anchor("rule/style", "Rust 2024 edition standard", None)
        .unwrap();

    // E. Another unrelated rule
    store
        .set_with_anchor(
            "decision/ui",
            "Tailwind CSS v4 with glassmorphism",
            Some("src/ui/styles.css:1"),
        )
        .unwrap();

    // Test selective context for src/auth/jwt.rs
    let active_files = vec!["src/auth/jwt.rs".to_string()];
    let (rules, rels, _sessions) = store.context_for_files(&active_files, None, 20).unwrap();

    let rule_keys: Vec<&str> = rules.iter().map(|r| r.key.as_str()).collect();

    // Must include direct match
    assert!(
        rule_keys.contains(&"architecture/auth"),
        "Should include directly anchored rule"
    );
    // Must include 1-hop related
    assert!(
        rule_keys.contains(&"gotcha/auth-header"),
        "Should include 1-hop related gotcha"
    );
    // Must include universal rule
    assert!(
        rule_keys.contains(&"rule/style"),
        "Should include universal unanchored rule"
    );

    // Must EXCLUDE unrelated rules (saving tokens!)
    assert!(
        !rule_keys.contains(&"architecture/db"),
        "Must exclude unrelated db rule"
    );
    assert!(
        !rule_keys.contains(&"decision/ui"),
        "Must exclude unrelated ui rule"
    );

    // Relations must contain the hypergraph link
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].source_key, "architecture/auth");
    assert_eq!(rels[0].target_key, "gotcha/auth-header");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_context_with_diff_in_git_repo() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_diff_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // Initialize git repo
    SysCommand::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    SysCommand::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    SysCommand::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Initialize agent-mem
    agent_mem::init::init_project(&temp_dir).unwrap();

    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let mut store = Store::open(&db_path, false).unwrap();

    store
        .set_with_anchor(
            "decision/auth",
            "Use argon2 for password hashing",
            Some("src/auth/hash.rs:10"),
        )
        .unwrap();
    store
        .set_with_anchor(
            "decision/db",
            "SQLite in WAL mode",
            Some("src/db/sqlite.rs:5"),
        )
        .unwrap();

    // Create an uncommitted change in src/auth/hash.rs
    let auth_dir = temp_dir.join("src").join("auth");
    fs::create_dir_all(&auth_dir).unwrap();
    fs::write(auth_dir.join("hash.rs"), "pub fn hash() {}").unwrap();

    // Run agent-mem context --diff
    let cmd = parse_args(vec!["agent-mem".into(), "context".into(), "--diff".into()]).unwrap();

    assert_eq!(
        cmd,
        Command::Context {
            anchor: None,
            topic: None,
            limit: 20,
            diff: true,
            files: Vec::new(),
        }
    );

    // Execute command
    let res = execute_command(cmd, &temp_dir);
    assert!(res.is_ok());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_exact_path_precision_over_alphabetical_basename_collision() {
    let mut store = Store::open_in_memory().unwrap();

    // 1. Seed two memories where "architecture/db" is alphabetically earlier than "decision/api",
    // but both share the generic basename "mod.rs".
    store
        .set_entry(
            "architecture/db",
            "Use PostgreSQL connection pool",
            Some("src/db/mod.rs:1"),
            Some("rule"),
        )
        .unwrap();

    store
        .set_entry(
            "decision/api",
            "Use GraphQL API router",
            Some("src/api/mod.rs:1"),
            Some("decision"),
        )
        .unwrap();

    // 2. Request context specifically for "src/api/mod.rs" with limit = 1.
    // Exact path matching must prioritize src/api/mod.rs over the alphabetically earlier src/db/mod.rs!
    let (rules, _, _) = store
        .context_for_files(&["src/api/mod.rs".to_string()], None, 1)
        .unwrap();

    assert_eq!(rules.len(), 1, "Should retrieve exactly 1 rule for limit=1");
    assert_eq!(
        rules[0].key, "decision/api",
        "Exact match for src/api/mod.rs must beat alphabetical precedence of src/db/mod.rs"
    );
    assert_eq!(rules[0].anchor.as_deref(), Some("src/api/mod.rs:1"));

    // 3. Request context for "src/db/mod.rs" with limit = 1
    let (db_rules, _, _) = store
        .context_for_files(&["src/db/mod.rs".to_string()], None, 1)
        .unwrap();

    assert_eq!(db_rules.len(), 1);
    assert_eq!(db_rules[0].key, "architecture/db");
}

#[test]
fn test_context_filtered_combined_anchor_and_topic() {
    let mut store = Store::open_in_memory().unwrap();

    // Seed rules on same anchor with different topics/kinds
    store
        .set_entry(
            "decision/auth",
            "JWT authentication",
            Some("src/auth/jwt.rs:10"),
            Some("decision"),
        )
        .unwrap();

    store
        .set_entry(
            "gotcha/auth",
            "Token expiration clock skew",
            Some("src/auth/jwt.rs:20"),
            Some("gotcha"),
        )
        .unwrap();

    store
        .set_entry(
            "gotcha/db",
            "Connection pool exhaustion",
            Some("src/db/pool.rs:5"),
            Some("gotcha"),
        )
        .unwrap();

    // 1. Query with BOTH anchor and topic specified: anchor = src/auth/jwt.rs, topic = "gotcha"
    let (rules, _, _) = store
        .context_filtered(Some("src/auth/jwt.rs"), Some("gotcha"), 10)
        .unwrap();

    assert_eq!(rules.len(), 1, "Must respect BOTH anchor AND topic filter");
    assert_eq!(rules[0].key, "gotcha/auth");
    assert_eq!(rules[0].kind, "gotcha");

    // 2. Query with BOTH anchor and topic = "decision"
    let (dec_rules, _, _) = store
        .context_filtered(Some("src/auth/jwt.rs"), Some("decision"), 10)
        .unwrap();

    assert_eq!(dec_rules.len(), 1);
    assert_eq!(dec_rules[0].key, "decision/auth");
}

#[test]
fn test_hierarchy_ranking_and_generic_basename_isolation() {
    let mut store = Store::open_in_memory().unwrap();

    store
        .set_entry(
            "pattern/api-sibling",
            "Use REST response envelope",
            Some("src/api/routes.rs:1"),
            Some("pattern"),
        )
        .unwrap();

    store
        .set_entry(
            "decision/api-exact",
            "GraphQL endpoint on /graphql",
            Some("src/api/mod.rs:1"),
            Some("decision"),
        )
        .unwrap();

    store
        .set_entry(
            "gotcha/db-mod",
            "Deadlock on concurrent transactions",
            Some("src/db/mod.rs:1"),
            Some("gotcha"),
        )
        .unwrap();

    // Query for src/api/mod.rs with limit = 2
    let (rules, _, _) = store
        .context_for_files(&["src/api/mod.rs".to_string()], None, 2)
        .unwrap();

    assert_eq!(rules.len(), 2);
    // 1st must be exact match
    assert_eq!(rules[0].key, "decision/api-exact");
    // 2nd must be same-directory sibling
    assert_eq!(rules[1].key, "pattern/api-sibling");
    // Generic mod.rs in different module (src/db/mod.rs) must NEVER be included
    assert!(!rules.iter().any(|r| r.key == "gotcha/db-mod"));
}

#[test]
fn test_many_changed_files_receive_fair_exact_match_candidates() {
    let mut store = Store::open_in_memory().unwrap();
    let mut files = Vec::new();
    for index in (0..100).rev() {
        let path = format!("services/service-{index:03}/src/handler.rs");
        store
            .set_with_anchor(
                &format!("decision/service-{index:03}"),
                &format!("Rule for service {index}"),
                Some(&format!("{path}:10")),
            )
            .unwrap();
        files.push(path);
    }

    let (rules, _, _) = store.context_for_files(&files, None, 20).unwrap();
    assert_eq!(rules.len(), 20);
    let keys: Vec<_> = rules.iter().map(|rule| rule.key.as_str()).collect();
    let expected: Vec<_> = (0..20)
        .map(|index| format!("decision/service-{index:03}"))
        .collect();
    assert_eq!(
        keys,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn test_topic_filter_applies_before_route_limit() {
    let mut store = Store::open_in_memory().unwrap();
    let anchor = "services/api/src/routes.rs:10";
    for index in 0..80 {
        store
            .set_with_anchor(
                &format!("aaa/noise-{index:03}"),
                "Unrelated rule on the same file",
                Some(anchor),
            )
            .unwrap();
    }
    store
        .set_with_anchor(
            "zzz/target",
            "Target rule after every alphabetical distractor",
            Some(anchor),
        )
        .unwrap();

    let (rules, _, _) = store
        .context_filtered(Some("services/api/src/routes.rs"), Some("zzz/"), 1)
        .unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].key, "zzz/target");
}

#[test]
fn test_incoming_edge_storm_does_not_hide_outgoing_context() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .set_with_anchor(
            "decision/direct",
            "Directly anchored decision",
            Some("src/api.rs:10"),
        )
        .unwrap();
    store
        .set("gotcha/required", "Required outgoing mitigation")
        .unwrap();
    store
        .relate("decision/direct", "mitigates", "gotcha/required")
        .unwrap();
    for index in 0..40 {
        let source = format!("aaa/incoming-{index:02}");
        store.set(&source, "Incoming graph noise").unwrap();
        store
            .relate(&source, "relates_to", "decision/direct")
            .unwrap();
    }

    let (rules, relations, _) = store.context_filtered(Some("src/api.rs"), None, 2).unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].key, "decision/direct");
    assert_eq!(rules[1].key, "gotcha/required");
    assert!(relations.iter().any(|relation| {
        relation.source_key == "decision/direct" && relation.target_key == "gotcha/required"
    }));
}
