use agent_mem::cli::Command as CliCommand;
use agent_mem::mcp::McpServer;
use agent_mem::store::{BatchRule, Store};
use serde_json::json;
use std::fs;
mod support;
use support::{TempDir, call};

#[test]
fn export_lock_failure_rejects_cli_and_mcp_writes_before_mutating_sqlite() {
    let dir = TempDir::new();
    let rules = dir.0.join(".agent-rules");
    let mut store = Store::open(&dir.db(), true).unwrap();
    store.set("existing/rule", "Keep this").unwrap();
    store.export_to_file(&rules).unwrap();
    let lock = dir.0.join(".agent-mem/agent-rules.lock");
    fs::remove_file(&lock).unwrap();
    fs::create_dir(&lock).unwrap();
    let server = McpServer::with_paths(dir.0.clone(), dir.0.join("global.db"));
    for arguments in [
        json!({"key":"new/rule", "val":"New advice"}),
        json!({"batch":[{"key":"new/rule", "val":"New advice"}]}),
    ] {
        assert_eq!(call(&server, "mem_set", arguments)["isError"], true);
        assert_eq!(store.get("new/rule").unwrap(), None);
    }
    assert_eq!(
        call(
            &server,
            "mem_manage",
            json!({"action":"delete", "key":"existing/rule"})
        )["isError"],
        true
    );
    assert!(store.get("existing/rule").unwrap().is_some());
    assert!(
        agent_mem::cli::execute_command(
            CliCommand::Set {
                key: "cli/rule".into(),
                val: "New advice".into(),
                anchor: None,
                kind: None,
                scope: agent_mem::cli::MemoryScope::Project,
            },
            &dir.0
        )
        .is_err()
    );
    assert_eq!(store.get("cli/rule").unwrap(), None);
    fs::remove_dir(&lock).unwrap();
    store.sync_with_file(&rules).unwrap();
    assert!(store.get("existing/rule").unwrap().is_some());
}

#[test]
fn partial_export_is_durably_quarantined_until_explicit_recovery() {
    let dir = TempDir::new();
    let rules = dir.0.join(".agent-rules.d");
    let mut store = Store::open(&dir.db(), true).unwrap();
    store.set("a/existing", "Original advice").unwrap();
    store.export_to_dir(&rules).unwrap();
    drop(store);
    // The first shard can be replaced, but renaming the second shard must fail.
    fs::create_dir(rules.join("zblocked.rules")).unwrap();
    let server = McpServer::with_paths(dir.0.clone(), dir.0.join("global.db"));
    let result = call(
        &server,
        "mem_set",
        json!({"batch":[
            {"key":"a/existing", "val":"Updated advice"},
            {"key":"zblocked/new", "val":"Durable local advice"},
        ]}),
    );
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("saved locally")
    );
    drop(server);
    let mut store = Store::open(&dir.db(), true).unwrap();
    let before = store.export_rules_text().unwrap();
    assert!(
        store
            .sync_with_dir(&rules, false)
            .unwrap_err()
            .to_string()
            .contains("Pending export")
    );
    let alias = rules.join(".");
    assert!(store.sync_with_path(&alias, true).is_err());
    let alternate = dir.0.join("other.rules");
    fs::write(&alternate, "a/existing = Stale advice\n").unwrap();
    assert!(store.sync_with_file(&alternate).is_err());
    assert_eq!(store.export_rules_text().unwrap(), before);
    assert_eq!(
        store.get("zblocked/new").unwrap().as_deref(),
        Some("Durable local advice")
    );
    fs::remove_dir(rules.join("zblocked.rules")).unwrap();
    store.sync_export(&rules).unwrap();
    store.sync_with_dir(&rules, false).unwrap();
    assert_eq!(store.export_rules_text().unwrap(), before);
    let mut replica = Store::open_in_memory().unwrap();
    replica.sync_with_dir(&rules, false).unwrap();
    assert_eq!(replica.export_rules_text().unwrap(), before);
}

#[test]
fn concurrent_mutations_and_imports_share_the_export_lock() {
    let dir = TempDir::new();
    let path = dir.0.join(".agent-rules");
    let mut store = Store::open(&dir.db(), true).unwrap();
    store.set("existing/rule", "Original advice").unwrap();
    store.export_to_file(&path).unwrap();
    drop(store);
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for writer in 0..3 {
            let barrier = &barrier;
            let path = &path;
            let db = dir.db();
            scope.spawn(move || {
                let mut store = Store::open(&db, true).unwrap();
                barrier.wait();
                for index in 0..8 {
                    let key = format!("writer-{writer}/rule-{index}");
                    store
                        .with_rules_export(Some(path), |store| store.set(&key, "Concurrent advice"))
                        .unwrap();
                }
            });
        }
        let barrier = &barrier;
        let path = &path;
        let db = dir.db();
        scope.spawn(move || {
            let mut store = Store::open(&db, true).unwrap();
            barrier.wait();
            for _ in 0..24 {
                store.sync_with_file(path).unwrap();
            }
        });
    });
    let mut replica = Store::open_in_memory().unwrap();
    replica.sync_with_file(&path).unwrap();
    let store = Store::open(&dir.db(), false).unwrap();
    assert_eq!(store.dump().unwrap().len(), 25);
    assert_eq!(
        replica.export_rules_text().unwrap(),
        store.export_rules_text().unwrap()
    );
}

#[test]
fn failed_atomic_mutation_does_not_leave_a_spurious_pending_export() {
    let dir = TempDir::new();
    let path = dir.0.join(".agent-rules");
    let mut store = Store::open(&dir.db(), true).unwrap();
    store.set("existing/rule", "Keep this advice").unwrap();
    store.export_to_file(&path).unwrap();
    assert!(
        store
            .with_rules_export(Some(&path), |store| store.set_batch(&[BatchRule {
                key: "new/rule",
                val: "New advice",
                anchor: None,
                kind: None,
                relation: Some(("relates_to", "missing/rule")),
            }]))
            .is_err()
    );
    assert_eq!(store.get("new/rule").unwrap(), None);
    store.sync_with_file(&path).unwrap();
}
