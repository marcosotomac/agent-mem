use std::fs;
use std::process::Command;

fn run_git(dir: &std::path::Path, args: &[&str]) -> String {
    let bin_dir = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("debug");
    let path_var = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("git")
        .current_dir(dir)
        .env("PATH", path_var)
        .args(args)
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
fn test_git_multi_branch_workflow_and_zero_merge_conflicts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_git_e2e_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Initialize real Git repo
    run_git(&temp_dir, &["init", "-b", "main"]);
    run_git(&temp_dir, &["config", "user.name", "AgentMemTester"]);
    run_git(&temp_dir, &["config", "user.email", "test@agent-mem.dev"]);

    // 2. Run agent-mem init
    let report = agent_mem::init::init_project(&temp_dir).unwrap();
    assert!(report.rules_file_created);
    assert!(report.hook_configured);
    assert!(report.post_merge_configured);

    // Initial commit
    run_git(&temp_dir, &["add", "."]);
    run_git(&temp_dir, &["commit", "-m", "chore: initial commit"]);

    // Verify post-commit hook recorded session
    let store_path = temp_dir.join(".agent-mem").join("mem.db");
    let store = agent_mem::store::Store::open(&store_path, false).unwrap();
    let sessions = store.session_list(5).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].1.contains("initial commit"));

    // 3. Create branch feature-auth
    run_git(&temp_dir, &["checkout", "-b", "feature-auth"]);

    // Set rule on feature-auth
    let set_auth = agent_mem::cli::Command::Set {
        key: "auth/provider".into(),
        val: "Clerk Authentication".into(),
        anchor: Some("src/auth.rs:10".into()),
    };
    agent_mem::cli::execute_command(set_auth, &temp_dir).unwrap();

    // Verify .agent-rules was updated
    let rules_content = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(rules_content.contains("auth/provider = Clerk Authentication (@ src/auth.rs:10)"));

    run_git(&temp_dir, &["add", "."]);
    run_git(&temp_dir, &["commit", "-m", "feat: add auth rule"]);

    // 4. Switch back to main -> test branch isolation
    run_git(&temp_dir, &["checkout", "main"]);

    // Sync main
    let sync_main = agent_mem::cli::Command::Sync {
        file: None,
        export: false,
    };
    agent_mem::cli::execute_command(sync_main, &temp_dir).unwrap();

    // In main, auth/provider MUST NOT exist!
    let get_auth_in_main = agent_mem::cli::Command::Get {
        key: "auth/provider".into(),
    };
    let get_res = agent_mem::cli::execute_command(get_auth_in_main, &temp_dir);
    assert!(matches!(get_res, Err(agent_mem::error::Error::NotFound(_))));

    // 5. Create branch feature-billing from main
    run_git(&temp_dir, &["checkout", "-b", "feature-billing"]);
    let set_billing = agent_mem::cli::Command::Set {
        key: "billing/gateway".into(),
        val: "Stripe Subscriptions API v3".into(),
        anchor: Some("src/billing.rs:25".into()),
    };
    agent_mem::cli::execute_command(set_billing, &temp_dir).unwrap();

    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &["commit", "-m", "feat: add stripe billing rule"],
    );

    // 6. Merge feature-auth into main
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(
        &temp_dir,
        &[
            "merge",
            "--no-ff",
            "feature-auth",
            "-m",
            "merge: integrate feature-auth",
        ],
    );

    // Verify auth/provider is now in main
    let store_main = agent_mem::store::Store::open(&store_path, false).unwrap();
    assert_eq!(
        store_main.get("auth/provider").unwrap().as_deref(),
        Some("Clerk Authentication")
    );

    // 7. Merge feature-billing into main (git merges .agent-rules cleanly without SQLite binary conflicts)
    run_git(
        &temp_dir,
        &[
            "merge",
            "--no-ff",
            "feature-billing",
            "-m",
            "merge: integrate feature-billing",
        ],
    );

    // Trigger sync on main
    let sync_merged = agent_mem::cli::Command::Sync {
        file: None,
        export: false,
    };
    agent_mem::cli::execute_command(sync_merged, &temp_dir).unwrap();

    let store_merged = agent_mem::store::Store::open(&store_path, false).unwrap();
    assert_eq!(
        store_merged.get("auth/provider").unwrap().as_deref(),
        Some("Clerk Authentication")
    );
    assert_eq!(
        store_merged.get("billing/gateway").unwrap().as_deref(),
        Some("Stripe Subscriptions API v3")
    );

    // 8. Test Deletion across branches: Zero Zombie Memories
    run_git(&temp_dir, &["checkout", "-b", "feature-cleanup"]);
    let del_cmd = agent_mem::cli::Command::Del {
        key: "auth/provider".into(),
    };
    agent_mem::cli::execute_command(del_cmd, &temp_dir).unwrap();

    run_git(&temp_dir, &["add", "."]);
    run_git(
        &temp_dir,
        &[
            "commit",
            "-m",
            "chore: delete deprecated auth/provider rule",
        ],
    );

    // Switch to main and merge cleanup
    run_git(&temp_dir, &["checkout", "main"]);
    run_git(
        &temp_dir,
        &[
            "merge",
            "--no-ff",
            "feature-cleanup",
            "-m",
            "merge: cleanup auth rule",
        ],
    );

    // Sync main
    let sync_cleanup = agent_mem::cli::Command::Sync {
        file: None,
        export: false,
    };
    agent_mem::cli::execute_command(sync_cleanup, &temp_dir).unwrap();

    let store_final = agent_mem::store::Store::open(&store_path, false).unwrap();
    // auth/provider MUST BE GONE (No zombie resurrection!)
    assert_eq!(store_final.get("auth/provider").unwrap(), None);
    // billing/gateway MUST STILL BE PRESENT
    assert_eq!(
        store_final.get("billing/gateway").unwrap().as_deref(),
        Some("Stripe Subscriptions API v3")
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
