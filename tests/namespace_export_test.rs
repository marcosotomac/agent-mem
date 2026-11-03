use agent_mem::store::Store;
use std::fs;
mod support;
use support::TempDir;

#[test]
fn modular_export_coalesces_case_variants_without_losing_keys_or_relations() {
    let dir = TempDir::new();
    let rules = dir.0.join(".agent-rules.d");
    let mut store = Store::open_in_memory().unwrap();
    for key in ["Auth/upper", "auth/lower", "AUTH/archived"] {
        store.set(key, "Distinct advice").unwrap();
    }
    store
        .archive("AUTH/archived", Some("No longer current"))
        .unwrap();
    store
        .relate("Auth/upper", "relates_to", "auth/lower")
        .unwrap();
    fs::create_dir(&rules).unwrap();
    // Simulate a pre-existing shard with the old spelling on either filesystem type.
    fs::write(rules.join("Auth.rules"), "obsolete/key = Old file\n").unwrap();
    store.export_to_dir(&rules).unwrap();
    let names: Vec<_> = fs::read_dir(&rules)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "one namespace shard and one graph shard");
    let contents = fs::read_to_string(rules.join("auth.rules")).unwrap();
    for key in ["Auth/upper", "auth/lower", "AUTH/archived"] {
        assert!(contents.contains(key));
    }
    let mut replica = Store::open_in_memory().unwrap();
    replica.sync_with_dir(&rules, false).unwrap();
    assert_eq!(
        replica.export_rules_text().unwrap(),
        store.export_rules_text().unwrap()
    );
    store.sync_with_dir(&rules, false).unwrap();
    assert!(store.get("Auth/upper").unwrap().is_some());
    assert!(store.get("auth/lower").unwrap().is_some());
}
