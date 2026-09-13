use agent_mem::mcp::{JsonRpcRequest, McpServer};
use serde_json::json;
use std::fs;

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
    let tools = list_resp.result.unwrap()["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        tools.len(),
        3,
        "must have exactly 3 surgical tools to preserve token budget"
    );

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tool_names, vec!["mem_set", "mem_find", "mem_context"]);

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

    let _ = fs::remove_dir_all(&temp_dir);
}
