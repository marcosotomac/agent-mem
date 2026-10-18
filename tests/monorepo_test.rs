use agent_mem::cli::{Command, execute_command};
use agent_mem::init::{find_project_root_from, init_project};
use std::fs;

#[test]
fn test_monorepo_deep_nesting_and_subproject_isolation() {
    let base_dir = std::env::temp_dir().join(format!(
        "agent_mem_monorepo_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    // 1. Setup Monorepo layout
    // /base_dir (Root with .git)
    //   ├── packages/core/src/
    //   ├── apps/web/src/pages/dashboard/
    //   └── subprojects/isolated_service/ (.agent-mem)
    //         └── src/controllers/

    let root_git = base_dir.join(".git");
    fs::create_dir_all(&root_git).unwrap();

    let deep_web_dir = base_dir
        .join("apps")
        .join("web")
        .join("src")
        .join("pages")
        .join("dashboard");
    fs::create_dir_all(&deep_web_dir).unwrap();

    let deep_core_dir = base_dir.join("packages").join("core").join("src");
    fs::create_dir_all(&deep_core_dir).unwrap();

    let isolated_subproject = base_dir.join("subprojects").join("isolated_service");
    let isolated_deep_dir = isolated_subproject.join("src").join("controllers");
    fs::create_dir_all(&isolated_deep_dir).unwrap();

    // Initialize Root Monorepo
    let root_report = init_project(&base_dir).unwrap();
    assert_eq!(root_report.root, base_dir);
    assert!(base_dir.join(".agent-mem").join("mem.db").exists());
    assert!(base_dir.join(".agent-rules").exists());

    // Initialize Isolated Subproject
    let sub_report = init_project(&isolated_subproject).unwrap();
    assert_eq!(sub_report.root, isolated_subproject);
    assert!(
        isolated_subproject
            .join(".agent-mem")
            .join("mem.db")
            .exists()
    );
    assert!(isolated_subproject.join(".agent-rules").exists());

    // 2. Test Deep Traversal from apps/web/... must resolve to Monorepo Root
    let found_from_web = find_project_root_from(&deep_web_dir);
    assert_eq!(found_from_web, base_dir);

    let found_from_core = find_project_root_from(&deep_core_dir);
    assert_eq!(found_from_core, base_dir);

    // 3. Test Deep Traversal from subprojects/isolated_service/src/controllers/ must resolve to Isolated Subproject
    let found_from_isolated = find_project_root_from(&isolated_deep_dir);
    assert_eq!(found_from_isolated, isolated_subproject);

    // 4. Test Data Isolation
    // Set rule in Monorepo root
    let root_set = Command::Set {
        key: "monorepo/toolchain".into(),
        val: "Turborepo + Cargo Workspaces".into(),
        anchor: Some("Cargo.toml:1".into()),
        kind: None,
        scope: agent_mem::cli::MemoryScope::Project,
    };
    execute_command(root_set, &found_from_web).unwrap();

    // Set rule in Isolated Subproject
    let sub_set = Command::Set {
        key: "service/framework".into(),
        val: "Actix Web 4.0".into(),
        anchor: Some("src/main.rs:5".into()),
        kind: None,
        scope: agent_mem::cli::MemoryScope::Project,
    };
    execute_command(sub_set, &found_from_isolated).unwrap();

    // Verify Monorepo Root has its rule and NOT the subproject rule
    let root_store =
        agent_mem::store::Store::open(&base_dir.join(".agent-mem").join("mem.db"), false).unwrap();
    assert_eq!(
        root_store.get("monorepo/toolchain").unwrap().as_deref(),
        Some("Turborepo + Cargo Workspaces")
    );
    assert_eq!(root_store.get("service/framework").unwrap(), None);

    // Verify Isolated Subproject has its rule and NOT the monorepo root rule
    let sub_store = agent_mem::store::Store::open(
        &isolated_subproject.join(".agent-mem").join("mem.db"),
        false,
    )
    .unwrap();
    assert_eq!(
        sub_store.get("service/framework").unwrap().as_deref(),
        Some("Actix Web 4.0")
    );
    assert_eq!(sub_store.get("monorepo/toolchain").unwrap(), None);

    // Verify .agent-rules files are completely isolated
    let root_rules = fs::read_to_string(base_dir.join(".agent-rules")).unwrap();
    assert!(root_rules.contains("monorepo/toolchain"));
    assert!(!root_rules.contains("service/framework"));

    let sub_rules = fs::read_to_string(isolated_subproject.join(".agent-rules")).unwrap();
    assert!(sub_rules.contains("service/framework"));
    assert!(!sub_rules.contains("monorepo/toolchain"));

    let _ = fs::remove_dir_all(&base_dir);
}

#[test]
fn test_modular_rules_directory_export_and_sync() {
    let base_dir = std::env::temp_dir().join(format!(
        "agent_mem_modular_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base_dir).unwrap();

    let root_git = base_dir.join(".git");
    fs::create_dir_all(&root_git).unwrap();

    let report = init_project(&base_dir).unwrap();
    assert!(report.gitattributes_updated);

    let gitattributes = fs::read_to_string(base_dir.join(".gitattributes")).unwrap();
    assert!(gitattributes.contains(".agent-rules merge=union"));
    assert!(gitattributes.contains("*.rules merge=union"));

    let db_path = base_dir.join(".agent-mem").join("mem.db");
    let mut store = agent_mem::store::Store::open(&db_path, true).unwrap();

    // Insert multi-domain rules
    store
        .set_entry(
            "auth/jwt-expiry",
            "JWT tokens expire in 15 minutes",
            Some("src/auth.rs:10"),
            Some("decision"),
        )
        .unwrap();
    store
        .set_entry(
            "auth/refresh-token",
            "Refresh tokens must be single-use rotated",
            Some("src/auth.rs:25"),
            Some("gotcha"),
        )
        .unwrap();
    store
        .set_entry(
            "billing/stripe-webhook",
            "Verify HMAC signature and check idempotency table",
            Some("src/billing.rs:5"),
            Some("decision"),
        )
        .unwrap();
    store
        .set_entry(
            "architecture/clean-boundaries",
            "Domain entities must have zero external dependencies",
            Some("src/domain.rs:1"),
            Some("rule"),
        )
        .unwrap();
    store
        .set_entry(
            "code-review-policy",
            "All PRs require minimum 2 approvals and passing CI",
            None,
            Some("rule"),
        )
        .unwrap();

    store
        .relate("auth/jwt-expiry", "mitigates", "auth/refresh-token")
        .unwrap();

    // Export to modular directory .agent-rules.d
    let rules_d = base_dir.join(".agent-rules.d");
    store.export_to_dir(&rules_d).unwrap();

    assert!(rules_d.is_dir());
    assert!(rules_d.join("auth.rules").exists());
    assert!(rules_d.join("billing.rules").exists());
    assert!(rules_d.join("architecture.rules").exists());
    assert!(rules_d.join("default.rules").exists());
    assert!(rules_d.join("relations.rules").exists());

    let auth_content = fs::read_to_string(rules_d.join("auth.rules")).unwrap();
    assert!(auth_content.contains("auth/jwt-expiry"));
    assert!(auth_content.contains("auth/refresh-token"));
    assert!(!auth_content.contains("billing/stripe-webhook"));

    let billing_content = fs::read_to_string(rules_d.join("billing.rules")).unwrap();
    assert!(billing_content.contains("billing/stripe-webhook"));
    assert!(!billing_content.contains("auth/jwt-expiry"));

    let default_content = fs::read_to_string(rules_d.join("default.rules")).unwrap();
    assert!(default_content.contains("code-review-policy"));

    let rel_content = fs::read_to_string(rules_d.join("relations.rules")).unwrap();
    assert!(rel_content.contains("[rel] auth/jwt-expiry -> mitigates -> auth/refresh-token"));

    // Verify resolve_rules_path automatically picks up .agent-rules.d
    assert_eq!(agent_mem::store::resolve_rules_path(&base_dir), rules_d);

    // Test editing a modular file directly and syncing back into SQLite
    let updated_auth = auth_content.replace("15 minutes", "30 minutes");
    fs::write(rules_d.join("auth.rules"), updated_auth).unwrap();

    let sync_rep = store.sync_with_dir(&rules_d, false).unwrap();
    assert_eq!(sync_rep.total, 5);

    let updated_record = store.get_entry("auth/jwt-expiry").unwrap().unwrap();
    assert_eq!(updated_record.val, "JWT tokens expire in 30 minutes");

    // Test fast-path no-op on directory sync
    let noop_rep = store.sync_with_dir(&rules_d, false).unwrap();
    assert_eq!(noop_rep.total, 5);

    drop(store);
    let _ = fs::remove_dir_all(&base_dir);
}
