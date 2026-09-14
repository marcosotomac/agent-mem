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
