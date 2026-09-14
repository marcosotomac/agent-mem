use crate::error::Result;
use crate::init::{find_project_root, global_db_path};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

pub struct McpServer {
    project_root: PathBuf,
    global_path: PathBuf,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            project_root: find_project_root(),
            global_path: global_db_path(),
        }
    }

    pub fn with_paths(project_root: PathBuf, global_path: PathBuf) -> Self {
        Self {
            project_root,
            global_path,
        }
    }

    fn project_db_path(&self) -> PathBuf {
        self.project_root.join(".agent-mem").join("mem.db")
    }

    fn open_store(&self, scope: &str, need_write: bool) -> Result<Store> {
        match scope {
            "global" => Store::open(&self.global_path, need_write),
            _ => Store::open(&self.project_db_path(), need_write),
        }
    }

    pub fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone().unwrap_or(Value::Null);

        match req.method.as_str() {
            "initialize" => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "agent-mem",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
                error: None,
            }),

            "notifications/initialized" => None,

            "ping" => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({})),
                error: None,
            }),

            "tools/list" => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "tools": [
                        {
                            "name": "mem_set",
                            "description": "Store rule, decision, or gotcha.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "key": { "type": "string" },
                                    "val": { "type": "string" },
                                    "anchor": { "type": "string", "description": "Code anchor" },
                                    "kind": { "type": "string", "description": "rule|decision|gotcha|pattern" },
                                    "rel": { "type": "string", "description": "rel_type:target" },
                                    "scope": { "type": "string", "enum": ["project", "global"] }
                                },
                                "required": ["key", "val"]
                            }
                        },
                        {
                            "name": "mem_find",
                            "description": "Search memories by BM25 keyword.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": { "type": "string" },
                                    "scope": { "type": "string", "enum": ["all", "project", "global"] }
                                },
                                "required": ["query"]
                            }
                        },
                        {
                            "name": "mem_context",
                            "description": "Export dense memory context block.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "diff": { "type": "boolean" },
                                    "files": { "type": "string" },
                                    "anchor": { "type": "string" },
                                    "topic": { "type": "string" },
                                    "limit": { "type": "integer" },
                                    "scope": { "type": "string", "enum": ["all", "project", "global"] }
                                }
                            }
                        }
                    ]
                })),
                error: None,
            }),

            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));

                match self.dispatch_tool(name, &args) {
                    Ok(text) => Some(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        })),
                        error: None,
                    }),
                    Err(err) => Some(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Error: {}", err)
                                }
                            ],
                            "isError": true
                        })),
                        error: None,
                    }),
                }
            }

            _ => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method '{}' not found", req.method),
                }),
            }),
        }
    }

    fn dispatch_tool(&self, name: &str, args: &Value) -> Result<String> {
        match name {
            "mem_set" => {
                let key = args.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'key'".into())
                })?;
                let val = args.get("val").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'val'".into())
                })?;
                let anchor = args.get("anchor").and_then(|v| v.as_str());
                let kind = args.get("kind").and_then(|v| v.as_str());
                let rel = args.get("rel").and_then(|v| v.as_str());
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("project");

                let mut store = self.open_store(scope, true)?;
                store.set_entry(key, val, anchor, kind)?;
                if let Some(r) = rel
                    && let Some((rel_type, target)) = r.split_once(':')
                {
                    let _ = store.relate(key, rel_type, target);
                }
                if scope == "project" {
                    let root = crate::init::find_project_root();
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        let _ = store.export_to_file(&rules_file);
                    }
                }
                let effective_kind = kind.unwrap_or_else(|| crate::store::infer_kind(key));
                let kind_tag = if effective_kind != "rule" {
                    format!("[{}] ", effective_kind)
                } else {
                    String::new()
                };
                match anchor {
                    Some(a) => Ok(format!("saved [{}] {}{} ({})", scope, kind_tag, key, a)),
                    None => Ok(format!("saved [{}] {}{}", scope, kind_tag, key)),
                }
            }

            "mem_find" => {
                let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'query'".into())
                })?;
                let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");

                let mut lines = Vec::new();

                if (scope == "all" || scope == "project")
                    && let Ok(store) = self.open_store("project", false)
                    && let Ok(results) = store.find(query)
                {
                    for r in results {
                        let kind_tag = if r.kind != "rule" {
                            format!("[{}] ", r.kind)
                        } else {
                            String::new()
                        };
                        let status_tag = if r.is_archived() {
                            r.archive_reason
                                .as_deref()
                                .map(|reason| format!(" [archived: {}]", reason))
                                .unwrap_or_else(|| " [archived]".to_string())
                        } else {
                            String::new()
                        };
                        match r.anchor {
                            Some(anchor) => lines.push(format!(
                                "[project] {}{}: {}{} ({})",
                                kind_tag, r.key, r.val, status_tag, anchor
                            )),
                            None => lines.push(format!(
                                "[project] {}{}: {}{}",
                                kind_tag, r.key, r.val, status_tag
                            )),
                        }
                    }
                }

                if (scope == "all" || scope == "global")
                    && let Ok(store) = self.open_store("global", false)
                    && let Ok(results) = store.find(query)
                {
                    for r in results {
                        let kind_tag = if r.kind != "rule" {
                            format!("[{}] ", r.kind)
                        } else {
                            String::new()
                        };
                        let status_tag = if r.is_archived() {
                            r.archive_reason
                                .as_deref()
                                .map(|reason| format!(" [archived: {}]", reason))
                                .unwrap_or_else(|| " [archived]".to_string())
                        } else {
                            String::new()
                        };
                        match r.anchor {
                            Some(anchor) => lines.push(format!(
                                "[global] {}{}: {}{} ({})",
                                kind_tag, r.key, r.val, status_tag, anchor
                            )),
                            None => lines.push(format!(
                                "[global] {}{}: {}{}",
                                kind_tag, r.key, r.val, status_tag
                            )),
                        }
                    }
                }

                if lines.is_empty() {
                    Ok(format!("No memories found for '{}'", query))
                } else {
                    Ok(lines.join("\n"))
                }
            }

            "mem_context" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");
                let anchor = args.get("anchor").and_then(|v| v.as_str());
                let topic = args.get("topic").and_then(|v| v.as_str());
                let diff = args.get("diff").and_then(|v| v.as_bool()).unwrap_or(false);
                let files_arg = args.get("files").and_then(|v| v.as_str()).unwrap_or("");
                let mut files = Vec::new();
                if !files_arg.is_empty() {
                    for f in files_arg.split(',') {
                        let t = f.trim();
                        if !t.is_empty() {
                            files.push(t.to_string());
                        }
                    }
                }
                if diff {
                    let diff_files = crate::hook::get_diff_files(&self.project_root);
                    for df in diff_files {
                        if !files.contains(&df) {
                            files.push(df);
                        }
                    }
                }

                let mut lines = Vec::new();

                if (scope == "all" || scope == "project")
                    && let Ok(store) = self.open_store("project", false)
                {
                    let (rules, rels, sessions) = if !files.is_empty() {
                        store
                            .context_for_files(&files, topic, limit)
                            .unwrap_or_default()
                    } else {
                        store
                            .context_filtered(anchor, topic, limit)
                            .unwrap_or_default()
                    };
                    if !rules.is_empty() {
                        lines.push("== PROJECT RULES ==".to_string());
                        for r in rules {
                            let kind_badge = if r.kind != "rule" {
                                format!("[{}] ", r.kind)
                            } else {
                                String::new()
                            };
                            match r.anchor {
                                Some(a) => lines
                                    .push(format!("{}{}: {} ({})", kind_badge, r.key, r.val, a)),
                                None => lines.push(format!("{}{}: {}", kind_badge, r.key, r.val)),
                            }
                        }
                    }
                    if !rels.is_empty() {
                        lines.push("== RELATIONS ==".to_string());
                        for rel in rels {
                            lines.push(format!(
                                "{} -> {} -> {}",
                                rel.source_key, rel.rel_type, rel.target_key
                            ));
                        }
                    }
                    if !sessions.is_empty() {
                        lines.push("== SESSIONS ==".to_string());
                        for (id, summary) in sessions {
                            lines.push(format!("[#{}] {}", id, summary));
                        }
                    }
                }

                if (scope == "all" || scope == "global")
                    && let Ok(store) = self.open_store("global", false)
                    && let Ok(rules) = store.dump()
                    && !rules.is_empty()
                {
                    lines.push("== GLOBAL PREFERENCES ==".to_string());
                    for (k, v, a) in rules.into_iter().take(limit) {
                        match a {
                            Some(anchor) => lines.push(format!("{}: {} ({})", k, v, anchor)),
                            None => lines.push(format!("{}: {}", k, v)),
                        }
                    }
                }

                if lines.is_empty() {
                    Ok("No active context recorded.".to_string())
                } else {
                    Ok(lines.join("\n"))
                }
            }

            unknown => Err(crate::error::Error::Usage(format!(
                "Unknown tool: {}",
                unknown
            ))),
        }
    }

    /// Run MCP loop reading line by line from stdin and replying to stdout.
    pub fn run_stdio(&self) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let resp = match serde_json::from_str::<JsonRpcRequest>(&line) {
                Ok(req) => self.handle_request(&req),
                Err(err) => Some(JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", err),
                    }),
                }),
            };

            if let Some(resp) = resp {
                let mut serialized = serde_json::to_string(&resp).map_err(|e| {
                    crate::error::Error::Usage(format!("JSON serialization error: {}", e))
                })?;
                serialized.push('\n');
                stdout.write_all(serialized.as_bytes())?;
                stdout.flush()?;
            }
        }

        Ok(())
    }
}
