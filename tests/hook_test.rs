use agent_mem::cli::{Command, execute_command, parse_args};
use agent_mem::hook::{
    EntityKind, extract_anchors, is_relevant_source_file, parse_conventional_commit,
    parse_trailers, run_post_commit, slugify,
};
use agent_mem::init::init_project;
use agent_mem::store::Store;
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
fn test_conventional_commit_parsing() {
    // 1. fix -> gotcha
    let parsed = parse_conventional_commit("fix(auth): handle token expiration", "");
    assert_eq!(parsed.commit_type.as_deref(), Some("fix"));
    assert_eq!(parsed.scope.as_deref(), Some("auth"));
    assert!(!parsed.is_breaking);
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Gotcha);
    assert_eq!(entity.key, "gotcha/auth/handle-token-expiration");
    assert_eq!(entity.val, "handle token expiration");

    // 2. bugfix -> gotcha
    let parsed = parse_conventional_commit("bugfix(db): connection pool timeout", "");
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Gotcha);
    assert_eq!(entity.key, "gotcha/db/connection-pool-timeout");

    // 3. feat -> decision
    let parsed = parse_conventional_commit("feat(api): add v2 GraphQL endpoint", "");
    assert_eq!(parsed.commit_type.as_deref(), Some("feat"));
    assert_eq!(parsed.scope.as_deref(), Some("api"));
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Decision);
    assert_eq!(entity.key, "decision/api/add-v2-graphql-endpoint");
    assert_eq!(entity.val, "add v2 GraphQL endpoint");

    // 4. refactor & perf -> pattern
    let parsed = parse_conventional_commit("refactor(core): extract engine module", "");
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Pattern);
    assert_eq!(entity.key, "pattern/core/extract-engine-module");

    let parsed = parse_conventional_commit("perf(query): index memories anchor", "");
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Pattern);
    assert_eq!(entity.key, "pattern/query/index-memories-anchor");

    // 5. chore, docs, ci, test, style -> skipped
    assert!(
        parse_conventional_commit("chore: bump dependencies", "")
            .entity
            .is_none()
    );
    assert!(
        parse_conventional_commit("docs(readme): update setup instructions", "")
            .entity
            .is_none()
    );
    assert!(
        parse_conventional_commit("ci: add github action", "")
            .entity
            .is_none()
    );
    assert!(
        parse_conventional_commit("test: add unit tests", "")
            .entity
            .is_none()
    );
    assert!(
        parse_conventional_commit("style: format with rustfmt", "")
            .entity
            .is_none()
    );

    // 6. breaking change via bang
    let parsed = parse_conventional_commit("feat(api)!: switch to protobuf serialization", "");
    assert!(parsed.is_breaking);
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Rule);
    assert_eq!(
        entity.key,
        "architecture/api/switch-to-protobuf-serialization"
    );
    assert_eq!(entity.val, "switch to protobuf serialization");

    // 7. breaking change via subject prefix
    let parsed = parse_conventional_commit("BREAKING CHANGE: drop node 14 support", "");
    assert!(parsed.is_breaking);
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Rule);
    assert_eq!(entity.key, "architecture/drop-node-14-support");
    assert_eq!(entity.val, "drop node 14 support");

    // 8. fix with body description
    let parsed = parse_conventional_commit(
        "fix(net): resolve udp packet drop",
        "Packets were silently dropped on timeout.",
    );
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Gotcha);
    assert_eq!(entity.key, "gotcha/net/resolve-udp-packet-drop");
    assert_eq!(
        entity.val,
        "resolve udp packet drop: Packets were silently dropped on timeout."
    );
}

#[test]
fn test_trailers_override_and_relations() {
    let subject = "feat(auth): migrate to jose library";
    let body = "\
Decision: We switched from jsonwebtoken to jose for native edge/ESM runtime support.
Key: decision/auth
Mitigates: gotcha/auth-node-crypto
Relates-To: decision/edge-runtime
Depends-On: architecture/node-crypto
Supersedes: decision/old-auth";

    let parsed = parse_conventional_commit(subject, body);
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Decision);
    assert_eq!(entity.key, "decision/auth");
    assert_eq!(
        entity.val,
        "We switched from jsonwebtoken to jose for native edge/ESM runtime support."
    );

    let rels = parsed.relations;
    assert_eq!(rels.len(), 4);
    assert!(rels.contains(&(
        "mitigates".to_string(),
        "gotcha/auth-node-crypto".to_string()
    )));
    assert!(rels.contains(&(
        "relates_to".to_string(),
        "decision/edge-runtime".to_string()
    )));
    assert!(rels.contains(&(
        "depends_on".to_string(),
        "architecture/node-crypto".to_string()
    )));
    assert!(rels.contains(&("supersedes".to_string(), "decision/old-auth".to_string())));

    // Explicit trailer in a chore commit overrides skip behavior
    let chore_subj = "chore(ci): update workflow";
    let chore_body = "Rule: Never bypass branch protection rules.\nKey: architecture/ci";
    let chore_parsed = parse_conventional_commit(chore_subj, chore_body);
    let chore_entity = chore_parsed.entity.unwrap();
    assert_eq!(chore_entity.kind, EntityKind::Rule);
    assert_eq!(chore_entity.key, "architecture/ci");
    assert_eq!(chore_entity.val, "Never bypass branch protection rules.");

    // Gotcha and Pattern trailers
    let gotcha_trailers = parse_trailers("Gotcha: SQLite busy_timeout must be at least 5000ms.");
    assert_eq!(gotcha_trailers.explicit_kind, Some(EntityKind::Gotcha));
    assert_eq!(
        gotcha_trailers.explicit_content.as_deref(),
        Some("SQLite busy_timeout must be at least 5000ms.")
    );

    let pattern_trailers = parse_trailers("Pattern: Encapsulate rusqlite transactions.");
    assert_eq!(pattern_trailers.explicit_kind, Some(EntityKind::Pattern));
    assert_eq!(
        pattern_trailers.explicit_content.as_deref(),
        Some("Encapsulate rusqlite transactions.")
    );
}

#[test]
fn test_anchor_resolution_logic() {
    assert!(is_relevant_source_file("src/main.rs"));
    assert!(is_relevant_source_file("lib/utils.py"));
    assert!(!is_relevant_source_file(".agent-rules"));
    assert!(!is_relevant_source_file(".agent-mem/mem.db"));
    assert!(!is_relevant_source_file(".gitignore"));
    assert!(!is_relevant_source_file(".gitattributes"));
    assert!(!is_relevant_source_file("Cargo.lock"));
    assert!(!is_relevant_source_file("package-lock.json"));

    let files = vec![
        "src/auth.rs".to_string(),
        ".agent-rules".to_string(),
        "src/token.rs".to_string(),
        "Cargo.lock".to_string(),
        "src/crypto.rs".to_string(),
        "src/extra.rs".to_string(),
    ];

    let anchors = extract_anchors(&files);
    assert_eq!(
        anchors.as_deref(),
        Some("src/auth.rs:1, src/token.rs:1, src/crypto.rs:1")
    );
}

#[test]
fn test_post_commit_hook_execution_full_flow() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_hook_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Initialize real Git repo
    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "HookTester"]);
    run_git(&temp_dir, &["config", "user.email", "hook@agent-mem.dev"]);

    // 2. Initialize agent-mem
    let report = init_project(&temp_dir).unwrap();
    assert!(report.hook_configured);
    assert!(report.rules_file_created);

    // 3. Create a source file and commit
    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("db.rs"),
        "pub fn connect() -> Result<(), ()> { Ok(()) }",
    )
    .unwrap();

    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &["commit", "-m", "fix(db): resolve connection pool leak"],
    );

    // 4. Run post-commit hook
    let hook_report = run_post_commit(&temp_dir, false).unwrap().unwrap();
    assert_eq!(
        hook_report.commit_subject,
        "fix(db): resolve connection pool leak"
    );
    assert!(!hook_report.commit_hash.is_empty());

    let entity = hook_report.entity_captured.unwrap();
    assert_eq!(entity.key, "gotcha/db/resolve-connection-pool-leak");
    assert_eq!(entity.val, "resolve connection pool leak");
    assert_eq!(entity.kind, "gotcha");
    assert_eq!(entity.anchor.as_deref(), Some("src/db.rs:1"));
    assert!(hook_report.rules_synced);

    // 5. Verify SQLite database directly
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let store = Store::open(&db_path, false).unwrap();
    let entry = store
        .get_entry("gotcha/db/resolve-connection-pool-leak")
        .unwrap()
        .unwrap();
    assert_eq!(entry.val, "resolve connection pool leak");
    assert_eq!(entry.kind, "gotcha");
    assert_eq!(entry.anchor.as_deref(), Some("src/db.rs:1"));

    let sessions = store.session_list(5).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].1.contains("resolve connection pool leak"));

    // 6. Verify .agent-rules was updated
    let rules_file = temp_dir.join(".agent-rules");
    assert!(rules_file.exists());
    let rules_text = fs::read_to_string(&rules_file).unwrap();
    assert!(rules_text.contains(
        "[gotcha] gotcha/db/resolve-connection-pool-leak = resolve connection pool leak (@ src/db.rs:1)"
    ));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_post_commit_hook_with_relations_and_sync() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_hook_rel_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "HookTester"]);
    run_git(&temp_dir, &["config", "user.email", "hook@agent-mem.dev"]);

    init_project(&temp_dir).unwrap();

    // Create target memory first
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set_entry(
                "gotcha/auth-node-crypto",
                "Node crypto breaks on cloudflare workers",
                None,
                Some("gotcha"),
            )
            .unwrap();
    }

    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("auth.rs"), "// auth module").unwrap();

    run_git(&temp_dir, &["add", "."]);
    let msg = "feat(auth): migrate to jose library\n\nDecision: We switched from jsonwebtoken to jose for native edge/ESM runtime support.\nKey: decision/auth\nMitigates: gotcha/auth-node-crypto";
    run_git(&temp_dir, &["commit", "-m", msg]);

    let hook_report = run_post_commit(&temp_dir, false).unwrap().unwrap();
    assert_eq!(hook_report.relations.len(), 1);
    assert_eq!(
        hook_report.relations[0],
        (
            "decision/auth".to_string(),
            "mitigates".to_string(),
            "gotcha/auth-node-crypto".to_string()
        )
    );

    let store = Store::open(&db_path, false).unwrap();
    let rels = store.get_relations("decision/auth").unwrap();
    assert_eq!(rels.len(), 1);
    assert_eq!(
        rels[0],
        (
            "mitigates".to_string(),
            "gotcha/auth-node-crypto".to_string()
        )
    );

    let rules_file = temp_dir.join(".agent-rules");
    let rules_text = fs::read_to_string(&rules_file).unwrap();
    assert!(rules_text.contains("[rel] decision/auth -> mitigates -> gotcha/auth-node-crypto"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hook_filters_noise_and_archives_explicitly_superseded_decision() {
    let root_path = std::env::temp_dir().join(format!(
        "agent_mem_hook_quality_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root_path).unwrap();
    let root = root_path.as_path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "HookTester"]);
    run_git(root, &["config", "user.email", "hook@agent-mem.dev"]);
    init_project(root).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();

    fs::write(root.join("src/cache.rs"), "// original").unwrap();
    run_git(root, &["add", "src/cache.rs"]);
    run_git(
        root,
        &["commit", "-m", "feat(cache): add redis lru backend"],
    );
    let first = run_post_commit(root, false).unwrap().unwrap();
    assert!(first.entity_captured.is_some());

    let repeated = run_post_commit(root, true).unwrap().unwrap();
    assert_eq!(repeated.skip_reason, Some("duplicate memory"));
    assert!(repeated.entity_captured.is_none());
    let replay = run_post_commit(root, false).unwrap().unwrap();
    assert_eq!(replay.skip_reason, Some("duplicate memory"));
    let store = Store::open(&root.join(".agent-mem/mem.db"), false).unwrap();
    assert_eq!(store.session_list(20).unwrap().len(), 1);

    fs::write(root.join("src/cache.rs"), "// duplicate").unwrap();
    run_git(root, &["add", "src/cache.rs"]);
    run_git(
        root,
        &["commit", "-m", "feat(storage): add redis lru backend"],
    );
    let duplicate = run_post_commit(root, false).unwrap().unwrap();
    assert_eq!(duplicate.skip_reason, Some("duplicate memory"));
    assert!(duplicate.entity_captured.is_none());

    fs::write(root.join("src/cache.rs"), "// temporary").unwrap();
    run_git(root, &["add", "src/cache.rs"]);
    run_git(root, &["commit", "-m", "feat(cache): wip try new backend"]);
    let transient = run_post_commit(root, false).unwrap().unwrap();
    assert_eq!(transient.skip_reason, Some("transient commit summary"));
    assert!(transient.entity_captured.is_none());

    fs::write(root.join("src/cache.rs"), "// replacement").unwrap();
    run_git(root, &["add", "src/cache.rs"]);
    run_git(
        root,
        &[
            "commit",
            "-m",
            "feat(cache): move to durable backend",
            "-m",
            "Decision: Use a durable backend for cache persistence.\nKey: decision/cache/durable-backend\nSupersedes: decision/cache/add-redis-lru-backend",
        ],
    );
    let replacement = run_post_commit(root, false).unwrap().unwrap();
    assert_eq!(
        replacement.entity_captured.unwrap().key,
        "decision/cache/durable-backend"
    );
    let store = Store::open(&root.join(".agent-mem/mem.db"), false).unwrap();
    let old = store
        .get_entry("decision/cache/add-redis-lru-backend")
        .unwrap()
        .unwrap();
    assert!(old.archived_at.is_some());
    assert_eq!(
        old.archive_reason.as_deref(),
        Some("superseded_by: decision/cache/durable-backend")
    );
    assert_eq!(
        store
            .get_relations("decision/cache/durable-backend")
            .unwrap(),
        vec![(
            "supersedes".to_string(),
            "decision/cache/add-redis-lru-backend".to_string()
        )]
    );
    assert!(
        store
            .get_entry("decision/cache/wip-try-new-backend")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_entry("decision/storage/add-redis-lru-backend")
            .unwrap()
            .is_none()
    );
    drop(store);
    fs::write(root.join("src/cache.rs"), "// old approach resurfaced").unwrap();
    run_git(root, &["add", "src/cache.rs"]);
    run_git(
        root,
        &["commit", "-m", "feat(cache): add redis lru backend"],
    );
    let obsolete = run_post_commit(root, false).unwrap().unwrap();
    assert_eq!(
        obsolete.skip_reason,
        Some("archived memory needs explicit replacement")
    );
    let reopened = Store::open(&root.join(".agent-mem/mem.db"), false).unwrap();
    assert!(
        reopened
            .get_entry("decision/cache/add-redis-lru-backend")
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    drop(reopened);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_commit_supersedes_requires_existing_target_atomically() {
    let root_path = std::env::temp_dir().join(format!(
        "agent_mem_hook_atomic_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root_path).unwrap();
    let db = root_path.join("mem.db");
    let mut store = Store::open(&db, true).unwrap();
    let result = store.capture_commit(
        Some((
            "decision/new",
            "Use the new adapter",
            None,
            Some("decision"),
        )),
        &[("decision/new", "supersedes", "decision/missing")],
        "[abcdef0] feat: change adapter",
    );
    assert!(result.is_err());
    assert!(store.get_entry("decision/new").unwrap().is_none());
    assert!(store.session_list(20).unwrap().is_empty());
    drop(store);
    let _ = fs::remove_dir_all(root_path);
}

#[test]
fn test_dry_run_mode() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_dry_run_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "HookTester"]);
    run_git(&temp_dir, &["config", "user.email", "hook@agent-mem.dev"]);

    init_project(&temp_dir).unwrap();

    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("cache.rs"), "pub struct Cache;").unwrap();
    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &["commit", "-m", "feat(cache): add redis lru backend"],
    );

    // Execute with dry_run = true
    let hook_report = run_post_commit(&temp_dir, true).unwrap().unwrap();
    assert!(hook_report.dry_run);
    assert!(hook_report.session_id.is_none());
    assert!(!hook_report.rules_synced);
    let entity = hook_report.entity_captured.unwrap();
    assert_eq!(entity.key, "decision/cache/add-redis-lru-backend");

    // Verify nothing written to DB
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let store = Store::open(&db_path, false).unwrap();
    assert!(
        store
            .get_entry("decision/cache/add-redis-lru-backend")
            .unwrap()
            .is_none()
    );
    let sessions = store.session_list(5).unwrap();
    assert!(sessions.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_non_git_and_empty_repo_graceful() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_non_git_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // Non-git directory -> Ok(None)
    let res = run_post_commit(&temp_dir, false).unwrap();
    assert!(res.is_none());

    // Empty git repo without any commits -> Ok(None)
    run_git(&temp_dir, &["init", "-b", "main"]);
    let res2 = run_post_commit(&temp_dir, false).unwrap();
    assert!(res2.is_none());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_hook_subcommand_parsing_and_execution() {
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "hook".into(),
            "post-commit".into()
        ])
        .unwrap(),
        Command::HookPostCommit { dry_run: false }
    );
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "hook".into(),
            "post-commit".into(),
            "--dry-run".into()
        ])
        .unwrap(),
        Command::HookPostCommit { dry_run: true }
    );
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "hook".into(),
            "post-commit".into(),
            "-n".into()
        ])
        .unwrap(),
        Command::HookPostCommit { dry_run: true }
    );

    // Invalid hook subcommand
    assert!(parse_args(vec!["agent-mem".into(), "hook".into(), "pre-push".into()]).is_err());

    // Execution in empty directory exits cleanly (Ok(()))
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_hook_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    assert!(execute_command(Command::HookPostCommit { dry_run: false }, &temp_dir).is_ok());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_slugify_token_efficiency_and_bounds() {
    // Standard phrases
    assert_eq!(slugify("validate JWT issuer"), "validate-jwt-issuer");
    assert_eq!(
        slugify("use constant-time comparison"),
        "use-constant-time-comparison"
    );

    // Special characters & punctuation
    assert_eq!(
        slugify("handle EOF (on socket closed!)"),
        "handle-eof-on-socket"
    );
    assert_eq!(
        slugify("fix: buffer overflow in parser"),
        "fix-buffer-overflow-in"
    );

    // Long sentences are bounded to save prompt tokens (< 32 chars)
    let long = "this is a very long commit summary describing architectural changes in detail";
    let slug = slugify(long);
    assert!(slug.len() <= 32);
    assert_eq!(slug, "this-is-a-very");

    // Empty and non-alphanumeric fallback
    assert_eq!(slugify(""), "general");
    assert_eq!(slugify("   ... --- !!!  "), "general");
}

#[test]
fn test_distinct_commits_on_same_scope_coexist_without_overwrite() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_coexist_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    run_git(&temp_dir, &["init"]);
    run_git(&temp_dir, &["config", "user.name", "Test Agent"]);
    run_git(
        &temp_dir,
        &["config", "user.email", "agent@antigravity.test"],
    );

    init_project(&temp_dir).unwrap();

    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();

    // Commit 1: fix(auth): validate jwt issuer
    fs::write(src_dir.join("jwt.rs"), "pub fn check_jwt() {}").unwrap();
    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &["commit", "-m", "fix(auth): validate jwt issuer"],
    );
    let report1 = run_post_commit(&temp_dir, false).unwrap().unwrap();
    let ent1 = report1.entity_captured.unwrap();
    assert_eq!(ent1.key, "gotcha/auth/validate-jwt-issuer");

    // Commit 2: fix(auth): use constant-time comparison
    fs::write(src_dir.join("crypto.rs"), "pub fn compare() {}").unwrap();
    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &["commit", "-m", "fix(auth): use constant-time comparison"],
    );
    let report2 = run_post_commit(&temp_dir, false).unwrap().unwrap();
    let ent2 = report2.entity_captured.unwrap();
    assert_eq!(ent2.key, "gotcha/auth/use-constant-time-comparison");

    // Verify BOTH distinct gotchas coexist in the SQLite store and are not overwritten!
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let store = Store::open(&db_path, false).unwrap();
    let r1 = store
        .get_entry("gotcha/auth/validate-jwt-issuer")
        .unwrap()
        .unwrap();
    let r2 = store
        .get_entry("gotcha/auth/use-constant-time-comparison")
        .unwrap()
        .unwrap();
    assert_eq!(r1.val, "validate jwt issuer");
    assert_eq!(r2.val, "use constant-time comparison");

    // Verify sessions recorded origin commit hashes
    let sessions = store.session_list(5).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|s| s.1.contains("validate jwt issuer") && s.1.starts_with('['))
    );
    assert!(
        sessions
            .iter()
            .any(|s| s.1.contains("use constant-time comparison") && s.1.starts_with('['))
    );

    // Commit 3: Explicit Key trailer allows deliberate update/deduplication
    fs::write(src_dir.join("jwt.rs"), "pub fn check_jwt_v2() {}").unwrap();
    run_git(&temp_dir, &["add", "."]);
    let msg = "fix(auth): update jwt issuer check\n\nKey: gotcha/auth/validate-jwt-issuer";
    run_git(&temp_dir, &["commit", "-m", msg]);
    let report3 = run_post_commit(&temp_dir, false).unwrap().unwrap();
    assert_eq!(
        report3.entity_captured.unwrap().key,
        "gotcha/auth/validate-jwt-issuer"
    );

    // Total gotchas remain 2 because Commit 3 explicitly targeted and updated the first one
    let all_memories = store.dump_all().unwrap();
    assert_eq!(all_memories.len(), 2);
    let updated_r1 = store
        .get_entry("gotcha/auth/validate-jwt-issuer")
        .unwrap()
        .unwrap();
    assert_eq!(updated_r1.val, "update jwt issuer check");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_commit_with_explicit_trailer_without_scope_is_captured() {
    let parsed = parse_conventional_commit(
        "chore: security patch",
        "Gotcha: SQLite busy_timeout must be at least 5000ms.\nKey: gotcha/db-timeout",
    );
    let entity = parsed.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Gotcha);
    assert_eq!(entity.key, "gotcha/db-timeout");
    assert_eq!(entity.val, "SQLite busy_timeout must be at least 5000ms.");

    let feat_parsed = parse_conventional_commit(
        "chore: infrastructure update",
        "Decision: Use argon2id for all password hashing.",
    );
    let feat_entity = feat_parsed.entity.unwrap();
    assert_eq!(feat_entity.kind, EntityKind::Decision);
    assert_eq!(feat_entity.key, "decision/infrastructure-update");
    assert_eq!(feat_entity.val, "Use argon2id for all password hashing.");
}

#[test]
fn test_slugify_folds_diacritics_and_filters_ecosystem_lockfiles() {
    use agent_mem::hook::{extract_anchors, slugify};

    assert_eq!(
        slugify("validación de sesión única"),
        "validacion-de-sesion-unica"
    );
    let parsed = parse_conventional_commit("fix(auth): validación de sesión", "");
    assert_eq!(
        parsed.entity.unwrap().key,
        "gotcha/auth/validacion-de-sesion"
    );

    let files = vec![
        "uv.lock".to_string(),
        "go.sum".to_string(),
        "bun.lockb".to_string(),
        "packages/web/package-lock.json".to_string(),
        "services/api/src/auth.py".to_string(),
    ];
    assert_eq!(
        extract_anchors(&files).as_deref(),
        Some("services/api/src/auth.py:1")
    );
}

#[test]
fn test_custom_trailers_dynamic_entity_kind() {
    let commit = parse_conventional_commit(
        "chore: update upload handler",
        "Security: Always sanitize SVG uploads to prevent stored XSS attacks.\nKey: security/sanitize-svg-uploads",
    );
    let entity = commit
        .entity
        .expect("custom trailer should be parsed as entity");
    match entity.kind {
        EntityKind::Custom(ref s) => assert_eq!(s, "security"),
        _ => panic!(
            "Expected EntityKind::Custom('security'), got {:?}",
            entity.kind
        ),
    }
    assert_eq!(entity.key, "security/sanitize-svg-uploads");
    assert_eq!(
        entity.val,
        "Always sanitize SVG uploads to prevent stored XSS attacks."
    );
}

#[test]
fn test_diff_directive_scanning_across_languages() {
    use agent_mem::hook::scan_diff_for_directives;

    let diff = r#"
diff --git a/src/auth/jwt.rs b/src/auth/jwt.rs
index 1234567..89abcdef 100644
--- a/src/auth/jwt.rs
+++ b/src/auth/jwt.rs
@@ -10,0 +25,3 @@
+// DECISION: enforce ed25519 signatures for all session tokens
+pub fn verify() {}
"#;
    let found = scan_diff_for_directives(diff);
    assert!(found.is_some());
    let (entity, anchor) = found.unwrap();
    assert_eq!(entity.kind, EntityKind::Decision);
    assert_eq!(
        entity.val,
        "enforce ed25519 signatures for all session tokens"
    );
    assert_eq!(anchor.as_deref(), Some("src/auth/jwt.rs:25"));

    // Python comment (#)
    let py_diff = r#"
diff --git a/backend/tasks/worker.py b/backend/tasks/worker.py
--- a/backend/tasks/worker.py
+++ b/backend/tasks/worker.py
@@ -1,0 +50,3 @@
+# GOTCHA: celery gevent pool breaks on threadlocal storage
+def run(): pass
"#;
    let found_py = scan_diff_for_directives(py_diff);
    assert!(found_py.is_some());
    let (entity, anchor) = found_py.unwrap();
    assert_eq!(entity.kind, EntityKind::Gotcha);
    assert_eq!(anchor.as_deref(), Some("backend/tasks/worker.py:50"));
}

#[test]
fn test_post_commit_fallback_on_poor_commit_message() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_test_diff_fallback_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let root = &temp_dir;

    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Test User"]);
    run_git(root, &["config", "user.email", "test@example.com"]);
    init_project(root).unwrap();

    let code_file = root.join("src").join("server.rs");
    fs::create_dir_all(code_file.parent().unwrap()).unwrap();
    fs::write(
        &code_file,
        "// GOTCHA: macOS AirPlay Receiver occupies port 5000 by default\npub fn run() {}\n",
    )
    .unwrap();

    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "wip"]);

    let report = run_post_commit(root, false)
        .expect("hook execution succeeds")
        .expect("directive extracted from diff fallback");

    let entity = report.entity_captured.expect("entity was captured");
    assert_eq!(entity.kind, "gotcha");
    assert!(entity.key.contains("macos-airplay"));
    assert!(
        entity
            .anchor
            .as_deref()
            .unwrap()
            .starts_with("src/server.rs:")
    );

    let db_path = root.join(".agent-mem").join("mem.db");
    let store = Store::open(&db_path, true).unwrap();
    let record = store
        .get_entry(&entity.key)
        .unwrap()
        .expect("rule in store");
    assert_eq!(
        record.val,
        "macOS AirPlay Receiver occupies port 5000 by default"
    );
    drop(store);
    let _ = fs::remove_dir_all(&temp_dir);
}
