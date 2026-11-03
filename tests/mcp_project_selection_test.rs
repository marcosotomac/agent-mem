use agent_mem::mcp::McpServer;
use agent_mem::registry::{ProjectRecord, ProjectRegistry};
use agent_mem::store::Store;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
mod support;
use support::{TempDir, call};

#[test]
fn ambiguous_name_cannot_bypass_selection_via_the_default_project() {
    for register_default in [false, true] {
        let dir = TempDir::new();
        let current = dir.0.join("group-a/same");
        let other = dir.0.join("group-b/same");
        let registry_dir = dir.0.join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        for root in [&current, &other] {
            Store::open(&root.join(".agent-mem/mem.db"), true).unwrap();
        }
        let record = |id: &str, path: &std::path::Path| ProjectRecord {
            id: id.into(),
            name: "same".into(),
            canonical_path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
            git_remote: None,
            last_accessed: 0,
            rules_count: 0,
            archived_count: 0,
            sessions_count: 0,
            db_size_bytes: 0,
        };
        let mut projects = vec![record("other-id", &other)];
        if register_default {
            projects.push(record("current-id", &current));
        }
        fs::write(
            registry_dir.join("projects.json"),
            serde_json::to_vec(&ProjectRegistry { projects }).unwrap(),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-mem"))
            .arg("mcp")
            .current_dir(&current)
            .env("AGENT_MEM_GLOBAL_DIR", &registry_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let requests = [
            json!({"key":"ambiguous/check","val":"Must not be written","project":"same"}),
            json!({"key":"default/check","val":"Current project"}),
            json!({"key":"path/check","val":"Current path","project":current.canonicalize().unwrap()}),
            json!({"key":"id/check","val":"Other project","project":"other-id"}),
            json!({"key":"path/check","val":"Other path","project":other.canonicalize().unwrap()}),
        ];
        for (id, args) in requests.iter().enumerate() {
            writeln!(
                child.stdin.as_mut().unwrap(),
                "{}",
                json!({
                    "jsonrpc":"2.0","id":id,"method":"tools/call",
                    "params":{"name":"mem_set","arguments":args},
                })
            )
            .unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let responses: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), requests.len());
        assert_eq!(responses[0]["result"]["isError"], true);
        assert!(
            responses[0]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Ambiguous project")
        );
        for response in &responses[1..] {
            assert_ne!(response["result"]["isError"], true, "{response}");
        }
        let current_store = Store::open(&current.join(".agent-mem/mem.db"), false).unwrap();
        let other_store = Store::open(&other.join(".agent-mem/mem.db"), false).unwrap();
        for store in [&current_store, &other_store] {
            assert_eq!(store.get("ambiguous/check").unwrap(), None);
        }
        assert_eq!(
            current_store.get("default/check").unwrap().as_deref(),
            Some("Current project")
        );
        assert_eq!(
            current_store.get("path/check").unwrap().as_deref(),
            Some("Current path")
        );
        assert_eq!(
            other_store.get("id/check").unwrap().as_deref(),
            Some("Other project")
        );
        assert_eq!(
            other_store.get("path/check").unwrap().as_deref(),
            Some("Other path")
        );
    }
}

#[test]
fn unique_default_name_still_resolves_without_a_registry_entry() {
    let dir = TempDir::new();
    let server = McpServer::with_paths(dir.0.clone(), dir.0.join("global.db"));
    let response = call(
        &server,
        "mem_set",
        json!({
            "key":"selection/check","val":"Unique default project",
            "project":dir.0.file_name().unwrap().to_string_lossy(),
        }),
    );
    assert_ne!(response["isError"], true);
    assert!(
        Store::open(&dir.db(), false)
            .unwrap()
            .get("selection/check")
            .unwrap()
            .is_some()
    );
}
