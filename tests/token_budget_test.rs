use agent_mem::mcp::{JsonRpcRequest, McpServer};
use serde_json::json;
use std::fs;

#[test]
fn test_mcp_schema_token_budget_ratchet() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_token_ratchet_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let global_db = temp_dir.join("global.db");

    let server = McpServer::with_paths(temp_dir.clone(), global_db);

    let list_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/list".into(),
        params: json!({}),
    };

    let resp = server.handle_request(&list_req).expect("response");
    let result_json = serde_json::to_string(&resp.result).expect("serialize");

    let char_count = result_json.len();
    println!(
        "agent-mem MCP tools/list payload length: {} characters",
        char_count
    );

    assert!(
        char_count < 1050,
        "MCP schema footprint regressed! Current: {} chars (limit: 1050 chars / ~160 tokens)",
        char_count
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
