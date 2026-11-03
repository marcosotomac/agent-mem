use agent_mem::mcp::McpServer;
use agent_mem::store::{Store, TRUST_LOCAL, TRUST_REVIEWED};
use serde_json::json;
mod support;
use support::{TempDir, call};

#[test]
fn review_is_bound_to_content_but_survives_identical_writes() {
    for field in ["value", "anchor", "kind", "lifecycle", "hook"] {
        let mut store = Store::open_in_memory().unwrap();
        store
            .set_entry(
                "safety/check",
                "Reviewed advice",
                Some("src/auth.rs:1"),
                Some("gotcha"),
            )
            .unwrap();
        store.set_trust("safety/check", TRUST_REVIEWED).unwrap();
        let reviewed = store.metadata("safety/check").unwrap().unwrap().reviewed_at;
        assert!(reviewed.is_some());
        store
            .set_entry(
                "safety/check",
                "Reviewed advice",
                Some("src/auth.rs:1"),
                Some("gotcha"),
            )
            .unwrap();
        assert_eq!(
            store.metadata("safety/check").unwrap().unwrap().reviewed_at,
            reviewed
        );
        assert_eq!(
            store.metadata("safety/check").unwrap().unwrap().trust,
            TRUST_REVIEWED
        );
        match field {
            "value" => store
                .set_entry(
                    "safety/check",
                    "Different advice",
                    Some("src/auth.rs:1"),
                    Some("gotcha"),
                )
                .unwrap(),
            "anchor" => store
                .set_entry(
                    "safety/check",
                    "Reviewed advice",
                    Some("src/other.rs:2"),
                    Some("gotcha"),
                )
                .unwrap(),
            "kind" => store
                .set_entry(
                    "safety/check",
                    "Reviewed advice",
                    Some("src/auth.rs:1"),
                    Some("decision"),
                )
                .unwrap(),
            "lifecycle" => {
                store.archive("safety/check", Some("Retired")).unwrap();
                store
                    .set_entry(
                        "safety/check",
                        "Reviewed advice",
                        Some("src/auth.rs:1"),
                        Some("gotcha"),
                    )
                    .unwrap();
            }
            "hook" => {
                store
                    .capture_commit(
                        Some((
                            "safety/check",
                            "Reviewed advice",
                            Some("src/auth.rs:1"),
                            Some("gotcha"),
                        )),
                        &[],
                        "",
                    )
                    .unwrap();
                assert_eq!(
                    store.metadata("safety/check").unwrap().unwrap().trust,
                    TRUST_REVIEWED
                );
                store
                    .capture_commit(
                        Some((
                            "safety/check",
                            "Replaced by hook",
                            Some("src/auth.rs:1"),
                            Some("gotcha"),
                        )),
                        &[],
                        "",
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let metadata = store.metadata("safety/check").unwrap().unwrap();
        assert_eq!(metadata.trust, TRUST_LOCAL, "{field}");
        assert_eq!(metadata.reviewed_at, None, "{field}");
        assert_eq!(
            store.history("safety/check").unwrap()[1].trust.as_deref(),
            Some(TRUST_REVIEWED)
        );
    }
}

#[test]
fn mcp_single_and_batch_replacements_cannot_keep_reviewed_status() {
    for batch in [false, true] {
        let dir = TempDir::new();
        let mut store = Store::open(&dir.db(), true).unwrap();
        store.set("safety/check", "Reviewed advice").unwrap();
        store.set_trust("safety/check", TRUST_REVIEWED).unwrap();
        store.export_to_file(&dir.0.join(".agent-rules")).unwrap();
        drop(store);
        let server = McpServer::with_paths(dir.0.clone(), dir.0.join("global.db"));
        let item = json!({"key":"safety/check", "val":"Completely different advice"});
        let arguments = if batch { json!({"batch":[item]}) } else { item };
        assert_ne!(call(&server, "mem_set", arguments)["isError"], true);
        let store = Store::open(&dir.db(), false).unwrap();
        let metadata = store.metadata("safety/check").unwrap().unwrap();
        assert_eq!(metadata.trust, TRUST_LOCAL);
        assert_eq!(metadata.reviewed_at, None);
    }
}
