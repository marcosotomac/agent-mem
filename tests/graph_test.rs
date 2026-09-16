use agent_mem::cli::{Command, execute_command};
use agent_mem::store::{Store, infer_kind};
use std::fs;

#[test]
fn test_multi_entity_kind_inference_and_explicit_storage() {
    assert_eq!(infer_kind("decision/auth-strategy"), "decision");
    assert_eq!(infer_kind("adr/0001-microservices"), "decision");
    assert_eq!(infer_kind("gotcha/sqlite-wal-locking"), "gotcha");
    assert_eq!(infer_kind("bug/connection-leak"), "gotcha");
    assert_eq!(infer_kind("trap/reentrancy"), "gotcha");
    assert_eq!(infer_kind("pattern/repository"), "pattern");
    assert_eq!(infer_kind("arch/db"), "rule");
    assert_eq!(infer_kind("coding-style"), "rule");
    assert_eq!(infer_kind("DECISION/auth"), "decision");
    assert_eq!(infer_kind("GoTcHa:unicode-🦀"), "gotcha");
    assert_eq!(infer_kind("PATTERN:reuse"), "pattern");
    assert_eq!(infer_kind("🦀/architecture"), "rule");

    let mut store = Store::open_in_memory().unwrap();

    // 1. Store via set_entry with explicit kind
    store
        .set_entry(
            "auth/jwt",
            "Use RS256 with rotation",
            Some("src/auth.rs:15"),
            Some("decision"),
        )
        .unwrap();

    // 2. Store via auto-inference
    store
        .set("gotcha/sqlite-timeout", "Busy timeout must be >= 5000ms")
        .unwrap();
    store
        .set("pattern/unit-of-work", "Encapsulate transactions cleanly")
        .unwrap();
    store
        .set("general/formatter", "Run cargo fmt before committing")
        .unwrap();

    let entry_jwt = store.get_entry("auth/jwt").unwrap().unwrap();
    assert_eq!(entry_jwt.kind, "decision");
    assert_eq!(entry_jwt.anchor.as_deref(), Some("src/auth.rs:15"));

    let entry_gotcha = store.get_entry("gotcha/sqlite-timeout").unwrap().unwrap();
    assert_eq!(entry_gotcha.kind, "gotcha");

    let entry_pat = store.get_entry("pattern/unit-of-work").unwrap().unwrap();
    assert_eq!(entry_pat.kind, "pattern");

    let entry_rule = store.get_entry("general/formatter").unwrap().unwrap();
    assert_eq!(entry_rule.kind, "rule");
}

#[test]
fn test_knowledge_hypergraph_relations_and_cascade_delete() {
    let mut store = Store::open_in_memory().unwrap();

    store
        .set_entry(
            "decision/jwt",
            "Use RS256",
            Some("src/auth.rs"),
            Some("decision"),
        )
        .unwrap();
    store
        .set_entry(
            "gotcha/replay",
            "Token replay attack possible without nonce",
            None,
            Some("gotcha"),
        )
        .unwrap();
    store
        .set_entry(
            "pattern/nonce-store",
            "Redis nonce cache with 5m TTL",
            None,
            Some("pattern"),
        )
        .unwrap();

    // Relate
    store
        .relate("decision/jwt", "mitigates", "gotcha/replay")
        .unwrap();
    store
        .relate("pattern/nonce-store", "depends_on", "decision/jwt")
        .unwrap();

    // Deduplication test (INSERT OR IGNORE)
    store
        .relate("decision/jwt", "mitigates", "gotcha/replay")
        .unwrap();

    let rels_jwt = store.get_relations("decision/jwt").unwrap();
    assert_eq!(rels_jwt.len(), 1);
    assert_eq!(
        rels_jwt[0],
        ("mitigates".to_string(), "gotcha/replay".to_string())
    );

    let all_rels = store.get_all_relations().unwrap();
    assert_eq!(all_rels.len(), 2);

    // Unrelate
    let unlinked = store
        .unrelate("pattern/nonce-store", "depends_on", "decision/jwt")
        .unwrap();
    assert!(unlinked);
    assert_eq!(store.get_all_relations().unwrap().len(), 1);

    // Cascade delete: deleting decision/jwt must remove its relation with gotcha/replay
    let deleted = store.del("decision/jwt").unwrap();
    assert!(deleted);
    assert_eq!(store.get_all_relations().unwrap().len(), 0);
}

#[test]
fn test_anchor_context_filtering_and_1_hop_traversal() {
    let mut store = Store::open_in_memory().unwrap();

    // Create 3 rules with anchors, 1 without, and a hypergraph relation
    store
        .set_entry(
            "auth/jwt",
            "JWT RS256 authentication",
            Some("src/auth/token.rs:20"),
            Some("decision"),
        )
        .unwrap();
    store
        .set_entry(
            "auth/gotcha",
            "Tokens without exp field pass validation",
            None,
            Some("gotcha"),
        )
        .unwrap();
    store
        .set_entry(
            "db/pool",
            "Max pool size 20",
            Some("src/db/pool.rs:10"),
            Some("rule"),
        )
        .unwrap();
    store
        .set_entry(
            "ui/theme",
            "Dark mode default",
            Some("src/ui/theme.rs:5"),
            Some("rule"),
        )
        .unwrap();

    // Link: auth/jwt relates to auth/gotcha
    store
        .relate("auth/jwt", "mitigates", "auth/gotcha")
        .unwrap();

    // Filter by anchor: "src/auth/token.rs"
    let (rules, rels, _) = store
        .context_filtered(Some("src/auth/token.rs"), None, 10)
        .unwrap();

    // Must retrieve auth/jwt (direct anchor match) AND auth/gotcha (1-hop hypergraph expansion)!
    let keys: Vec<&str> = rules.iter().map(|r| r.key.as_str()).collect();
    assert!(keys.contains(&"auth/jwt"));
    assert!(keys.contains(&"auth/gotcha"));
    // Must NOT contain unrelated db/pool or ui/theme (cutting token footprint by >50% here, >95% in large repos)
    assert!(!keys.contains(&"db/pool"));
    assert!(!keys.contains(&"ui/theme"));

    // Relations must contain the relevant link
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].source_key, "auth/jwt");
    assert_eq!(rels[0].rel_type, "mitigates");
    assert_eq!(rels[0].target_key, "auth/gotcha");

    // Filter by topic: "db/"
    let (topic_rules, _, _) = store.context_filtered(None, Some("db/"), 10).unwrap();
    assert_eq!(topic_rules.len(), 1);
    assert_eq!(topic_rules[0].key, "db/pool");
}

#[test]
fn test_plain_text_team_git_sync_roundtrip_with_graph_relations() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_graph_sync_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let rules_file = temp_dir.join(".agent-rules");

    // 1. Create entities and relations in Store
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_entry(
                "auth/jwt",
                "JWT RS256",
                Some("src/auth.rs:10"),
                Some("decision"),
            )
            .unwrap();
        store
            .set_entry("gotcha/leak", "Don't log payload", None, Some("gotcha"))
            .unwrap();
        store
            .relate("auth/jwt", "mitigates", "gotcha/leak")
            .unwrap();

        store.sync_export(&rules_file).unwrap();
    }

    // 2. Verify .agent-rules text format
    let content = fs::read_to_string(&rules_file).unwrap();
    assert!(content.contains("[decision] auth/jwt = JWT RS256 (@ src/auth.rs:10)"));
    assert!(content.contains("[gotcha] gotcha/leak = Don't log payload"));
    assert!(content.contains("[rel] auth/jwt -> mitigates -> gotcha/leak"));

    // 3. Open fresh second store and sync from file
    let db_path_2 = temp_dir.join(".agent-mem").join("mem_replica.db");
    {
        let mut store2 = Store::open(&db_path_2, true).unwrap();
        let report = store2.sync_with_file(&rules_file).unwrap();
        assert_eq!(report.total, 2);

        let jwt_entry = store2.get_entry("auth/jwt").unwrap().unwrap();
        assert_eq!(jwt_entry.kind, "decision");
        assert_eq!(jwt_entry.anchor.as_deref(), Some("src/auth.rs:10"));

        let leak_entry = store2.get_entry("gotcha/leak").unwrap().unwrap();
        assert_eq!(leak_entry.kind, "gotcha");

        let rels = store2.get_relations("auth/jwt").unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(
            rels[0],
            ("mitigates".to_string(), "gotcha/leak".to_string())
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_relate_unrelate_and_context_filtered() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_graph_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    agent_mem::init::init_project(&temp_dir).unwrap();

    // 1. CLI Set with kind and anchor
    let set_cmd = Command::Set {
        key: "api/graphql".into(),
        val: "Use DataLoader to prevent N+1 queries".into(),
        anchor: Some("src/graphql/schema.rs:40".into()),
        kind: Some("decision".into()),
    };
    execute_command(set_cmd, &temp_dir).unwrap();

    let set_gotcha = Command::Set {
        key: "gotcha/n-plus-one".into(),
        val: "Eager loading creates memory spikes".into(),
        anchor: None,
        kind: Some("gotcha".into()),
    };
    execute_command(set_gotcha, &temp_dir).unwrap();

    // 2. CLI Relate
    let relate_cmd = Command::Relate {
        source: "api/graphql".into(),
        rel_type: "mitigates".into(),
        target: "gotcha/n-plus-one".into(),
    };
    execute_command(relate_cmd, &temp_dir).unwrap();

    // Verify .agent-rules was automatically synced with the relation
    let rules_content = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(rules_content.contains("[rel] api/graphql -> mitigates -> gotcha/n-plus-one"));

    // 3. CLI Unrelate
    let unrelate_cmd = Command::Unrelate {
        source: "api/graphql".into(),
        rel_type: "mitigates".into(),
        target: "gotcha/n-plus-one".into(),
    };
    execute_command(unrelate_cmd, &temp_dir).unwrap();

    let rules_content_after = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(!rules_content_after.contains("[rel] api/graphql -> mitigates -> gotcha/n-plus-one"));

    let _ = fs::remove_dir_all(&temp_dir);
}
