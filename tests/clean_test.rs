use agent_mem::cli::{Command, execute_command, parse_args};
use agent_mem::hook::run_post_commit;
use agent_mem::init::init_project;
use agent_mem::store::{Store, extract_anchor_paths};
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let mut full_args = vec!["-c", "core.hooksPath=/dev/null"];
    full_args.extend_from_slice(args);
    let output = StdCommand::new("git")
        .current_dir(dir)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute git {:?}: {}", args, e));

    if !output.status.success() {
        panic!(
            "git {:?} failed with exit code {:?}\nStdout: {}\nStderr: {}",
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn test_extract_anchor_paths_normalization() {
    let paths = extract_anchor_paths("@ src/auth/jwt.rs:42, src/models/user.rs:10");
    assert_eq!(paths, vec!["src/auth/jwt.rs", "src/models/user.rs"]);

    let paths = extract_anchor_paths("@ src\\win\\path.rs:1");
    assert_eq!(paths, vec!["src/win/path.rs"]);

    let paths = extract_anchor_paths("./src/local.rs:99");
    assert_eq!(paths, vec!["src/local.rs"]);

    let paths = extract_anchor_paths("   @   ");
    assert!(paths.is_empty());
}

#[test]
fn test_clean_zombies_dry_run_and_execution() {
    let temp_dir = std::env::temp_dir().join(format!("agent_mem_clean_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src")).unwrap();

    // Create an existing file
    fs::write(temp_dir.join("src/live.rs"), "pub fn live() {}").unwrap();
    // Do NOT create src/dead.rs

    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let mut store = Store::open(&db_path, true).unwrap();

    // Set 3 rules:
    // 1. live anchor
    store
        .set_entry(
            "rule/live",
            "active rule",
            Some("src/live.rs:1"),
            Some("rule"),
        )
        .unwrap();
    // 2. dead anchor (file does not exist)
    store
        .set_entry(
            "gotcha/dead",
            "dead gotcha",
            Some("src/dead.rs:10"),
            Some("gotcha"),
        )
        .unwrap();
    // 3. no anchor
    store
        .set_entry("pattern/general", "unanchored rule", None, Some("pattern"))
        .unwrap();

    // Dry run
    let report_dry = store.clean_zombies(&temp_dir, true).unwrap();
    assert_eq!(report_dry.len(), 1);
    assert_eq!(report_dry[0].key, "gotcha/dead");
    assert_eq!(report_dry[0].missing_paths, vec!["src/dead.rs"]);
    assert!(!report_dry[0].archived);

    // Verify DB was NOT mutated
    let dead_entry = store.get_entry("gotcha/dead").unwrap().unwrap();
    assert!(dead_entry.archived_at.is_none());

    // Actual execution
    let report_act = store.clean_zombies(&temp_dir, false).unwrap();
    assert_eq!(report_act.len(), 1);
    assert_eq!(report_act[0].key, "gotcha/dead");
    assert_eq!(report_act[0].missing_paths, vec!["src/dead.rs"]);
    assert!(report_act[0].archived);

    // Verify DB WAS mutated: archived_at is set, archive_reason is set
    let dead_entry = store.get_entry("gotcha/dead").unwrap().unwrap();
    assert!(dead_entry.archived_at.is_some());
    assert_eq!(
        dead_entry.archive_reason.as_deref(),
        Some("file_deleted: src/dead.rs")
    );

    // Active dump should only contain live & general
    let active = store.dump_active().unwrap();
    assert_eq!(active.len(), 2);
    assert_eq!(active[0].key, "pattern/general");
    assert_eq!(active[1].key, "rule/live");

    // Second run should find 0 zombies
    let report_clean = store.clean_zombies(&temp_dir, false).unwrap();
    assert!(report_clean.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_clean_cli_parsing_and_execution() {
    let cmd = parse_args(vec!["agent-mem".into(), "clean".into()]).unwrap();
    assert_eq!(cmd, Command::Clean { dry_run: false });

    let cmd_dry = parse_args(vec!["agent-mem".into(), "clean".into(), "--dry-run".into()]).unwrap();
    assert_eq!(cmd_dry, Command::Clean { dry_run: true });

    let cmd_n = parse_args(vec!["agent-mem".into(), "clean".into(), "-n".into()]).unwrap();
    assert_eq!(cmd_n, Command::Clean { dry_run: true });

    // Test execution with .agent-rules sync
    let temp_dir = std::env::temp_dir().join(format!("agent_mem_clean_cli_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    init_project(&temp_dir).unwrap();

    // Set rule with missing file
    execute_command(
        Command::Set {
            key: "gotcha/obsolete".into(),
            val: "stale rule".into(),
            anchor: Some("src/ghost.rs:1".into()),
            kind: Some("gotcha".into()),
        },
        &temp_dir,
    )
    .unwrap();

    let rules_before = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(rules_before.contains("gotcha/obsolete"));

    // Run clean via CLI
    execute_command(Command::Clean { dry_run: false }, &temp_dir).unwrap();

    // Verify .agent-rules was automatically synced: moved from active to [archived]
    let rules_after = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(
        !rules_after
            .lines()
            .any(|l| l.starts_with("[gotcha] gotcha/obsolete ="))
    );
    assert!(rules_after.contains("[archived] [gotcha] gotcha/obsolete ="));
    assert!(rules_after.contains("--reason: file_deleted: src/ghost.rs"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_clean_in_post_commit_hook() {
    let temp_dir =
        std::env::temp_dir().join(format!("agent_mem_clean_hook_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src")).unwrap();

    run_git(&temp_dir, &["init"]);
    run_git(&temp_dir, &["config", "user.name", "AgentMem Test"]);
    run_git(
        &temp_dir,
        &["config", "user.email", "test@agentmem.internal"],
    );

    init_project(&temp_dir).unwrap();

    // Create file src/legacy.rs and commit
    fs::write(temp_dir.join("src/legacy.rs"), "// legacy code").unwrap();
    run_git(&temp_dir, &["add", "."]);
    run_git(&temp_dir, &["commit", "-m", "chore: add legacy code"]);

    // Manually add a rule anchored to src/legacy.rs
    execute_command(
        Command::Set {
            key: "gotcha/legacy".into(),
            val: "watch out for legacy quirks".into(),
            anchor: Some("src/legacy.rs:1".into()),
            kind: Some("gotcha".into()),
        },
        &temp_dir,
    )
    .unwrap();

    // Now delete src/legacy.rs and commit with git
    fs::remove_file(temp_dir.join("src/legacy.rs")).unwrap();
    run_git(&temp_dir, &["add", "-A"]);
    run_git(&temp_dir, &["commit", "-m", "refactor: drop legacy code"]);

    // Run post-commit hook (or inspect result if git hook already ran during commit)
    let report = run_post_commit(&temp_dir, false).unwrap().unwrap();

    // Verify zombie was archived either by git's post-commit hook execution or by run_post_commit
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let store = Store::open(&db_path, false).unwrap();
    let entry = store.get_entry("gotcha/legacy").unwrap().unwrap();
    assert!(entry.archived_at.is_some());
    assert_eq!(
        entry.archive_reason.as_deref(),
        Some("file_deleted: src/legacy.rs")
    );

    assert_eq!(report.zombies.len(), 1);
    assert_eq!(report.zombies[0].key, "gotcha/legacy");
    assert_eq!(report.zombies[0].missing_paths, vec!["src/legacy.rs"]);
    assert!(report.zombies[0].archived);

    // Verify .agent-rules was synced and rule moved to [archived]
    let rules = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(
        !rules
            .lines()
            .any(|l| l.starts_with("[gotcha] gotcha/legacy ="))
    );
    assert!(rules.contains("[archived] [gotcha] gotcha/legacy ="));
    assert!(rules.contains("--reason: file_deleted: src/legacy.rs"));

    let _ = fs::remove_dir_all(&temp_dir);
}
