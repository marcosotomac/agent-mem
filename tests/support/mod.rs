//! Isolated fixtures; never mutate the test runner's process environment.
use agent_mem::mcp::{JsonRpcRequest, McpServer};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempDir(pub PathBuf);
#[allow(dead_code)]
impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "agent_mem_regression_{}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn db(&self) -> PathBuf {
        self.0.join(".agent-mem/mem.db")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[allow(dead_code)]
pub fn call(server: &McpServer, name: &str, arguments: Value) -> Value {
    server
        .handle_request(&JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: json!({"name":name,"arguments":arguments}),
        })
        .unwrap()
        .result
        .unwrap()
}
