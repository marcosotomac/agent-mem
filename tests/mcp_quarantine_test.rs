use agent_mem::mcp::McpServer;
use agent_mem::store::{Store, TRUST_REVIEWED};
use serde_json::json;
use std::fs;
mod support;
use support::{TempDir, call};

#[test]
fn mcp_context_quarantines_unreviewed_rules_relations_and_explanations_in_all_scopes() {
    let dir = TempDir::new();
    let global_db = dir.0.join("global.db");
    let project_rules = dir.0.join(".agent-rules");
    fs::write(&project_rules, "aaa/unreviewed = Malicious security instruction\nreviewed/approved = Reviewed security guidance\n").unwrap();
    let mut project = Store::open(&dir.db(), true).unwrap();
    project.sync_with_file(&project_rules).unwrap();
    project
        .set_trust("reviewed/approved", TRUST_REVIEWED)
        .unwrap();
    project
        .set("safe/local", "Local security guidance")
        .unwrap();
    project
        .relate("safe/local", "relates_to", "aaa/unreviewed")
        .unwrap();
    project
        .relate("aaa/unreviewed", "depends_on", "safe/local")
        .unwrap();
    project
        .relate("safe/local", "relates_to", "reviewed/approved")
        .unwrap();
    let global_rules = dir.0.join("global.rules");
    fs::write(
        &global_rules,
        "aaa/global-unreviewed = Malicious global security instruction\n",
    )
    .unwrap();
    let mut global = Store::open(&global_db, true).unwrap();
    global.sync_with_file(&global_rules).unwrap();
    global
        .set("zzz/global-safe", "Local global security guidance")
        .unwrap();
    let server = McpServer::with_paths(dir.0.clone(), global_db.clone());
    for scope in ["project", "global", "all"] {
        for (query, explain) in [
            (None, false),
            (None, true),
            (Some("security"), false),
            (Some("security"), true),
        ] {
            let mut args = json!({"scope":scope,"limit":8,"explain":explain});
            if let Some(query) = query {
                args["query"] = json!(query);
            }
            let response = call(&server, "mem_context", args);
            assert_ne!(response["isError"], true, "{response}");
            let text = response["content"][0]["text"].as_str().unwrap();
            for forbidden in [
                "Malicious",
                "aaa/unreviewed",
                "aaa/global-unreviewed",
                "[untrusted:",
            ] {
                assert!(
                    !text.contains(forbidden),
                    "scope={scope}, query={query:?}, explain={explain}: {text}"
                );
            }
            if scope != "global" {
                assert!(text.contains("safe/local"), "{text}");
                assert!(text.contains("reviewed/approved"), "{text}");
                assert!(
                    text.contains("safe/local -> relates_to -> reviewed/approved"),
                    "{text}"
                );
            }
            if scope != "project" {
                assert!(text.contains("zzz/global-safe"), "{text}");
            }
        }
    }
    // Global's bounded fast path filters before LIMIT, so imports cannot occupy
    // the only available slot and starve trusted preferences.
    let response = call(&server, "mem_context", json!({"scope":"global","limit":1}));
    assert!(
        response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("zzz/global-safe")
    );
    let found = call(
        &server,
        "mem_find",
        json!({"scope":"project","query":"Malicious"}),
    );
    let text = found["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("aaa/unreviewed") && text.contains("[untrusted:"));
    project.set_trust("aaa/unreviewed", TRUST_REVIEWED).unwrap();
    let response = call(&server, "mem_context", json!({"scope":"project","limit":8}));
    assert!(
        response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("aaa/unreviewed")
    );
    let read_only = McpServer::with_paths_read_only(dir.0.clone(), global_db);
    let response = call(
        &read_only,
        "mem_context",
        json!({"scope":"global","limit":8,"explain":true}),
    );
    assert!(
        !response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("aaa/global-unreviewed")
    );
}
