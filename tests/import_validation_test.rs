use agent_mem::store::{MAX_VALUE_BYTES, Store};
use serde_json::json;
use std::fs;
mod support;
use support::TempDir;

#[test]
fn archive_reason_import_validation_is_atomic_for_plain_json_and_modular_inputs() {
    let dir = TempDir::new();
    let secret = format!("ghp_{}", "A".repeat(36));
    let oversized = "é".repeat(MAX_VALUE_BYTES / 2 + 1);
    for reason in [&secret, &oversized] {
        for format in ["plain", "json", "modular"] {
            let mut store = Store::open_in_memory().unwrap();
            store.set("existing/rule", "Do not delete this").unwrap();
            store.session_add("Do not change this checkpoint").unwrap();
            let before = store.export_rules_text().unwrap();
            let input = if format == "json" {
                format!(
                    "[rule-json-v1] {}\n",
                    json!({
                        "key":"unsafe/rule", "val":"Safe value", "kind":"rule",
                        "anchor":null, "archived_at":1, "archive_reason":reason,
                    })
                )
            } else {
                format!("[archived] unsafe/rule = Safe value --reason: {reason}\n")
            };
            let path = if format == "modular" {
                let path = dir.0.join(".agent-rules.d");
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("unsafe.rules"), input).unwrap();
                path
            } else {
                let path = dir.0.join(".agent-rules");
                fs::write(&path, input).unwrap();
                path
            };
            assert!(
                store.sync_with_path(&path, true).is_err(),
                "{format} accepted unsafe reason"
            );
            assert_eq!(store.export_rules_text().unwrap(), before);
            assert_eq!(store.session_list(20).unwrap().len(), 1);
            assert_eq!(store.history("existing/rule").unwrap().len(), 1);
        }
    }
    let mut store = Store::open_in_memory().unwrap();
    let path = dir.0.join("boundary.rules");
    fs::write(
        &path,
        format!(
            "[archived] safe/rule = Safe value --reason: {}\n",
            "a".repeat(MAX_VALUE_BYTES)
        ),
    )
    .unwrap();
    store.sync_with_file(&path).unwrap();
    assert_eq!(
        store
            .get_entry("safe/rule")
            .unwrap()
            .unwrap()
            .archive_reason
            .unwrap()
            .len(),
        MAX_VALUE_BYTES
    );
}
