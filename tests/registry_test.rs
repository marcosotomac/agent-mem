use agent_mem::registry::ProjectRegistry;
use agent_mem::store::Store;
use std::fs;

static REGISTRY_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_registry_registration_and_anti_collision() {
    let _lock = REGISTRY_TEST_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let global_dir = std::env::temp_dir().join(format!(
        "agent_mem_reg_test_global_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&global_dir).unwrap();
    unsafe {
        std::env::set_var("AGENT_MEM_GLOBAL_DIR", &global_dir);
    }

    let repo_dir = std::env::temp_dir().join(format!(
        "agent_mem_reg_repo_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&repo_dir).unwrap();

    // Create fake .git/config with origin remote
    let git_dir = repo_dir.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    let git_config = r#"[core]
	repositoryformatversion = 0
	filemode = true
	bare = false
	logallrefupdates = true
	ignorecase = true
	precomposeunicode = true
[remote "origin"]
	url = git@github.com:test-user/my-test-project.git
	fetch = +refs/heads/*:refs/remotes/origin/*
[branch "main"]
	remote = origin
	merge = refs/heads/main
"#;
    fs::write(git_dir.join("config"), git_config).unwrap();

    // Initialize .agent-mem/mem.db
    let db_path = repo_dir.join(".agent-mem").join("mem.db");
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set("frontend/react", "Use Next.js App Router")
            .unwrap();
        store.session_add("Initial session").unwrap();
    }

    // 1. Verify git remote extraction
    let remote = ProjectRegistry::extract_git_remote(&repo_dir);
    assert_eq!(
        remote.as_deref(),
        Some("git@github.com:test-user/my-test-project.git")
    );

    // 2. Register project
    let mut reg = ProjectRegistry::load().unwrap();
    let record1 = reg.register(&repo_dir).unwrap();
    assert_eq!(record1.rules_count, 1);
    assert_eq!(record1.sessions_count, 1);
    assert_eq!(
        record1.git_remote.as_deref(),
        Some("git@github.com:test-user/my-test-project.git")
    );
    assert_eq!(reg.projects.len(), 1);

    // 3. Register AGAIN (idempotency check)
    let record2 = reg.register(&repo_dir).unwrap();
    assert_eq!(reg.projects.len(), 1, "Must not duplicate existing project");
    assert_eq!(record1.id, record2.id);

    // 4. Test symlink deduplication (accessing project via symlink must resolve to same project)
    let symlink_dir = std::env::temp_dir().join(format!(
        "agent_mem_symlink_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&repo_dir, &symlink_dir).unwrap();
        let record3 = reg.register(&symlink_dir).unwrap();
        assert_eq!(
            reg.projects.len(),
            1,
            "Symlink to existing project must be deduplicated via canonical path"
        );
        assert_eq!(record1.id, record3.id);
        let _ = fs::remove_file(&symlink_dir);
    }

    // 5. Test moved/renamed directory deduplication via Git Remote
    // Simulate moving repo_dir to moved_dir
    let moved_dir = std::env::temp_dir().join(format!(
        "agent_mem_moved_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::rename(&repo_dir, &moved_dir).unwrap();

    // Now repo_dir does not exist, but moved_dir has the same git remote!
    let record4 = reg.register(&moved_dir).unwrap();
    assert_eq!(
        reg.projects.len(),
        1,
        "Moved repo with same git remote must replace the dead path instead of duplicating"
    );
    assert_eq!(record4.id, record1.id);
    assert_eq!(
        record4.canonical_path,
        moved_dir
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );

    // 6. Test prune
    // Remove moved_dir from disk
    fs::remove_dir_all(&moved_dir).unwrap();
    let pruned = reg.prune().unwrap();
    assert_eq!(pruned, 1, "Prune should remove deleted project");
    assert_eq!(reg.projects.len(), 0);

    unsafe {
        std::env::remove_var("AGENT_MEM_GLOBAL_DIR");
    }
    let _ = fs::remove_dir_all(&global_dir);
}

#[test]
fn test_registry_atomic_save_and_persistence() {
    let _lock = REGISTRY_TEST_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let global_dir = std::env::temp_dir().join(format!(
        "agent_mem_reg_atomic_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&global_dir).unwrap();
    unsafe {
        std::env::set_var("AGENT_MEM_GLOBAL_DIR", &global_dir);
    }

    let p1 = std::env::temp_dir().join("test_p1");
    let p2 = std::env::temp_dir().join("test_p2");
    fs::create_dir_all(&p1).unwrap();
    fs::create_dir_all(&p2).unwrap();

    let mut reg = ProjectRegistry::load().unwrap();
    reg.register(&p1).unwrap();
    reg.register(&p2).unwrap();
    assert_eq!(reg.projects.len(), 2);

    // Reload from disk to verify atomic save
    let reloaded = ProjectRegistry::load().unwrap();
    assert_eq!(reloaded.projects.len(), 2);

    // Deregister by id
    let target_id = reloaded.projects[0].id.clone();
    let mut reg_mut = reloaded;
    assert!(reg_mut.deregister(&target_id).unwrap());
    assert_eq!(reg_mut.projects.len(), 1);

    // Reload again
    let final_reg = ProjectRegistry::load().unwrap();
    assert_eq!(final_reg.projects.len(), 1);

    let _ = fs::remove_dir_all(&p1);
    let _ = fs::remove_dir_all(&p2);
    unsafe {
        std::env::remove_var("AGENT_MEM_GLOBAL_DIR");
    }
    let _ = fs::remove_dir_all(&global_dir);
}

#[test]
fn test_registry_ignores_temp_paths_when_global_dir_unset() {
    let _lock = REGISTRY_TEST_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    unsafe {
        std::env::remove_var("AGENT_MEM_GLOBAL_DIR");
    }

    let temp_repo = std::env::temp_dir().join(format!(
        "agent_mem_reg_unregistered_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_repo).unwrap();

    let mut reg = ProjectRegistry::load().unwrap();
    let initial_count = reg.projects.len();

    let record = reg.register(&temp_repo).unwrap();
    assert_eq!(record.name, temp_repo.file_name().unwrap().to_str().unwrap());

    // Should not have incremented or saved
    assert_eq!(reg.projects.len(), initial_count);

    let reloaded = ProjectRegistry::load().unwrap();
    assert_eq!(reloaded.projects.len(), initial_count);

    let _ = fs::remove_dir_all(&temp_repo);
}

