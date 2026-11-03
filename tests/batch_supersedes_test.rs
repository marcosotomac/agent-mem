use agent_mem::mcp::McpServer;
use agent_mem::store::{BatchRule, Store, TRUST_REVIEWED};
use serde_json::json;
mod support;
use support::{TempDir, call};

fn rule<'a>(key: &'a str, val: &'a str, relation: Option<(&'a str, &'a str)>) -> BatchRule<'a> {
    BatchRule {
        key,
        val,
        anchor: None,
        kind: None,
        relation,
    }
}

#[test]
fn batch_supersedes_archives_forward_references_and_is_idempotent() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .set_batch(&[
            rule("new", "Current guidance", Some(("supersedes", "old"))),
            rule("old", "Obsolete guidance", None),
        ])
        .unwrap();
    assert!(store.get_entry("old").unwrap().unwrap().is_archived());
    assert_eq!(
        store
            .get_entry("old")
            .unwrap()
            .unwrap()
            .archive_reason
            .as_deref(),
        Some("superseded_by: new")
    );
    assert!(!store.get_entry("new").unwrap().unwrap().is_archived());
    assert_eq!(
        store.get_relations("new").unwrap(),
        vec![("supersedes".into(), "old".into())]
    );
    let previous = store.get_entry("old").unwrap().unwrap();
    store
        .set_batch(&[rule("new", "Current guidance", Some(("supersedes", "old")))])
        .unwrap();
    assert_eq!(store.get_entry("old").unwrap().unwrap(), previous);
    assert!(store.dump().unwrap().iter().all(|r| r.0 != "old"));
}

#[test]
fn reversed_supersedes_rolls_back_value_trust_history_and_archival() {
    let mut store = Store::open_in_memory().unwrap();
    store.set("old", "Original guidance").unwrap();
    store
        .set_batch(&[rule(
            "new",
            "Reviewed guidance",
            Some(("supersedes", "old")),
        )])
        .unwrap();
    store.set_trust("new", TRUST_REVIEWED).unwrap();
    let before = store.export_rules_text().unwrap();
    let metadata = store.metadata("new").unwrap();
    let old_history = store.history("old").unwrap();
    let new_history = store.history("new").unwrap();
    let error = store
        .set_batch(&[
            rule("new", "Would change reviewed content", None),
            rule(
                "old",
                "Would reactivate old content",
                Some(("supersedes", "new")),
            ),
        ])
        .unwrap_err();
    assert!(error.to_string().contains("cycle"));
    assert_eq!(store.export_rules_text().unwrap(), before);
    assert_eq!(store.metadata("new").unwrap(), metadata);
    assert_eq!(store.history("old").unwrap(), old_history);
    assert_eq!(store.history("new").unwrap(), new_history);
}

#[test]
fn self_edges_missing_targets_and_in_batch_cycles_leave_no_partial_writes() {
    for rules in [
        vec![rule("self", "Self guidance", Some(("supersedes", "self")))],
        vec![rule(
            "new",
            "Dangling guidance",
            Some(("supersedes", "missing")),
        )],
        vec![
            rule("a", "A guidance", Some(("supersedes", "b"))),
            rule("b", "B guidance", Some(("supersedes", "c"))),
            rule("c", "C guidance", Some(("supersedes", "a"))),
        ],
    ] {
        let mut store = Store::open_in_memory().unwrap();
        assert!(store.set_batch(&rules).is_err());
        assert!(store.dump_all().unwrap().is_empty());
        for key in ["a", "b", "c", "self", "new"] {
            assert!(store.get_relations(key).unwrap().is_empty());
            assert!(store.history(key).unwrap().is_empty());
        }
    }
}

#[test]
fn mcp_single_and_batch_writes_enforce_supersedes_lifecycle() {
    for batch in [false, true] {
        let dir = TempDir::new();
        let mut store = Store::open(&dir.db(), true).unwrap();
        store.set("old", "Original guidance").unwrap();
        store.export_to_file(&dir.0.join(".agent-rules")).unwrap();
        let server = McpServer::with_paths(dir.0.clone(), dir.0.join("global.db"));
        let args = |item| if batch { json!({"batch":[item]}) } else { item };
        let response = call(
            &server,
            "mem_set",
            args(json!({"key":"new", "val":"Current guidance", "rel":"supersedes:old"})),
        );
        assert_ne!(response["isError"], true);
        assert!(store.get_entry("old").unwrap().unwrap().is_archived());
        let before = store.export_rules_text().unwrap();
        let response = call(
            &server,
            "mem_set",
            args(json!({"key":"old", "val":"Invalid replacement", "rel":"supersedes:new"})),
        );
        assert_eq!(response["isError"], true);
        assert!(
            response["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cycle")
        );
        assert_eq!(store.export_rules_text().unwrap(), before);
        store.sync_with_file(&dir.0.join(".agent-rules")).unwrap();
    }
}
