use agent_mem::cli::{Command, parse_args};
use agent_mem::init::find_project_root_from;
use std::fs;

#[test]
fn test_parse_args() {
    // Empty args defaults to help
    assert_eq!(parse_args(vec!["agent-mem".into()]).unwrap(), Command::Help);

    // Help
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "--help".into()]).unwrap(),
        Command::Help
    );

    // Version
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "-v".into()]).unwrap(),
        Command::Version
    );

    // Init
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "init".into()]).unwrap(),
        Command::Init { path: None }
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "init".into(), "my-dir".into()]).unwrap(),
        Command::Init {
            path: Some("my-dir".into())
        }
    );

    // Set
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "set".into(),
            "k".into(),
            "v1".into(),
            "v2".into()
        ])
        .unwrap(),
        Command::Set {
            key: "k".into(),
            val: "v1 v2".into(),
            anchor: None,
            kind: None,
        }
    );

    // Set with anchor and kind
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "set".into(),
            "k".into(),
            "v1".into(),
            "--anchor".into(),
            "src/lib.rs:42".into(),
            "--kind".into(),
            "decision".into(),
        ])
        .unwrap(),
        Command::Set {
            key: "k".into(),
            val: "v1".into(),
            anchor: Some("src/lib.rs:42".into()),
            kind: Some("decision".into()),
        }
    );

    // Relate
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "relate".into(),
            "auth/jwt".into(),
            "mitigates".into(),
            "gotcha/replay".into(),
        ])
        .unwrap(),
        Command::Relate {
            source: "auth/jwt".into(),
            rel_type: "mitigates".into(),
            target: "gotcha/replay".into(),
        }
    );

    // Unrelate
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "unrelate".into(),
            "auth/jwt".into(),
            "mitigates".into(),
            "gotcha/replay".into(),
        ])
        .unwrap(),
        Command::Unrelate {
            source: "auth/jwt".into(),
            rel_type: "mitigates".into(),
            target: "gotcha/replay".into(),
        }
    );

    // Get
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "get".into(), "k".into()]).unwrap(),
        Command::Get { key: "k".into() }
    );

    // Del
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "del".into(), "k".into()]).unwrap(),
        Command::Del { key: "k".into() }
    );

    // Find
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "find".into(),
            "foo".into(),
            "bar".into()
        ])
        .unwrap(),
        Command::Find {
            query: "foo bar".into()
        }
    );

    // Session add
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "session".into(),
            "add".into(),
            "checkpoint".into()
        ])
        .unwrap(),
        Command::SessionAdd {
            summary: "checkpoint".into()
        }
    );

    // Session list
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "session".into(), "list".into()]).unwrap(),
        Command::SessionList
    );

    // Context
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "context".into()]).unwrap(),
        Command::Context {
            anchor: None,
            topic: None,
            limit: 20,
            diff: false,
            files: Vec::new(),
        }
    );

    // Context with filters
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "context".into(),
            "--anchor".into(),
            "src/auth.rs".into(),
            "--topic".into(),
            "auth/".into(),
            "--limit".into(),
            "5".into(),
        ])
        .unwrap(),
        Command::Context {
            anchor: Some("src/auth.rs".into()),
            topic: Some("auth/".into()),
            limit: 5,
            diff: false,
            files: Vec::new(),
        }
    );

    // Context with diff and files
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "context".into(),
            "--diff".into(),
            "--files".into(),
            "src/main.rs,src/lib.rs".into(),
        ])
        .unwrap(),
        Command::Context {
            anchor: None,
            topic: None,
            limit: 20,
            diff: true,
            files: vec!["src/main.rs".into(), "src/lib.rs".into()],
        }
    );

    // Mcp
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "mcp".into()]).unwrap(),
        Command::Mcp
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "--mcp".into()]).unwrap(),
        Command::Mcp
    );

    // Doctor
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "doctor".into()]).unwrap(),
        Command::Doctor
    );

    // Tui / Ui
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "tui".into()]).unwrap(),
        Command::Tui
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "ui".into()]).unwrap(),
        Command::Tui
    );

    // Mcp Install (auto)
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "mcp".into(), "install".into()]).unwrap(),
        Command::McpInstall { client: None }
    );

    // Mcp Install (claude)
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "mcp".into(),
            "install".into(),
            "claude".into()
        ])
        .unwrap(),
        Command::McpInstall {
            client: Some("claude".into())
        }
    );

    // Sync
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "sync".into()]).unwrap(),
        Command::Sync {
            file: None,
            export: false
        }
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "sync".into(), "--export".into()]).unwrap(),
        Command::Sync {
            file: None,
            export: true
        }
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "sync".into(), "team.rules".into()]).unwrap(),
        Command::Sync {
            file: Some("team.rules".into()),
            export: false
        }
    );
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "sync".into(),
            "team.rules".into(),
            "-e".into()
        ])
        .unwrap(),
        Command::Sync {
            file: Some("team.rules".into()),
            export: true
        }
    );

    // Projects
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "projects".into()]).unwrap(),
        Command::Projects { prune: false }
    );
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "projects".into(), "prune".into()]).unwrap(),
        Command::Projects { prune: true }
    );
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "projects".into(),
            "--prune".into()
        ])
        .unwrap(),
        Command::Projects { prune: true }
    );

    // Missing required args return error
    assert!(parse_args(vec!["agent-mem".into(), "get".into()]).is_err());
    assert!(parse_args(vec!["agent-mem".into(), "unknown-cmd".into()]).is_err());
}

#[test]
fn test_root_traversal() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sub_dir = temp_dir.join("a").join("b").join("c");
    fs::create_dir_all(&sub_dir).unwrap();

    // Create marker at temp_dir
    let git_marker = temp_dir.join(".git");
    fs::create_dir_all(&git_marker).unwrap();

    let found = find_project_root_from(&sub_dir);
    assert_eq!(found, temp_dir);

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_read_does_not_mutate_uninitialized_directory() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_uninit_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Get on uninitialized directory must fail without creating .agent-mem
    let cmd = Command::Get {
        key: "some_key".into(),
    };
    let res = agent_mem::cli::execute_command(cmd, &temp_dir);
    assert!(matches!(res, Err(agent_mem::error::Error::NotInitialized)));
    assert!(!temp_dir.join(".agent-mem").exists());

    // 2. Dump on uninitialized directory must also fail without creating .agent-mem
    let dump_cmd = Command::Dump;
    let dump_res = agent_mem::cli::execute_command(dump_cmd, &temp_dir);
    assert!(matches!(
        dump_res,
        Err(agent_mem::error::Error::NotInitialized)
    ));
    assert!(!temp_dir.join(".agent-mem").exists());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_get_nonexistent_returns_not_found() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // Initialize first so store exists
    agent_mem::init::init_project(&temp_dir).unwrap();

    let cmd = Command::Get {
        key: "nonexistent_rule".into(),
    };
    let res = agent_mem::cli::execute_command(cmd, &temp_dir);

    match res {
        Err(agent_mem::error::Error::NotFound(k)) => assert_eq!(k, "nonexistent_rule"),
        other => panic!("expected NotFound error, got {:?}", other),
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_init_creates_and_upgrades_trigger_block() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_init_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Initial init creates AGENTS.md with Persistent Memory Protocol
    let report = agent_mem::init::init_project(&temp_dir).unwrap();
    assert!(report.created_rule_file);
    let agents_md = temp_dir.join("AGENTS.md");
    assert!(agents_md.exists());
    let content = fs::read_to_string(&agents_md).unwrap();
    assert!(content.contains("<!-- agent-mem -->"));
    assert!(content.contains("Run `agent-mem context`"));
    assert!(content.contains("<!-- /agent-mem -->"));

    // 2. Second init is idempotent
    let report2 = agent_mem::init::init_project(&temp_dir).unwrap();
    assert!(!report2.created_rule_file);
    assert!(!report2.trigger_updated);

    // 3. Upgrades legacy one-liner
    fs::write(
        &agents_md,
        "# Project\n\nMemory: run `agent-mem get <key>` to check rules, `agent-mem set <key> \"<rule>\"` to save.\n",
    )
    .unwrap();
    let report3 = agent_mem::init::init_project(&temp_dir).unwrap();
    assert!(report3.trigger_updated);
    let upgraded = fs::read_to_string(&agents_md).unwrap();
    assert!(upgraded.contains("Run `agent-mem context`"));
    assert!(!upgraded.contains("Memory: run `agent-mem get"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_init_creates_rules_file_and_git_hooks() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_init_sync_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    // Simulate git repo
    fs::create_dir_all(temp_dir.join(".git").join("hooks")).unwrap();

    let report = agent_mem::init::init_project(&temp_dir).unwrap();
    assert!(report.rules_file_created);
    assert!(report.hook_configured);
    assert!(report.post_merge_configured);

    // Verify .agent-rules was created
    let rules_file = temp_dir.join(".agent-rules");
    assert!(rules_file.exists());
    let rules_content = fs::read_to_string(&rules_file).unwrap();
    assert!(rules_content.contains("# .agent-rules"));

    // Verify .git/hooks/post-merge was created
    let post_merge = temp_dir.join(".git").join("hooks").join("post-merge");
    assert!(post_merge.exists());
    let hook_content = fs::read_to_string(&post_merge).unwrap();
    assert!(hook_content.contains("agent-mem sync"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_sync_and_auto_sync_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_sync_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Initialize project
    agent_mem::init::init_project(&temp_dir).unwrap();
    let rules_file = temp_dir.join(".agent-rules");
    assert!(rules_file.exists());

    // 2. Setting a rule in CLI automatically updates .agent-rules
    let set_cmd = Command::Set {
        key: "arch/db".into(),
        val: "SQLite WAL".into(),
        anchor: Some("src/main.rs:10".into()),
        kind: None,
    };
    agent_mem::cli::execute_command(set_cmd, &temp_dir).unwrap();

    let content_after_set = fs::read_to_string(&rules_file).unwrap();
    assert!(content_after_set.contains("arch/db = SQLite WAL (@ src/main.rs:10)"));

    // 3. Deleting a rule in CLI automatically updates .agent-rules
    let del_cmd = Command::Del {
        key: "arch/db".into(),
    };
    agent_mem::cli::execute_command(del_cmd, &temp_dir).unwrap();

    let content_after_del = fs::read_to_string(&rules_file).unwrap();
    assert!(!content_after_del.contains("arch/db"));

    // 4. Manual sync from updated file into SQLite
    fs::write(
        &rules_file,
        "# Teammate added rules in Git\nteam/convention = Standard Rust 2024 (@ Cargo.toml:5)\n",
    )
    .unwrap();

    let sync_cmd = Command::Sync {
        file: None,
        export: false,
    };
    agent_mem::cli::execute_command(sync_cmd, &temp_dir).unwrap();

    // Verify rule is active in SQLite
    let get_cmd = Command::Get {
        key: "team/convention".into(),
    };
    assert!(agent_mem::cli::execute_command(get_cmd, &temp_dir).is_ok());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_init_git_worktree_hook_resolution_and_commondir() {
    let base_temp = std::env::temp_dir().join(format!(
        "agent_mem_worktree_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base_temp).unwrap();

    let main_repo = base_temp.join("main_repo");
    let main_git = main_repo.join(".git");
    let main_hooks = main_git.join("hooks");
    let worktree_meta = main_git.join("worktrees").join("feature_wt");
    fs::create_dir_all(&main_hooks).unwrap();
    fs::create_dir_all(&worktree_meta).unwrap();

    // Create commondir pointing to ../..
    fs::write(worktree_meta.join("commondir"), "../..\n").unwrap();

    // Create the worktree checkout
    let worktree_dir = base_temp.join("feature_wt");
    fs::create_dir_all(&worktree_dir).unwrap();
    fs::write(
        worktree_dir.join(".git"),
        format!("gitdir: {}\n", worktree_meta.display()),
    )
    .unwrap();

    // Initialize agent-mem in the worktree
    let report = agent_mem::init::init_project(&worktree_dir).unwrap();
    assert!(report.hook_configured);
    assert!(report.post_merge_configured);
    assert!(report.post_checkout_configured);
    assert!(report.post_rewrite_configured);

    // Verify hooks were installed in main_repo/.git/hooks
    let post_commit = main_hooks.join("post-commit");
    assert!(post_commit.exists());
    let hook_content = fs::read_to_string(&post_commit).unwrap();
    assert!(
        hook_content.contains("agent-mem hook post-commit")
            || hook_content.contains("agent-mem session add")
    );

    let post_rewrite = main_hooks.join("post-rewrite");
    assert!(post_rewrite.exists());
    let hook_content = fs::read_to_string(&post_rewrite).unwrap();
    assert!(hook_content.contains("agent-mem sync"));

    // Verify inspect_git_health detects it as a valid git repo with active hooks
    let health = agent_mem::init::inspect_git_health(&worktree_dir);
    assert!(health.is_git_repo);
    assert!(health.post_commit_active);
    assert!(health.post_merge_active);
    assert!(health.post_checkout_active);
    assert!(health.post_rewrite_active);

    let _ = fs::remove_dir_all(&base_temp);
}

#[test]
fn test_init_git_submodule_hook_resolution() {
    let base_temp = std::env::temp_dir().join(format!(
        "agent_mem_submodule_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base_temp).unwrap();

    let main_repo = base_temp.join("main_repo");
    let submodule_git = main_repo.join(".git").join("modules").join("nested_sub");
    fs::create_dir_all(&submodule_git).unwrap();

    let submodule_dir = main_repo.join("nested_sub");
    fs::create_dir_all(&submodule_dir).unwrap();
    fs::write(
        submodule_dir.join(".git"),
        format!("gitdir: {}\n", submodule_git.display()),
    )
    .unwrap();

    let report = agent_mem::init::init_project(&submodule_dir).unwrap();
    assert!(report.hook_configured);
    assert!(report.post_merge_configured);

    // Hooks should be installed in submodule's git modules dir
    let post_commit = submodule_git.join("hooks").join("post-commit");
    assert!(post_commit.exists());

    let health = agent_mem::init::inspect_git_health(&submodule_dir);
    assert!(health.is_git_repo);
    assert!(health.post_commit_active);

    let _ = fs::remove_dir_all(&base_temp);
}
