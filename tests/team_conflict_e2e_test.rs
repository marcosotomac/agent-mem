use agent_mem::init::init_project;
use agent_mem::store::Store;
use std::process::Command;

fn run_git(cwd: &std::path::Path, args: &[&str]) -> String {
    let mut full_args = vec!["-c", "core.hooksPath=/dev/null"];
    full_args.extend_from_slice(args);
    let output = Command::new("git")
        .current_dir(cwd)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute git {:?}: {}", args, e));

    if !output.status.success() {
        panic!(
            "git {:?} failed in {:?}:\nSTDOUT: {}\nSTDERR: {}",
            args,
            cwd,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn test_team_multi_branch_swarm_and_union_merge_resolution() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_team_swarm_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Setup git repo
    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "Team Lead"]);
    run_git(&temp_dir, &["config", "user.email", "lead@enterprise.com"]);

    // 2. Initialize agent-mem
    let report = init_project(&temp_dir).unwrap();
    assert!(report.gitattributes_updated);
    assert!(report.post_merge_configured);
    assert!(report.post_checkout_configured);

    // Seed base architecture on main
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let rules_path = temp_dir.join(".agent-rules");

    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor("arch/core", "Hexagonal Architecture", Some("src/main.rs:1"))
            .unwrap();
        store.sync_export(&rules_path).unwrap();
    }

    run_git(&temp_dir, &["add", "."]);
    run_git(&temp_dir, &["commit", "-m", "chore: initial team seed"]);

    // 3. Alice develops Auth in parallel
    run_git(&temp_dir, &["checkout", "-b", "alice/feature-auth"]);
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor(
                "auth/jwt",
                "RS256 with 15m expiration",
                Some("src/auth.rs:10"),
            )
            .unwrap();
        store
            .set("auth/mfa", "TOTP authentication required for admin roles")
            .unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", ".agent-rules"]);
    run_git(
        &temp_dir,
        &["commit", "-m", "feat(auth): add jwt and mfa policies"],
    );

    // 4. Bob develops Billing in parallel from main
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(&temp_dir, &["checkout", "-b", "bob/feature-billing"]);
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor(
                "billing/stripe",
                "Idempotency keys on all charge requests",
                Some("src/billing.rs:25"),
            )
            .unwrap();
        store
            .set("billing/currency", "Strict ISO 4217 standard")
            .unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", ".agent-rules"]);
    run_git(
        &temp_dir,
        &[
            "commit",
            "-m",
            "feat(billing): add stripe and currency rules",
        ],
    );

    // 5. Charlie develops Cache in parallel from main
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(&temp_dir, &["checkout", "-b", "charlie/feature-cache"]);
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor(
                "cache/redis",
                "Cluster mode with sentinel failover",
                Some("src/cache.rs:5"),
            )
            .unwrap();
        store.set("cache/ttl", "Default TTL 3600 seconds").unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", ".agent-rules"]);
    run_git(
        &temp_dir,
        &[
            "commit",
            "-m",
            "feat(cache): add redis cluster and ttl rules",
        ],
    );

    // 6. Merge Alice -> main
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(&temp_dir, &["merge", "alice/feature-auth"]);

    // 7. Merge Bob -> main (Union merge handles divergent rules without conflict)
    run_git(&temp_dir, &["merge", "bob/feature-billing"]);

    // 8. Merge Charlie -> main
    run_git(&temp_dir, &["merge", "charlie/feature-cache"]);

    // 9. Sync and verify SQLite on main
    let mut store = Store::open(&db_path, true).unwrap();
    let sync_rep = store.sync_with_file(&rules_path).unwrap();

    // All 7 rules must be present without duplicates or loss
    assert_eq!(sync_rep.total, 7);
    assert_eq!(
        store.get("arch/core").unwrap().as_deref(),
        Some("Hexagonal Architecture")
    );
    assert_eq!(
        store.get("auth/jwt").unwrap().as_deref(),
        Some("RS256 with 15m expiration")
    );
    assert_eq!(
        store.get("billing/stripe").unwrap().as_deref(),
        Some("Idempotency keys on all charge requests")
    );
    assert_eq!(
        store.get("cache/redis").unwrap().as_deref(),
        Some("Cluster mode with sentinel failover")
    );

    // BM25 full-text search across merged team knowledge
    let hits = store.find("Idempotency").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, "billing/stripe");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_conflicting_edits_on_same_rule_key_across_branches() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_key_conflict_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "Lead"]);
    run_git(&temp_dir, &["config", "user.email", "lead@enterprise.com"]);

    init_project(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let rules_path = temp_dir.join(".agent-rules");

    {
        let mut store = Store::open(&db_path, true).unwrap();
        store.set("auth/token", "Initial token standard").unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", "."]);
    run_git(&temp_dir, &["commit", "-m", "initial commit"]);

    // Branch A updates auth/token
    run_git(&temp_dir, &["checkout", "-b", "branch-a"]);
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store.set("auth/token", "Branch A: Use Ed25519").unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", ".agent-rules"]);
    run_git(&temp_dir, &["commit", "-m", "update from branch a"]);

    // Branch B updates auth/token with different value
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(&temp_dir, &["checkout", "-b", "branch-b"]);
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store.set("auth/token", "Branch B: Use ES384").unwrap();
        store.sync_export(&rules_path).unwrap();
    }
    run_git(&temp_dir, &["add", ".agent-rules"]);
    run_git(&temp_dir, &["commit", "-m", "update from branch b"]);

    // Merge Branch A
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(&temp_dir, &["merge", "branch-a"]);

    // Merge Branch B (union merge combines both lines)
    run_git(&temp_dir, &["merge", "branch-b"]);

    // Sync must resolve deterministically via UPSERT without database crash
    let mut store = Store::open(&db_path, true).unwrap();
    let report = store.sync_with_file(&rules_path).unwrap();
    assert_eq!(report.total, 1);

    let val = store.get("auth/token").unwrap().expect("rule exists");
    assert!(
        val.starts_with("Branch"),
        "Deterministic resolution succeeded: {}",
        val
    );

    // Clean export consolidates and eliminates duplicate lines
    let export_rep = store.sync_export(&rules_path).unwrap();
    assert_eq!(export_rep.total, 1);
    let clean_content = std::fs::read_to_string(&rules_path).unwrap();
    let auth_token_occurrences = clean_content.matches("auth/token").count();
    assert_eq!(
        auth_token_occurrences, 1,
        "Duplicate keys consolidated into 1 line"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_dirty_team_file_with_crlf_and_comments() {
    let dirty = "
# Comment line with # another hash
// C-style comment
; INI style comment

auth/jwt = RS256 with equals = inside = the = value\r\n\
db/pool: 20 connections max (@ src/db.rs:40)\r\n\
\r\n\
  whitespace_key   =   clean value with spaces   \r\n\
<<<<<<< HEAD\r\n\
conflict/ignored = version 1\r\n\
=======\r\n\
conflict/ignored = version 2\r\n\
>>>>>>> other-branch\r\n\
";

    let parsed = Store::parse_rules_text(dirty);
    assert_eq!(parsed.len(), 5);

    let map: std::collections::HashMap<String, (String, Option<String>)> = parsed
        .into_iter()
        .map(|r| (r.key, (r.val, r.anchor)))
        .collect();

    assert_eq!(map.len(), 4);

    assert_eq!(
        map.get("auth/jwt").unwrap().0,
        "RS256 with equals = inside = the = value"
    );
    assert_eq!(
        map.get("db/pool").unwrap(),
        &(
            "20 connections max".to_string(),
            Some("src/db.rs:40".to_string())
        )
    );
    assert_eq!(
        map.get("whitespace_key").unwrap().0,
        "clean value with spaces"
    );
}
