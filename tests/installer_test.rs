use agent_mem::installer::{
    ALL_CLIENTS, TargetClient, install_detected_clients_with_home, install_to_path,
    strip_json_comments, uninstall_from_path,
};
use serde_json::{Value, json};
use std::fs;

#[test]
fn test_installer_client_parsing() {
    assert_eq!(TargetClient::parse("claude"), Some(TargetClient::Claude));
    assert_eq!(
        TargetClient::parse("claude-desktop"),
        Some(TargetClient::Claude)
    );
    assert_eq!(
        TargetClient::parse("claude-code"),
        Some(TargetClient::ClaudeCode)
    );
    assert_eq!(TargetClient::parse("codex"), Some(TargetClient::Codex));
    assert_eq!(
        TargetClient::parse("openai-codex"),
        Some(TargetClient::Codex)
    );
    assert_eq!(TargetClient::parse("cursor"), Some(TargetClient::Cursor));
    assert_eq!(
        TargetClient::parse("antigravity"),
        Some(TargetClient::Antigravity)
    );
    assert_eq!(TargetClient::parse("gemini"), Some(TargetClient::Gemini));
    assert_eq!(
        TargetClient::parse("gemini-cli"),
        Some(TargetClient::Gemini)
    );
    assert_eq!(
        TargetClient::parse("windsurf"),
        Some(TargetClient::Windsurf)
    );
    assert_eq!(TargetClient::parse("codeium"), Some(TargetClient::Windsurf));
    assert_eq!(TargetClient::parse("vscode"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("vs-code"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("code"), Some(TargetClient::VSCode));
    assert_eq!(TargetClient::parse("trae"), Some(TargetClient::Trae));
    assert_eq!(TargetClient::parse("zed"), Some(TargetClient::Zed));
    assert_eq!(
        TargetClient::parse("opencode"),
        Some(TargetClient::OpenCode)
    );
    assert_eq!(TargetClient::parse("roocode"), Some(TargetClient::RooCode));
    assert_eq!(TargetClient::parse("roo-code"), Some(TargetClient::RooCode));
    assert_eq!(TargetClient::parse("cline"), Some(TargetClient::Cline));
    assert_eq!(
        TargetClient::parse("continue"),
        Some(TargetClient::Continue)
    );
    assert_eq!(
        TargetClient::parse("continue-dev"),
        Some(TargetClient::Continue)
    );
    assert_eq!(TargetClient::parse("kiro"), Some(TargetClient::Kiro));
    assert_eq!(TargetClient::parse("qwen"), Some(TargetClient::Qwen));
    assert_eq!(
        TargetClient::parse("kilocode"),
        Some(TargetClient::KiloCode)
    );
    assert_eq!(TargetClient::parse("kilo"), Some(TargetClient::KiloCode));
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

#[test]
fn test_installer_codex_toml_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_codex_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("config.toml");

    // Existing TOML configuration
    let initial_toml = r#"model = "gpt-5.6-sol"
personality = "pragmatic"

[mcp_servers.codegraph]
command = "codegraph"
args = ["serve", "--mcp"]
"#;
    fs::write(&config_path, initial_toml).unwrap();

    // 1. Install agent-mem into Codex TOML
    let res = install_to_path(&config_path, TargetClient::Codex).expect("install should succeed");
    assert!(!res.already_configured);

    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("[mcp_servers.agent-mem]"));
    assert!(content.contains("args = [\"mcp\"]"));
    // Ensure existing settings were preserved
    assert!(content.contains("model = \"gpt-5.6-sol\""));
    assert!(content.contains("[mcp_servers.codegraph]"));

    // 2. Idempotency test
    let res2 =
        install_to_path(&config_path, TargetClient::Codex).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_claude_code_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_claude_code_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join(".claude.json");

    let res =
        install_to_path(&config_path, TargetClient::ClaudeCode).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(content["mcpServers"]["agent-mem"]["type"], "stdio");
    assert_eq!(content["mcpServers"]["agent-mem"]["args"][0], "mcp");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_installer_continue_format() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_continue_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("config.json");

    // Array format test
    let initial = json!({
        "models": [],
        "mcpServers": [
            {
                "name": "other-tool",
                "command": "other",
                "args": []
            }
        ]
    });
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial).unwrap(),
    )
    .unwrap();

    let res =
        install_to_path(&config_path, TargetClient::Continue).expect("install should succeed");
    assert!(!res.already_configured);

    let content: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    let arr = content["mcpServers"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[1]["name"], "agent-mem");
    assert_eq!(arr[1]["args"][0], "mcp");

    // Idempotency
    let res2 =
        install_to_path(&config_path, TargetClient::Continue).expect("re-install should succeed");
    assert!(res2.already_configured);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_install_detected_clients_with_mock_home() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_detected_clients_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let mock_home = temp_dir.join("home");
    fs::create_dir_all(mock_home.join(".cursor")).unwrap();

    let results = install_detected_clients_with_home(&mock_home)
        .expect("auto detection with mock home should succeed");

    assert!(!results.is_empty());
    let has_cursor = results.iter().any(|c| c.client == TargetClient::Cursor);
    assert!(has_cursor);

    let cursor_config = mock_home.join(".cursor").join("mcp.json");
    assert!(cursor_config.exists());
    let content = fs::read_to_string(&cursor_config).unwrap();
    assert!(content.contains("agent-mem"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_uninstall_from_path_json_and_toml() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_uninstall_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Test standard mcpServers object (Cursor)
    let cursor_path = temp_dir.join("cursor_mcp.json");
    let initial_cursor = json!({
        "mcpServers": {
            "agent-mem": { "command": "agent-mem", "args": ["mcp"] },
            "other-server": { "command": "other", "args": ["serve"] }
        }
    });
    fs::write(
        &cursor_path,
        serde_json::to_string_pretty(&initial_cursor).unwrap(),
    )
    .unwrap();

    let un_res = uninstall_from_path(&cursor_path, TargetClient::Cursor).unwrap();
    assert!(un_res.removed);

    let parsed_cursor: Value =
        serde_json::from_str(&fs::read_to_string(&cursor_path).unwrap()).unwrap();
    assert!(parsed_cursor["mcpServers"]["agent-mem"].is_null());
    assert!(parsed_cursor["mcpServers"]["other-server"].is_object());

    // Idempotent uninstall
    let un_res2 = uninstall_from_path(&cursor_path, TargetClient::Cursor).unwrap();
    assert!(!un_res2.removed);

    // 2. Test Zed context_servers
    let zed_path = temp_dir.join("zed_settings.json");
    let initial_zed = json!({
        "context_servers": {
            "agent-mem": { "command": "agent-mem", "args": ["mcp"] },
            "keep-me": { "command": "keep" }
        }
    });
    fs::write(
        &zed_path,
        serde_json::to_string_pretty(&initial_zed).unwrap(),
    )
    .unwrap();

    let un_zed = uninstall_from_path(&zed_path, TargetClient::Zed).unwrap();
    assert!(un_zed.removed);
    let parsed_zed: Value = serde_json::from_str(&fs::read_to_string(&zed_path).unwrap()).unwrap();
    assert!(parsed_zed["context_servers"]["agent-mem"].is_null());
    assert!(parsed_zed["context_servers"]["keep-me"].is_object());

    // 3. Test Continue mcpServers array
    let cont_path = temp_dir.join("continue_config.json");
    let initial_cont = json!({
        "mcpServers": [
            { "name": "agent-mem", "command": "agent-mem", "args": ["mcp"] },
            { "name": "preserve-this", "command": "preserve" }
        ]
    });
    fs::write(
        &cont_path,
        serde_json::to_string_pretty(&initial_cont).unwrap(),
    )
    .unwrap();

    let un_cont = uninstall_from_path(&cont_path, TargetClient::Continue).unwrap();
    assert!(un_cont.removed);
    let parsed_cont: Value =
        serde_json::from_str(&fs::read_to_string(&cont_path).unwrap()).unwrap();
    let arr = parsed_cont["mcpServers"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "preserve-this");

    // 4. Test Codex TOML
    let codex_path = temp_dir.join("codex_config.toml");
    let toml_initial = "\
[editor]
theme = \"dark\"

[mcp_servers.agent-mem]
command = \"agent-mem\"
args = [\"mcp\"]

[mcp_servers.other]
command = \"other\"
";
    fs::write(&codex_path, toml_initial).unwrap();

    let un_codex = uninstall_from_path(&codex_path, TargetClient::Codex).unwrap();
    assert!(un_codex.removed);
    let toml_after = fs::read_to_string(&codex_path).unwrap();
    assert!(!toml_after.contains("[mcp_servers.agent-mem]"));
    assert!(toml_after.contains("[editor]"));
    assert!(toml_after.contains("[mcp_servers.other]"));

    let _ = fs::remove_dir_all(&temp_dir);
}
