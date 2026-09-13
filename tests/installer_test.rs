use agent_mem::installer::{TargetClient, install_to_path};
use serde_json::{Value, json};
use std::fs;

#[test]
fn test_installer_injects_and_preserves_servers() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_install_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join(".cursor").join("mcp.json");

    // 1. Initial install into Cursor (creates file ~/.cursor/mcp.json)
    let res1 = install_to_path(&config_path, TargetClient::Cursor).expect("install should succeed");
    assert_eq!(res1.already_configured, false);
    assert!(res1.path.exists());

    let content: Value = serde_json::from_str(&fs::read_to_string(&res1.path).unwrap()).unwrap();
    assert_eq!(content["mcpServers"]["agent-mem"]["args"][0], "mcp");

    // 2. Add another server manually to simulate existing user config
    let mut modified = content.clone();
    modified["mcpServers"]["other-tool"] = json!({
        "command": "other-tool",
        "args": ["start"]
    });
    fs::write(&res1.path, serde_json::to_string_pretty(&modified).unwrap()).unwrap();

    // 3. Re-run installer (idempotent update)
    let res2 =
        install_to_path(&config_path, TargetClient::Cursor).expect("re-install should succeed");
    assert_eq!(res2.already_configured, true);

    let content2: Value = serde_json::from_str(&fs::read_to_string(&res2.path).unwrap()).unwrap();
    // Verify both agent-mem and other-tool exist!
    assert!(content2["mcpServers"]["agent-mem"].is_object());
    assert!(content2["mcpServers"]["other-tool"].is_object());

    let _ = fs::remove_dir_all(&temp_dir);
}
