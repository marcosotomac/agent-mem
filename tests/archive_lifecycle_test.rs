use agent_mem::cli::{Command, parse_args};
use agent_mem::init::init_project;
use agent_mem::store::Store;
use std::fs;
use std::process::Command as SysCommand;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = SysCommand::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git execution failed");
    if !output.status.success() {
        panic!(
            "git command failed: git {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn test_archive_and_unarchive_lifecycle() {
    let mut store = Store::open_in_memory().unwrap();

    // 1. Set active rule
    store
        .set_with_anchor("arch/rest", "REST v1 endpoints", Some("src/api.rs:10"))
        .unwrap();
    store.set("arch/core", "Hexagonal Architecture").unwrap();

    // Both are active initially
    assert_eq!(store.dump().unwrap().len(), 2);
    let (rules, _) = store.context().unwrap();
    assert_eq!(rules.len(), 2);

    // 2. Archive arch/rest with a migration rationale
    let archived = store
        .archive("arch/rest", Some("Migrated to gRPC in v2"))
        .unwrap();
    assert!(archived);

    // Context and dump MUST only contain active rules (zero prompt token waste)
    let active_rules = store.dump().unwrap();
    assert_eq!(active_rules.len(), 1);
    assert_eq!(active_rules[0].0, "arch/core");

    let (context_rules, _) = store.context().unwrap();
    assert_eq!(context_rules.len(), 1);
    assert_eq!(context_rules[0].0, "arch/core");

    // All rules and archived rules queries
    let all_rules = store.dump_all().unwrap();
    assert_eq!(all_rules.len(), 2);

    let archived_only = store.dump_archived().unwrap();
    assert_eq!(archived_only.len(), 1);
    assert_eq!(archived_only[0].key, "arch/rest");
    assert_eq!(
        archived_only[0].archive_reason.as_deref(),
        Some("Migrated to gRPC in v2")
    );
    assert!(archived_only[0].is_archived());

    // 3. get_entry retrieves metadata
    let entry = store.get_entry("arch/rest").unwrap().expect("found");
    assert!(entry.is_archived());
    assert_eq!(
        entry.archive_reason.as_deref(),
        Some("Migrated to gRPC in v2")
    );

    // 4. BM25 search finds archived rule via reason and key
    let hits_reason = store.find("gRPC").unwrap();
    assert_eq!(hits_reason.len(), 1);
    assert_eq!(hits_reason[0].key, "arch/rest");
    assert!(hits_reason[0].is_archived());

    let hits_key = store.find("REST").unwrap();
    assert_eq!(hits_key.len(), 1);
    assert_eq!(hits_key[0].key, "arch/rest");
    assert!(hits_key[0].is_archived());

    // 5. Unarchive / reactivate
    let unarchived = store.unarchive("arch/rest").unwrap();
    assert!(unarchived);

    let active_again = store.dump().unwrap();
    assert_eq!(active_again.len(), 2);
    let entry_active = store.get_entry("arch/rest").unwrap().expect("found");
    assert!(!entry_active.is_archived());
    assert_eq!(entry_active.archive_reason, None);
}

#[test]
fn test_re_setting_archived_rule_reactivates_it() {
    let mut store = Store::open_in_memory().unwrap();
    store.set("db/engine", "MongoDB 4.2").unwrap();

    store
        .archive("db/engine", Some("Deprecated in favor of Postgres"))
        .unwrap();
    assert_eq!(store.dump().unwrap().len(), 0);

    // Explicitly setting a new value resets archived status
    store
        .set("db/engine", "PostgreSQL 16 with connection pooling")
        .unwrap();
    let entry = store.get_entry("db/engine").unwrap().expect("found");
    assert!(!entry.is_archived());
    assert_eq!(entry.archive_reason, None);
    assert_eq!(store.dump().unwrap().len(), 1);
}

#[test]
fn test_archive_cli_parsing_and_file_sync_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_archive_e2e_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "Architect"]);
    run_git(
        &temp_dir,
        &["config", "user.email", "architect@company.com"],
    );

    init_project(&temp_dir).unwrap();

    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let rules_path = temp_dir.join(".agent-rules");

    // Seed rules
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_with_anchor("arch/mvc", "Legacy MVC pattern", Some("src/old.rs:1"))
            .unwrap();
        store
            .set("arch/clean", "Clean Architecture with ports & adapters")
            .unwrap();
        store.sync_export(&rules_path).unwrap();
    }

    // Parse CLI archive commands
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "archive".into(),
            "arch/mvc".into(),
            "--reason".into(),
            "Replaced by Clean Architecture".into(),
        ])
        .unwrap(),
        Command::Archive {
            key: "arch/mvc".into(),
            reason: Some("Replaced by Clean Architecture".into()),
        }
    );

    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "unarchive".into(),
            "arch/mvc".into(),
        ])
        .unwrap(),
        Command::Unarchive {
            key: "arch/mvc".into(),
        }
    );

    // Perform archive via store and sync export
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .archive("arch/mvc", Some("Replaced by Clean Architecture"))
            .unwrap();
        store.sync_export(&rules_path).unwrap();
    }

    // Verify .agent-rules file content structure
    let file_content = fs::read_to_string(&rules_path).unwrap();
    assert!(file_content.contains("arch/clean = Clean Architecture with ports & adapters"));
    assert!(file_content.contains("# Archived Rules"));
    assert!(file_content.contains(
        "[archived] arch/mvc = Legacy MVC pattern (@ src/old.rs:1) --reason: Replaced by Clean Architecture"
    ));

    // Simulate collaborator editing .agent-rules externally with another archived rule
    let modified_rules = format!(
        "{}\n[archived] queue/sqs = AWS SQS --reason: Replaced by Kafka\n",
        file_content.trim()
    );
    fs::write(&rules_path, modified_rules).unwrap();

    // Sync from file into SQLite
    {
        let mut store = Store::open(&db_path, true).unwrap();
        let report = store.sync_with_file(&rules_path).unwrap();
        assert_eq!(report.total, 3);

        // Active should still be just 1
        assert_eq!(store.dump().unwrap().len(), 1);

        // Both archived rules should exist
        let archived = store.dump_archived().unwrap();
        assert_eq!(archived.len(), 2);

        let sqs_entry = store.get_entry("queue/sqs").unwrap().expect("found");
        assert!(sqs_entry.is_archived());
        assert_eq!(
            sqs_entry.archive_reason.as_deref(),
            Some("Replaced by Kafka")
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}
