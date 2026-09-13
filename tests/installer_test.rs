use agent_mem::installer::{ALL_CLIENTS, TargetClient, install_to_path, strip_json_comments};
use serde_json::{Value, json};
use std::fs;

#[test]
fn test_installer_client_parsing() {
    assert_eq!(TargetClient::parse("claude"), Some(TargetClient::Claude));
    assert_eq!(
        TargetClient::parse("claude-desktop"),
        Some(TargetClient::Claude)
    );
    assert_eq!(TargetClient::parse("cursor"), Some(TargetClient::Cursor));
    assert_eq!(
        TargetClient::parse("antigravity"),
        Some(TargetClient::Antigravity)
    );
    assert_eq!(
        TargetClient::parse("gemini"),
        Some(TargetClient::Antigravity)
    );
    assert_eq!(
        TargetClient::parse("windsurf"),
        Some(TargetClient::Windsurf)
    );
    assert_eq!(TargetClient::parse("codeium"), Some(TargetClient::Windsurf));
    assert_eq!(TargetClient::parse("vscode"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("vs-code"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("code"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("zed"), Some(TargetClient::Zed));
    assert_eq!(
        TargetClient::parse("opencode"),
        Some(TargetClient::OpenCode)
    );
    assert_eq!(TargetClient::parse("roocode"), Some(TargetClient::RooCode));
    assert_eq!(TargetClient::parse("roo-code"), Some(TargetClient::RooCode));
    assert_eq!(TargetClient::parse("cline"), Some(TargetClient::Cline));
    assert_eq!(TargetClient::parse("unknown-editor"), None);

    for client in ALL_CLIENTS {
        assert_eq!(TargetClient::parse(client.cli_id()), Some(*client));
    }
}

#[test]
fn test_installer_strip_json_comments() {
    let jsonc = r#"
    // Header comment
    {
      "key": "value", // inline comment
      "url": "https://example.com/api", /* block comment */
      "nested": {
        /* multiline
           comment */
        "count": 42
      }
    }
    "#;
    let stripped = strip_json_comments(jsonc);
    let parsed: Value = serde_json::from_str(&stripped).expect("valid stripped JSON");
    assert_eq!(parsed["key"], "value");
    assert_eq!(parsed["url"], "https://example.com/api");
    assert_eq!(parsed["nested"]["count"], 42);
}

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
    assert!(!res1.already_configured);
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
    assert!(res2.already_configured);

    let content2: Value = serde_json::from_str(&fs::read_to_string(&res2.path).unwrap()).unwrap();
    // Verify both agent-mem and other-tool exist!
    assert!(content2["mcpServers"]["agent-mem"].is_object());
    assert!(content2["mcpServers"]["other-tool"].is_object());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_vscode_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_vscode_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("mcp.json");

    let res = install_to_path(&config_path, TargetClient::VSCode).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(content["servers"]["agent-mem"]["type"], "stdio");
    assert_eq!(content["servers"]["agent-mem"]["args"][0], "mcp");

    // Idempotency
    let res2 =
        install_to_path(&config_path, TargetClient::VSCode).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_zed_format_with_comments() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_zed_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("settings.json");

    // Simulate existing Zed settings with top comment
    let initial_content = r#"// Zed settings
// Configuration file
{
  "theme": "One Dark",
  "vim": false
}
"#;
    fs::write(&config_path, initial_content).unwrap();

    let res = install_to_path(&config_path, TargetClient::Zed).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(content["theme"], "One Dark");
    assert_eq!(content["vim"], false);
    assert!(content["context_servers"]["agent-mem"].is_object());
    assert_eq!(content["context_servers"]["agent-mem"]["args"][0], "mcp");

    // Idempotency
    let res2 = install_to_path(&config_path, TargetClient::Zed).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_opencode_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_opencode_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("opencode.json");

    let initial = json!({
        "default_agent": "sdd-orchestrator",
        "mcp": {
            "existing": {
                "type": "remote",
                "url": "https://example.com"
            }
        }
    });
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial).unwrap(),
    )
    .unwrap();

    let res =
        install_to_path(&config_path, TargetClient::OpenCode).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(content["default_agent"], "sdd-orchestrator");
    assert_eq!(content["mcp"]["existing"]["type"], "remote");
    assert_eq!(content["mcp"]["agent-mem"]["type"], "local");
    assert_eq!(content["mcp"]["agent-mem"]["command"][1], "mcp");

    // Idempotency
    let res2 =
        install_to_path(&config_path, TargetClient::OpenCode).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_windsurf_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_windsurf_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("mcp_config.json");

    let res =
        install_to_path(&config_path, TargetClient::Windsurf).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert!(content["mcpServers"]["agent-mem"].is_object());
    assert_eq!(content["mcpServers"]["agent-mem"]["args"][0], "mcp");

    // Idempotency
    let res2 =
        install_to_path(&config_path, TargetClient::Windsurf).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}
