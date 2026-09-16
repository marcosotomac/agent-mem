use agent_mem::mcp::{JsonRpcRequest, LEGACY_PROTOCOL_VERSION, MODERN_PROTOCOL_VERSION, McpServer};
use agent_mem::store::Store;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn modern_meta(version: &str) -> serde_json::Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[test]
fn test_mcp_dual_era_discovery_and_modern_tools() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_modern_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let server = McpServer::with_paths(temp_dir.clone(), temp_dir.join("global.db"));

    let discover = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!("discover-1")),
        method: "server/discover".into(),
        params: json!({ "_meta": modern_meta(MODERN_PROTOCOL_VERSION) }),
    };
    let result = server.handle_request(&discover).unwrap().result.unwrap();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["supportedVersions"],
        json!([MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION])
    );
    assert!(result["capabilities"]["tools"].is_object());
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "agent-mem"
    );
    assert_eq!(result["cacheScope"], "public");
    assert!(result["ttlMs"].as_u64().unwrap() > 0);

    let list = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/list".into(),
        params: json!({ "_meta": modern_meta(MODERN_PROTOCOL_VERSION) }),
    };
    let result = server.handle_request(&list).unwrap().result.unwrap();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["tools"].as_array().unwrap().len(), 4);
    assert_eq!(result["cacheScope"], "public");

    let call = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(3)),
        method: "tools/call".into(),
        params: json!({
            "_meta": modern_meta(MODERN_PROTOCOL_VERSION),
            "name": "mem_find",
            "arguments": { "query": "missing", "scope": "project" }
        }),
    };
    let result = server.handle_request(&call).unwrap().result.unwrap();
    assert_eq!(result["resultType"], "complete");
    assert!(result["content"][0]["text"].is_string());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_modern_metadata_validation_and_version_error() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_version_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let server = McpServer::with_paths(temp_dir.clone(), temp_dir.join("global.db"));

    let unsupported = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "server/discover".into(),
        params: json!({ "_meta": modern_meta("2099-01-01") }),
    };
    let response = server.handle_request(&unsupported).unwrap();
    let error = response.error.unwrap();
    assert_eq!(error.code, -32022);
    assert_eq!(error.data.as_ref().unwrap()["requested"], "2099-01-01");
    assert_eq!(
        error.data.unwrap()["supported"],
        json!([MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION])
    );

    let missing_capabilities = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/list".into(),
        params: json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION
            }
        }),
    };
    let error = server
        .handle_request(&missing_capabilities)
        .unwrap()
        .error
        .unwrap();
    assert_eq!(error.code, -32602);

    let missing_meta = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(3)),
        method: "server/discover".into(),
        params: json!({}),
    };
    assert_eq!(
        server
            .handle_request(&missing_meta)
            .unwrap()
            .error
            .unwrap()
            .code,
        -32602
    );

    let notification = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: None,
        method: "notifications/cancelled".into(),
        params: json!({ "requestId": 42 }),
    };
    assert!(server.handle_request(&notification).is_none());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_modern_discovery_over_stdio() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_stdio_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-mem"))
        .arg("mcp")
        .current_dir(&temp_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP stdio server");

    let request = json!({
        "jsonrpc": "2.0",
        "id": "discover-stdio",
        "method": "server/discover",
        "params": { "_meta": modern_meta(MODERN_PROTOCOL_VERSION) }
    });
    writeln!(child.stdin.as_mut().unwrap(), "{request}").unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("MCP process exits on EOF");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "stdout must contain exactly one JSON-RPC message"
    );
    let response: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(response["id"], "discover-stdio");
    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(
        response["result"]["supportedVersions"][0],
        MODERN_PROTOCOL_VERSION
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_initialize_and_tools_list() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let global_dir = temp_dir.join("global");
    fs::create_dir_all(&global_dir).unwrap();

    let server = McpServer::with_paths(temp_dir.clone(), global_dir.join("global.db"));

    // 1. Initialize
    let init_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "initialize".into(),
        params: json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "1.0" }
        }),
    };
    let init_resp = server.handle_request(&init_req).expect("response expected");
    assert_eq!(init_resp.id, json!(1));
    let res = init_resp.result.expect("result expected");
    assert_eq!(res["serverInfo"]["name"], "agent-mem");
    assert_eq!(res["protocolVersion"], "2024-11-05");

    // 2. Tools list
    let list_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/list".into(),
        params: json!({}),
    };
    let list_resp = server.handle_request(&list_req).expect("response expected");
    let list_result = list_resp.result.unwrap();
    let tools = list_result["tools"].as_array().unwrap().clone();
    assert_eq!(
        tools.len(),
        4,
        "must have exactly 4 surgical tools to preserve token budget"
    );

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        tool_names,
        vec!["mem_set", "mem_find", "mem_context", "mem_manage"]
    );
    assert!(list_result["resultType"].is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_tool_execution_and_dual_scopes() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_exec_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let global_dir = temp_dir.join("global");
    fs::create_dir_all(&global_dir).unwrap();
    fs::write(temp_dir.join(".agent-rules"), "# isolated MCP test rules\n").unwrap();

    let server = McpServer::with_paths(temp_dir.clone(), global_dir.join("global.db"));

    // 1. mem_set into project scope
    let set_proj_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(10)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": {
                "key": "architecture/auth",
                "val": "JWT RS256 with key rotation",
                "anchor": "src/auth/jwt.rs:42",
                "scope": "project"
            }
        }),
    };
    let resp = server.handle_request(&set_proj_req).unwrap();
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        text,
        "saved [project] architecture/auth (src/auth/jwt.rs:42)"
    );
    assert!(
        fs::read_to_string(temp_dir.join(".agent-rules"))
            .unwrap()
            .contains("architecture/auth")
    );

    // 2. mem_set into global scope
    let set_glob_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(11)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": {
                "key": "personal/editor",
                "val": "Neovim with Zellij terminal multiplexer",
                "scope": "global"
            }
        }),
    };
    let resp = server.handle_request(&set_glob_req).unwrap();
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(text, "saved [global] personal/editor");

    // 3. mem_find across both scopes
    let find_all_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(12)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_find",
            "arguments": {
                "query": "architecture",
                "scope": "all"
            }
        }),
    };
    let resp = server.handle_request(&find_all_req).unwrap();
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("[project] architecture/auth: JWT RS256 with key rotation"));

    // 4. mem_find restricted to global scope
    let find_glob_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(13)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_find",
            "arguments": {
                "query": "Neovim",
                "scope": "global"
            }
        }),
    };
    let resp = server.handle_request(&find_glob_req).unwrap();
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("[global] personal/editor: Neovim with Zellij"));

    // 5. mem_context output
    let context_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(14)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_context",
            "arguments": {
                "limit": 5
            }
        }),
    };
    let resp = server.handle_request(&context_req).unwrap();
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("== PROJECT RULES =="));
    assert!(text.contains("architecture/auth: JWT RS256 with key rotation"));
    assert!(text.contains("== GLOBAL PREFERENCES =="));
    assert!(text.contains("personal/editor: Neovim with Zellij"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_exceptions_and_errors() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_err_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let global_dir = temp_dir.join("global");
    fs::create_dir_all(&global_dir).unwrap();

    let server = McpServer::with_paths(temp_dir.clone(), global_dir.join("global.db"));

    // 1. Unknown method returns -32601 Method not found
    let unknown_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(99)),
        method: "nonexistent_method".into(),
        params: json!({}),
    };
    let resp = server.handle_request(&unknown_req).unwrap();
    assert_eq!(resp.error.as_ref().unwrap().code, -32601);

    // 2. Unknown tool returns error response with isError: true
    let unknown_tool_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(100)),
        method: "tools/call".into(),
        params: json!({
            "name": "unknown_tool",
            "arguments": {}
        }),
    };
    let resp = server.handle_request(&unknown_tool_req).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown tool")
    );

    // 3. mem_set with missing arguments returns error with isError: true
    let missing_arg_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(101)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": { "key": "foo" }
        }),
    };
    let resp = server.handle_request(&missing_arg_req).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Missing required argument 'val'")
    );

    // 4. mem_set with empty key returns error with isError: true
    let empty_key_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(102)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": { "key": "  ", "val": "bar" }
        }),
    };
    let resp = server.handle_request(&empty_key_req).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Memory key cannot be empty")
    );

    // 5. notifications with id: null never receive a response
    let notif_null_id = serde_json::from_str::<JsonRpcRequest>(
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","id":null}"#,
    )
    .unwrap();
    assert!(server.handle_request(&notif_null_id).is_none());

    // 6. mem_set with invalid rel format returns isError: true
    let invalid_rel_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(103)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": { "key": "k", "val": "v", "rel": "malformed_rel" }
        }),
    };
    let resp = server.handle_request(&invalid_rel_req).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Invalid rel format")
    );
    let store = Store::open(&temp_dir.join(".agent-mem").join("mem.db"), false).unwrap();
    assert!(
        store.get_entry("k").unwrap().is_none(),
        "a rejected relation must roll back its memory"
    );
    assert!(
        store.get_all_relations().unwrap().is_empty(),
        "a rejected relation must not leave an edge"
    );
    drop(store);

    // 7. mem_set with invalid scope returns isError: true
    let invalid_scope_set = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(104)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": { "key": "k", "val": "v", "scope": "invalid_scope" }
        }),
    };
    let resp = server.handle_request(&invalid_scope_set).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Invalid scope")
    );

    // 8. mem_find with invalid scope returns isError: true
    let invalid_scope_find = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(105)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_find",
            "arguments": { "query": "test", "scope": "invalid_scope" }
        }),
    };
    let resp = server.handle_request(&invalid_scope_find).unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Invalid scope")
    );

    // 9. Storage error propagation: corrupted database in mem_find returns isError: true, NOT empty results
    drop(server);
    let corrupted_db = temp_dir.join(".agent-mem").join("mem.db");
    fs::create_dir_all(corrupted_db.parent().unwrap()).unwrap();
    fs::write(&corrupted_db, b"THIS IS NOT A VALID SQLITE DATABASE").unwrap();

    let find_corrupted_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(106)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_find",
            "arguments": { "query": "auth", "scope": "project" }
        }),
    };
    let corrupted_server = McpServer::with_paths(temp_dir.clone(), global_dir.join("global.db"));
    let resp = corrupted_server
        .handle_request(&find_corrupted_req)
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["isError"], true);
    let err_msg = res["content"][0]["text"].as_str().unwrap();
    assert!(
        err_msg.contains("Error:"),
        "Expected error message on corrupted DB, got: {}",
        err_msg
    );
    assert!(
        !err_msg.contains("No memories found"),
        "Must not hide corruption as 'No memories found'"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_manage_lifecycle_and_file_sync() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_manage_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let global_dir = temp_dir.join("global");
    fs::create_dir_all(&global_dir).unwrap();

    let server = McpServer::with_paths(temp_dir.clone(), global_dir.join("global.db"));

    // 1. Initialize project with .agent-rules
    agent_mem::init::init_project(&temp_dir).unwrap();

    // 2. Set base rules
    let set_req1 = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": {
                "key": "decision/auth-v1",
                "val": "Use legacy JWT",
                "scope": "project"
            }
        }),
    };
    assert!(
        !server.handle_request(&set_req1).unwrap().result.unwrap()["isError"]
            .as_bool()
            .unwrap_or(false)
    );

    let set_req2 = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": {
                "key": "decision/auth-v2",
                "val": "Use Paseto tokens",
                "scope": "project"
            }
        }),
    };
    assert!(
        !server.handle_request(&set_req2).unwrap().result.unwrap()["isError"]
            .as_bool()
            .unwrap_or(false)
    );

    // 3. Relate auth-v2 -> supersedes -> auth-v1 via mem_manage
    let link_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(3)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "relate",
                "key": "decision/auth-v2",
                "rel": "supersedes:decision/auth-v1"
            }
        }),
    };
    let resp = server.handle_request(&link_req).unwrap();
    let res = resp.result.unwrap();
    assert!(!res["isError"].as_bool().unwrap_or(false));
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("linked [project] decision/auth-v2 -> supersedes -> decision/auth-v1")
    );

    let rules = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(rules.contains("[rel] decision/auth-v2 -> supersedes -> decision/auth-v1"));

    // 4. Archive auth-v1 via mem_manage
    let archive_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(4)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "archive",
                "key": "decision/auth-v1",
                "reason": "superseded by auth-v2"
            }
        }),
    };
    let resp = server.handle_request(&archive_req).unwrap();
    let res = resp.result.unwrap();
    assert!(!res["isError"].as_bool().unwrap_or(false));
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("archived [project] decision/auth-v1 (reason: superseded by auth-v2)")
    );

    let rules = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(rules.contains(
        "[archived] [decision] decision/auth-v1 = Use legacy JWT --reason: superseded by auth-v2"
    ));

    // 5. Unarchive auth-v1 via mem_manage
    let unarchive_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(5)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "unarchive",
                "key": "decision/auth-v1"
            }
        }),
    };
    let resp = server.handle_request(&unarchive_req).unwrap();
    let res = resp.result.unwrap();
    assert!(!res["isError"].as_bool().unwrap_or(false));
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unarchived [project] decision/auth-v1")
    );

    // 6. Unrelate via mem_manage
    let unlink_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(6)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "unrelate",
                "key": "decision/auth-v2",
                "rel": "supersedes:decision/auth-v1"
            }
        }),
    };
    let resp = server.handle_request(&unlink_req).unwrap();
    let res = resp.result.unwrap();
    assert!(!res["isError"].as_bool().unwrap_or(false));
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unlinked [project] decision/auth-v2 -> supersedes -> decision/auth-v1")
    );

    let rules = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(!rules.contains("[rel] decision/auth-v2 -> supersedes -> decision/auth-v1"));

    // 7. Delete auth-v1 via mem_manage
    let del_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(7)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "delete",
                "key": "decision/auth-v1"
            }
        }),
    };
    let resp = server.handle_request(&del_req).unwrap();
    let res = resp.result.unwrap();
    assert!(!res["isError"].as_bool().unwrap_or(false));
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("deleted [project] decision/auth-v1")
    );

    let rules = fs::read_to_string(temp_dir.join(".agent-rules")).unwrap();
    assert!(!rules.contains("decision/auth-v1"));

    // 8. Delete nonexistent returns error
    let del_again = server.handle_request(&del_req).unwrap();
    assert_eq!(del_again.result.unwrap()["isError"], true);

    // 9. Unknown action returns error
    let unknown_action = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(8)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_manage",
            "arguments": {
                "action": "destroy",
                "key": "decision/auth-v2"
            }
        }),
    };
    let resp = server.handle_request(&unknown_action).unwrap();
    assert_eq!(resp.result.unwrap()["isError"], true);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_context_enforces_rule_and_output_budgets() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_budget_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");
    let mut store = Store::open(&db_path, true).unwrap();
    for i in 0..60 {
        store
            .set(&format!("rule/{i:03}"), &format!("compact value {i}"))
            .unwrap();
    }
    drop(store);

    let server = McpServer::with_paths(temp_dir.clone(), temp_dir.join("global.db"));
    let capped_limit = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_context",
            "arguments": { "scope": "project", "limit": u64::MAX }
        }),
    };
    let result = server
        .handle_request(&capped_limit)
        .unwrap()
        .result
        .unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("rule/"))
            .count(),
        50
    );

    let mut store = Store::open(&db_path, true).unwrap();
    store.set("rule/000", &"x".repeat(32 * 1024)).unwrap();
    drop(store);
    let capped_bytes = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_context",
            "arguments": { "scope": "project", "limit": 1 }
        }),
    };
    let result = server
        .handle_request(&capped_bytes)
        .unwrap()
        .result
        .unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.len() <= 16 * 1024);
    assert!(text.ends_with("...[truncated: output budget]"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_invalid_relation_is_rejected_before_storage_io() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_prevalidate_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let server = McpServer::with_paths(temp_dir.clone(), temp_dir.join("global.db"));
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_set",
            "arguments": { "key": "k", "val": "v", "rel": "malformed" }
        }),
    };

    let result = server.handle_request(&request).unwrap().result.unwrap();
    assert_eq!(result["isError"], true);
    assert!(!temp_dir.join(".agent-mem").join("mem.db").exists());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mcp_context_full_rule_packing_and_global_space_reservation() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_mcp_packing_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let project_db = temp_dir.join(".agent-mem").join("mem.db");
    let global_db = temp_dir.join("global.db");

    // 1. Seed project with a huge 10KB rule AND a critical short authorization rule
    let mut p_store = Store::open(&project_db, true).unwrap();
    p_store
        .set(
            "gotcha/traceback",
            &format!("Large stack trace: {}", "A".repeat(10 * 1024)),
        )
        .unwrap();
    p_store
        .set_entry(
            "decision/auth",
            "Require JWT RS256 authentication on all endpoints",
            Some("src/auth/jwt.rs:1"),
            Some("decision"),
        )
        .unwrap();
    let _ = p_store.session_add("feat: implement token verification").unwrap();
    drop(p_store);

    // 2. Seed global preferences
    let mut g_store = Store::open(&global_db, true).unwrap();
    g_store
        .set("personal/style", "Favor immutability and pure functions")
        .unwrap();
    drop(g_store);

    let server = McpServer::with_paths(temp_dir.clone(), global_db);

    // 3. Call mem_context with scope="all" and limit=5
    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({
            "name": "mem_context",
            "arguments": { "scope": "all", "limit": 5 }
        }),
    };
    let res = server.handle_request(&req).unwrap().result.unwrap();
    let text = res["content"][0]["text"].as_str().unwrap();

    // A. Verify critical decision/auth is NOT hidden or crowded out by the large traceback rule
    assert!(
        text.contains("[decision] decision/auth: Require JWT RS256 authentication on all endpoints"),
        "Critical brief authorization rule must be present in full"
    );

    // B. Verify large traceback rule was compactly bounded without starving the rest of the context
    assert!(text.contains("gotcha/traceback"));
    assert!(text.contains("... [truncated; use mem_find key=\"gotcha/traceback\" for full content]"));

    // C. Verify sessions were not crowded out
    assert!(text.contains("== SESSIONS =="));
    assert!(text.contains("feat: implement token verification"));

    // D. Verify global preferences were NOT starved out when scope="all"
    assert!(text.contains("== GLOBAL PREFERENCES =="));
    assert!(text.contains("personal/style: Favor immutability and pure functions"));

    // E. Total text does not exceed budget
    assert!(text.len() <= 16 * 1024);

    let _ = fs::remove_dir_all(&temp_dir);
}
