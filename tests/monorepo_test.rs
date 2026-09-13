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
    };
    execute_command(root_set, &found_from_web).unwrap();

    // Set rule in Isolated Subproject
    let sub_set = Command::Set {
        key: "service/framework".into(),
        val: "Actix Web 4.0".into(),
        anchor: Some("src/main.rs:5".into()),
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
