use agent_mem::mcp::{JsonRpcRequest, MODERN_PROTOCOL_VERSION, McpServer};
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
        char_count < 1450,
        "MCP schema footprint regressed! Current: {} chars (limit: 1450 chars / ~220 tokens)",
        char_count
    );

    let modern_req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(2)),
        method: "tools/list".into(),
        params: json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }),
    };
    let modern_resp = server.handle_request(&modern_req).expect("modern response");
    let modern_chars = serde_json::to_string(&modern_resp.result)
        .expect("serialize modern response")
        .len();
    println!(
        "Modern MCP tools/list: {modern_chars} chars (+{} over legacy)",
        modern_chars - char_count
    );
    assert!(
        modern_chars <= char_count + 80,
        "Modern MCP metadata overhead regressed: legacy={char_count}, modern={modern_chars}"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
