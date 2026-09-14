use crate::error::Result;
use crate::init::{find_project_root, global_db_path};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::{RefCell, RefMut};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const CACHE_TTL_MS: u64 = 86_400_000;
const DEFAULT_CONTEXT_LIMIT: usize = 10;
const MAX_CONTEXT_LIMIT: usize = 50;
const MAX_TEXT_RESULT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProtocolEra {
    Legacy,
    Modern,
}

enum ProtocolError {
    MissingMeta,
    MissingProtocolVersion,
    MissingClientCapabilities,
    UnsupportedProtocolVersion(String),
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub struct McpServer {
    project_root: PathBuf,
    global_path: PathBuf,
    project_store: RefCell<Option<Store>>,
    global_store: RefCell<Option<Store>>,
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
            project_store: RefCell::new(None),
            global_store: RefCell::new(None),
        }
    }

    pub fn with_paths(project_root: PathBuf, global_path: PathBuf) -> Self {
        Self {
            project_root,
            global_path,
            project_store: RefCell::new(None),
            global_store: RefCell::new(None),
        }
    }

    fn project_db_path(&self) -> PathBuf {
        self.project_root.join(".agent-mem").join("mem.db")
    }

    fn open_store(&self, scope: &str, need_write: bool) -> Result<RefMut<'_, Store>> {
        let (slot, path) = match scope {
            "global" => (&self.global_store, self.global_path.clone()),
            "project" => (&self.project_store, self.project_db_path()),
            other => Err(crate::error::Error::Usage(format!(
                "Invalid scope '{}'. Supported scopes: 'project', 'global'",
                other
            )))?,
        };

        let mut cached = slot.borrow_mut();
        if cached.is_none() {
            *cached = Some(Store::open(&path, need_write)?);
        }
        Ok(RefMut::map(cached, |store| {
            store.as_mut().expect("store cache was initialized")
        }))
    }

    fn open_read_store(&self, scope: &str) -> Result<Option<RefMut<'_, Store>>> {
        match self.open_store(scope, false) {
            Ok(store) => Ok(Some(store)),
            Err(crate::error::Error::NotInitialized) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn parse_relation(raw: &str) -> Result<(&str, &str)> {
        let (rel_type, target) = raw.split_once(':').ok_or_else(|| {
            crate::error::Error::Usage(format!(
                "Invalid rel format '{}'. Expected 'rel_type:target'",
                raw
            ))
        })?;
        let rel_type = rel_type.trim();
        let target = target.trim();
        if rel_type.is_empty() || target.is_empty() {
            return Err(crate::error::Error::Usage(
                "Relation type and target cannot be empty".into(),
            ));
        }
        Ok((rel_type, target))
    }

    fn bounded_text(mut text: String) -> String {
        if text.len() <= MAX_TEXT_RESULT_BYTES {
            return text;
        }
        const NOTICE: &str = "\n...[truncated: output budget]";
        let mut end = MAX_TEXT_RESULT_BYTES.saturating_sub(NOTICE.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(NOTICE);
        text
    }

    #[inline]
    fn response(id: Value, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    #[inline]
    fn error(
        id: Value,
        code: i32,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }

    #[inline]
    fn server_info() -> Value {
        json!({
            "name": "agent-mem",
            "version": env!("CARGO_PKG_VERSION")
        })
    }

    #[inline]
    fn supported_versions() -> Value {
        json!([MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION])
    }

    #[cold]
    fn protocol_error(id: Value, error: ProtocolError) -> JsonRpcResponse {
        match error {
            ProtocolError::MissingMeta => {
                Self::error(id, -32602, "Missing required request _meta", None)
            }
            ProtocolError::MissingProtocolVersion => Self::error(
                id,
                -32602,
                format!("Missing required _meta.{}", PROTOCOL_VERSION_META),
                None,
            ),
            ProtocolError::MissingClientCapabilities => Self::error(
                id,
                -32602,
                format!("Missing required _meta.{}", CLIENT_CAPABILITIES_META),
                None,
            ),
            ProtocolError::UnsupportedProtocolVersion(requested) => Self::error(
                id,
                -32022,
                "Unsupported protocol version",
                Some(json!({
                    "supported": [MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION],
                    "requested": requested
                })),
            ),
        }
    }

    /// Detect modern requests without adding connection state. Legacy requests remain
    /// byte-compatible; modern requests validate their mandatory per-request metadata.
    #[inline]
    fn protocol_era(
        &self,
        req: &JsonRpcRequest,
    ) -> std::result::Result<ProtocolEra, ProtocolError> {
        if req.method == "initialize" {
            return Ok(ProtocolEra::Legacy);
        }

        let meta = req.params.get("_meta");
        let modern_candidate = req.method == "server/discover"
            || meta.is_some_and(|m| {
                m.as_object().is_none_or(|o| {
                    o.contains_key(PROTOCOL_VERSION_META)
                        || o.contains_key(CLIENT_CAPABILITIES_META)
                })
            });

        if !modern_candidate {
            return Ok(ProtocolEra::Legacy);
        }

        let Some(meta) = meta.and_then(Value::as_object) else {
            return Err(ProtocolError::MissingMeta);
        };
        let Some(version) = meta.get(PROTOCOL_VERSION_META).and_then(Value::as_str) else {
            return Err(ProtocolError::MissingProtocolVersion);
        };

        if version != MODERN_PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedProtocolVersion(
                version.to_owned(),
            ));
        }

        if !meta
            .get(CLIENT_CAPABILITIES_META)
            .is_some_and(Value::is_object)
        {
            return Err(ProtocolError::MissingClientCapabilities);
        }

        Ok(ProtocolEra::Modern)
    }

    fn tools_list_result(era: ProtocolEra) -> Value {
        let mut result = json!({
            "tools": [
                {
                    "name": "mem_set",
                    "description": "Store a memory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "val": { "type": "string" },
                            "anchor": { "type": "string", "description": "path[:line]" },
                            "kind": { "type": "string", "enum": ["rule", "decision", "gotcha", "pattern"] },
                            "rel": { "type": "string", "description": "type:target" },
                            "scope": { "type": "string", "enum": ["project", "global"] }
                        },
                        "required": ["key", "val"]
                    }
                },
                {
                    "name": "mem_find",
                    "description": "Search memories.",
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
                    "description": "Get relevant context.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "diff": { "type": "boolean" },
                            "files": { "type": "string", "description": "Comma-separated paths" },
                            "anchor": { "type": "string" },
                            "topic": { "type": "string" },
                            "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                            "scope": { "type": "string", "enum": ["all", "project", "global"] }
                        }
                    }
                },
                {
                    "name": "mem_manage",
                    "description": "Archive, delete, or link memories.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["archive", "unarchive", "delete", "relate", "unrelate"]
                            },
                            "key": { "type": "string" },
                            "reason": { "type": "string" },
                            "rel": { "type": "string", "description": "type:target" },
                            "scope": { "type": "string", "enum": ["project", "global"] }
                        },
                        "required": ["action", "key"]
                    }
                }
            ]
        });

        if era == ProtocolEra::Modern {
            let object = result.as_object_mut().expect("tools result is an object");
            object.insert("resultType".into(), json!("complete"));
            object.insert("ttlMs".into(), json!(CACHE_TTL_MS));
            object.insert("cacheScope".into(), json!("public"));
        }

        result
    }

    pub fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        // JSON-RPC notifications never receive a response per specification.
        if req.id.as_ref().is_none_or(Value::is_null) || req.method.starts_with("notifications/") {
            return None;
        }

        let id = req.id.clone()?;
        let era = match self.protocol_era(req) {
            Ok(era) => era,
            Err(error) => return Some(Self::protocol_error(id, error)),
        };

        match req.method.as_str() {
            "initialize" => Some(Self::response(
                id,
                json!({
                    "protocolVersion": LEGACY_PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": Self::server_info()
                }),
            )),

            "server/discover" => Some(Self::response(
                id,
                json!({
                    "resultType": "complete",
                    "supportedVersions": Self::supported_versions(),
                    "capabilities": { "tools": {} },
                    "_meta": { "io.modelcontextprotocol/serverInfo": Self::server_info() },
                    "ttlMs": CACHE_TTL_MS,
                    "cacheScope": "public"
                }),
            )),

            "ping" => Some(Self::response(
                id,
                if era == ProtocolEra::Modern {
                    json!({ "resultType": "complete" })
                } else {
                    json!({})
                },
            )),

            "tools/list" => Some(Self::response(id, Self::tools_list_result(era))),

            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));

                match self.dispatch_tool(name, &args) {
                    Ok(text) => {
                        let mut result = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        });
                        if era == ProtocolEra::Modern {
                            result["resultType"] = json!("complete");
                        }
                        Some(Self::response(id, result))
                    }
                    Err(err) => {
                        let mut result = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Error: {}", err)
                                }
                            ],
                            "isError": true
                        });
                        if era == ProtocolEra::Modern {
                            result["resultType"] = json!("complete");
                        }
                        Some(Self::response(id, result))
                    }
                }
            }

            _ => Some(Self::error(
                id,
                -32601,
                format!("Method '{}' not found", req.method),
                None,
            )),
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

                let relation = rel.map(Self::parse_relation).transpose()?;
                let mut store = self.open_store(scope, true)?;
                store.set_entry_with_relation(key, val, anchor, kind, relation)?;
                if scope == "project" {
                    let rules_file = self.project_root.join(".agent-rules");
                    if rules_file.exists() {
                        store.export_to_file(&rules_file)?;
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
                if !matches!(scope, "all" | "project" | "global") {
                    return Err(crate::error::Error::Usage(format!(
                        "Invalid scope '{}'. Supported scopes: 'all', 'project', 'global'",
                        scope
                    )));
                }

                let mut lines = Vec::new();

                if (scope == "all" || scope == "project")
                    && let Some(store) = self.open_read_store("project")?
                {
                    let results = store.find(query)?;
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
                    && let Some(store) = self.open_read_store("global")?
                {
                    let results = store.find(query)?;
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
                    Ok(Self::bounded_text(lines.join("\n")))
                }
            }

            "mem_context" => {
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(DEFAULT_CONTEXT_LIMIT)
                    .min(MAX_CONTEXT_LIMIT);
                let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");
                if !matches!(scope, "all" | "project" | "global") {
                    return Err(crate::error::Error::Usage(format!(
                        "Invalid scope '{}'. Supported scopes: 'all', 'project', 'global'",
                        scope
                    )));
                }
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
                let mut remaining_rules = limit;

                if (scope == "all" || scope == "project")
                    && let Some(store) = self.open_read_store("project")?
                {
                    let (rules, rels, sessions) = if !files.is_empty() {
                        store.context_for_files(&files, topic, limit)?
                    } else {
                        store.context_filtered(anchor, topic, limit)?
                    };
                    remaining_rules = remaining_rules.saturating_sub(rules.len());
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

                if remaining_rules > 0
                    && (scope == "all" || scope == "global")
                    && let Some(store) = self.open_read_store("global")?
                {
                    let rules = store.dump_limited(remaining_rules)?;
                    if !rules.is_empty() {
                        lines.push("== GLOBAL PREFERENCES ==".to_string());
                        for (k, v, a) in rules {
                            match a {
                                Some(anchor) => lines.push(format!("{}: {} ({})", k, v, anchor)),
                                None => lines.push(format!("{}: {}", k, v)),
                            }
                        }
                    }
                }

                if lines.is_empty() {
                    Ok("No active context recorded.".to_string())
                } else {
                    Ok(Self::bounded_text(lines.join("\n")))
                }
            }

            "mem_manage" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'action'".into())
                })?;
                let key = args.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'key'".into())
                })?;
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("project");

                let parsed_relation = match action {
                    "archive" | "unarchive" | "delete" | "del" | "rm" => None,
                    "relate" | "link" | "unrelate" | "unlink" => {
                        let rel = args.get("rel").and_then(|v| v.as_str()).ok_or_else(|| {
                            crate::error::Error::Usage(
                                "Missing required argument 'rel'. Expected 'rel_type:target'"
                                    .into(),
                            )
                        })?;
                        Some(Self::parse_relation(rel)?)
                    }
                    unknown => {
                        return Err(crate::error::Error::Usage(format!(
                            "Unknown manage action '{}'. Supported actions: 'archive', 'unarchive', 'delete', 'relate', 'unrelate'",
                            unknown
                        )));
                    }
                };

                let mut store = self.open_store(scope, true)?;

                let msg = match action {
                    "archive" => {
                        let reason = args.get("reason").and_then(|v| v.as_str());
                        let archived = store.archive(key, reason)?;
                        if !archived {
                            return Err(crate::error::Error::NotFound(key.to_string()));
                        }
                        if scope == "project" {
                            let rules_file = self.project_root.join(".agent-rules");
                            if rules_file.exists() {
                                store.export_to_file(&rules_file)?;
                            }
                        }
                        match reason {
                            Some(r) => format!("archived [{}] {} (reason: {})", scope, key, r),
                            None => format!("archived [{}] {}", scope, key),
                        }
                    }
                    "unarchive" => {
                        let unarchived = store.unarchive(key)?;
                        if !unarchived {
                            return Err(crate::error::Error::NotFound(key.to_string()));
                        }
                        if scope == "project" {
                            let rules_file = self.project_root.join(".agent-rules");
                            if rules_file.exists() {
                                store.export_to_file(&rules_file)?;
                            }
                        }
                        format!("unarchived [{}] {}", scope, key)
                    }
                    "delete" | "del" | "rm" => {
                        let deleted = store.del(key)?;
                        if !deleted {
                            return Err(crate::error::Error::NotFound(key.to_string()));
                        }
                        if scope == "project" {
                            let rules_file = self.project_root.join(".agent-rules");
                            if rules_file.exists() {
                                store.export_to_file(&rules_file)?;
                            }
                        }
                        format!("deleted [{}] {}", scope, key)
                    }
                    "relate" | "link" => {
                        let (rel_type, target) = parsed_relation.expect("relation was validated");
                        store.relate(key, rel_type, target)?;
                        if scope == "project" {
                            let rules_file = self.project_root.join(".agent-rules");
                            if rules_file.exists() {
                                store.export_to_file(&rules_file)?;
                            }
                        }
                        format!("linked [{}] {} -> {} -> {}", scope, key, rel_type, target)
                    }
                    "unrelate" | "unlink" => {
                        let (rel_type, target) = parsed_relation.expect("relation was validated");
                        let unlinked = store.unrelate(key, rel_type, target)?;
                        if !unlinked {
                            return Err(crate::error::Error::NotFound(format!(
                                "relation {} -> {} -> {}",
                                key, rel_type, target
                            )));
                        }
                        if scope == "project" {
                            let rules_file = self.project_root.join(".agent-rules");
                            if rules_file.exists() {
                                store.export_to_file(&rules_file)?;
                            }
                        }
                        format!("unlinked [{}] {} -> {} -> {}", scope, key, rel_type, target)
                    }
                    _ => unreachable!("action was validated before opening the store"),
                };
                Ok(msg)
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
                        data: None,
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
