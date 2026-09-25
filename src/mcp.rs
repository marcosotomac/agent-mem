use crate::error::Result;
use crate::init::{find_project_root, global_db_path};
use crate::registry::{ProjectRecord, ProjectRegistry};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const CACHE_TTL_MS: u64 = 86_400_000;
const DEFAULT_CONTEXT_LIMIT: usize = 10;
const MAX_CONTEXT_LIMIT: usize = 50;
const MAX_TEXT_RESULT_BYTES: usize = 16 * 1024;
const MAX_BATCH_ITEMS: usize = 256;
const MAX_BATCH_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

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
    default_project_root: Option<PathBuf>,
    allow_project_create: bool,
    projects: Vec<ProjectRecord>,
    registry_error: Option<String>,
    global_path: PathBuf,
    project_stores: RefCell<HashMap<PathBuf, Store>>,
    global_store: RefCell<Option<Store>>,
    read_only: bool,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        Self::new_with_mode(false)
    }

    pub fn new_with_mode(read_only: bool) -> Self {
        let read_only = read_only
            || std::env::var("AGENT_MEM_READ_ONLY")
                .ok()
                .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let root = find_project_root();
        let default_project_root = root
            .join(".agent-mem")
            .join("mem.db")
            .exists()
            .then(|| ProjectRegistry::canonicalize_path(&root));
        let (projects, registry_error) = match ProjectRegistry::load() {
            Ok(registry) => (registry.projects, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        Self {
            default_project_root,
            allow_project_create: false,
            projects,
            registry_error,
            global_path: global_db_path(),
            project_stores: RefCell::new(HashMap::new()),
            global_store: RefCell::new(None),
            read_only,
        }
    }

    pub fn with_paths(project_root: PathBuf, global_path: PathBuf) -> Self {
        Self {
            default_project_root: Some(ProjectRegistry::canonicalize_path(&project_root)),
            allow_project_create: true,
            projects: Vec::new(),
            registry_error: None,
            global_path,
            project_stores: RefCell::new(HashMap::new()),
            global_store: RefCell::new(None),
            read_only: false,
        }
    }

    pub fn with_paths_read_only(project_root: PathBuf, global_path: PathBuf) -> Self {
        let mut server = Self::with_paths(project_root, global_path);
        server.read_only = true;
        server
    }

    fn live_registered_projects(&self) -> impl Iterator<Item = &ProjectRecord> {
        self.projects.iter().filter(|project| {
            Path::new(&project.canonical_path)
                .join(".agent-mem")
                .join("mem.db")
                .exists()
        })
    }

    fn resolve_project_root(&self, selector: Option<&str>) -> Result<PathBuf> {
        if let Some(error) = &self.registry_error {
            return Err(crate::error::Error::Usage(error.clone()));
        }
        if let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) {
            if let Some(default) = &self.default_project_root
                && (selector == default.to_string_lossy()
                    || selector
                        == default
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(""))
            {
                return Ok(default.clone());
            }

            let matches: Vec<&ProjectRecord> = self
                .live_registered_projects()
                .filter(|project| {
                    project.id == selector
                        || project.name == selector
                        || project.canonical_path == selector
                })
                .collect();
            return match matches.as_slice() {
                [project] => Ok(PathBuf::from(&project.canonical_path)),
                [] => Err(crate::error::Error::Usage(format!(
                    "Unknown project '{selector}'. Use an id, unique name, or registered path"
                ))),
                _ => Err(crate::error::Error::Usage(format!(
                    "Ambiguous project name '{selector}'. Use its id or registered path"
                ))),
            };
        }

        if let Some(default) = &self.default_project_root {
            return Ok(default.clone());
        }

        let projects: Vec<&ProjectRecord> = self.live_registered_projects().collect();
        match projects.as_slice() {
            [project] => Ok(PathBuf::from(&project.canonical_path)),
            [] => Err(crate::error::Error::Usage(
                "No initialized project is available. Run 'agent-mem init' in a repository".into(),
            )),
            _ => {
                let choices = projects
                    .iter()
                    .take(8)
                    .map(|project| format!("{}:{}", project.id, project.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(crate::error::Error::Usage(format!(
                    "Project is required. Registered projects: {choices}"
                )))
            }
        }
    }

    fn project_for_scope(&self, args: &Value, scope: &str) -> Result<Option<PathBuf>> {
        if scope == "global" {
            return Ok(None);
        }
        self.resolve_project_root(args.get("project").and_then(Value::as_str))
            .map(Some)
    }

    fn open_project_store(&self, root: &Path, need_write: bool) -> Result<RefMut<'_, Store>> {
        let root = ProjectRegistry::canonicalize_path(root);
        let db_path = root.join(".agent-mem").join("mem.db");
        if !db_path.exists() && !self.allow_project_create {
            return Err(crate::error::Error::NotInitialized);
        }

        let mut stores = self.project_stores.borrow_mut();
        if !stores.contains_key(&root) {
            stores.insert(root.clone(), Store::open(&db_path, need_write)?);
        }
        Ok(RefMut::map(stores, |stores| {
            stores
                .get_mut(&root)
                .expect("project store cache initialized")
        }))
    }

    fn open_global_store(&self, need_write: bool) -> Result<RefMut<'_, Store>> {
        let mut cached = self.global_store.borrow_mut();
        if cached.is_none() {
            *cached = Some(Store::open(&self.global_path, need_write)?);
        }
        Ok(RefMut::map(cached, |store| {
            store.as_mut().expect("store cache was initialized")
        }))
    }

    fn open_store(
        &self,
        scope: &str,
        project_root: Option<&Path>,
        need_write: bool,
    ) -> Result<RefMut<'_, Store>> {
        match scope {
            "global" => self.open_global_store(need_write),
            "project" => self.open_project_store(
                project_root.ok_or_else(|| {
                    crate::error::Error::Usage("Project scope requires a project".into())
                })?,
                need_write,
            ),
            other => Err(crate::error::Error::Usage(format!(
                "Invalid scope '{other}'. Supported scopes: 'project', 'global'"
            ))),
        }
    }

    fn open_read_store(
        &self,
        scope: &str,
        project_root: Option<&Path>,
    ) -> Result<Option<RefMut<'_, Store>>> {
        match self.open_store(scope, project_root, false) {
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

    fn tools_list_result(&self, era: ProtocolEra) -> Value {
        let mut result = json!({
            "tools": [
                {
                    "name": "mem_set",
                    "description": "Store a memory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "batch": { "type": "array" },
                            "key": { "type": "string" },
                            "val": { "type": "string" },
                            "anchor": { "type": "string" },
                            "kind": { "type": "string", "enum": ["rule", "decision", "gotcha", "pattern"] },
                            "rel": { "type": "string" },
                            "project": { "type": "string" },
                            "scope": { "type": "string", "enum": ["project", "global"] }
                        }
                    }
                },
                {
                    "name": "mem_find",
                    "description": "Search memories.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "project": { "type": "string" },
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
                            "files": { "type": "string" },
                            "anchor": { "type": "string" },
                            "topic": { "type": "string" },
                            "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                            "project": { "type": "string" },
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
                            "rel": { "type": "string" },
                            "project": { "type": "string" },
                            "scope": { "type": "string", "enum": ["project", "global"] }
                        },
                        "required": ["action", "key"]
                    }
                }
            ]
        });

        if self.read_only
            && let Some(tools) = result.get_mut("tools").and_then(Value::as_array_mut)
        {
            tools.retain(|tool| {
                matches!(
                    tool.get("name").and_then(Value::as_str),
                    Some("mem_find" | "mem_context")
                )
            });
        }

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

            "tools/list" => Some(Self::response(id, self.tools_list_result(era))),

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

    fn format_context_rule(
        kind: &str,
        key: &str,
        val: &str,
        anchor: Option<&str>,
        compact: bool,
        trust_badge: &str,
    ) -> String {
        let kind_badge = if kind != "rule" {
            format!("[{}] ", kind)
        } else {
            String::new()
        };
        let effective_val = if compact && val.len() > 1024 {
            let mut end = 1024;
            while !val.is_char_boundary(end) {
                end -= 1;
            }
            format!(
                "{} ... [truncated; use mem_find key=\"{}\" for full content]",
                &val[..end],
                key
            )
        } else {
            val.to_string()
        };
        match anchor {
            Some(a) => format!(
                "{}{}{}: {} ({})",
                trust_badge, kind_badge, key, effective_val, a
            ),
            None => format!("{}{}{}: {}", trust_badge, kind_badge, key, effective_val),
        }
    }

    fn trust_badge(store: &Store, key: &str) -> Result<String> {
        Ok(match store.metadata(key)? {
            Some(metadata) if metadata.trust == crate::store::TRUST_UNTRUSTED => {
                format!("[untrusted:{}] ", metadata.provenance)
            }
            _ => String::new(),
        })
    }

    fn dispatch_tool(&self, name: &str, args: &Value) -> Result<String> {
        if self.read_only && matches!(name, "mem_set" | "mem_manage") {
            return Err(crate::error::Error::Usage(
                "MCP server is read-only; write tools are disabled".into(),
            ));
        }
        match name {
            "mem_set" => {
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("project");
                let project_root = self.project_for_scope(args, scope)?;

                if let Some(batch_val) = args.get("batch").and_then(|v| v.as_array()) {
                    if batch_val.is_empty() {
                        return Err(crate::error::Error::Usage(
                            "Batch array cannot be empty".into(),
                        ));
                    }
                    if batch_val.len() > MAX_BATCH_ITEMS {
                        return Err(crate::error::Error::Usage(format!(
                            "Batch exceeds the {MAX_BATCH_ITEMS}-item limit"
                        )));
                    }

                    let batch_bytes = batch_val.iter().try_fold(0usize, |total, item| {
                        let item_bytes = ["key", "val", "anchor", "kind", "rel"]
                            .iter()
                            .filter_map(|field| item.get(field).and_then(Value::as_str))
                            .try_fold(0usize, |sum, value| sum.checked_add(value.len()))?;
                        total.checked_add(item_bytes)
                    });
                    if batch_bytes.is_none_or(|bytes| bytes > MAX_BATCH_BYTES) {
                        return Err(crate::error::Error::Usage(format!(
                            "Batch exceeds the {MAX_BATCH_BYTES}-byte payload limit"
                        )));
                    }

                    struct ParsedBatchItem {
                        key: String,
                        val: String,
                        anchor: Option<String>,
                        kind: Option<String>,
                        rel: Option<(String, String)>,
                    }

                    let mut items = Vec::with_capacity(batch_val.len());
                    for (idx, item) in batch_val.iter().enumerate() {
                        let key = item.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                            crate::error::Error::Usage(format!("Batch item #{} missing 'key'", idx))
                        })?;
                        let val = item.get("val").and_then(|v| v.as_str()).ok_or_else(|| {
                            crate::error::Error::Usage(format!("Batch item #{} missing 'val'", idx))
                        })?;
                        let anchor = item
                            .get("anchor")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let kind = item.get("kind").and_then(|v| v.as_str()).map(String::from);
                        let rel = item
                            .get("rel")
                            .and_then(|v| v.as_str())
                            .map(Self::parse_relation)
                            .transpose()?
                            .map(|(t, r)| (t.to_string(), r.to_string()));

                        items.push(ParsedBatchItem {
                            key: key.to_string(),
                            val: val.to_string(),
                            anchor,
                            kind,
                            rel,
                        });
                    }

                    let batch_rules: Vec<crate::store::BatchRule> = items
                        .iter()
                        .map(|it| crate::store::BatchRule {
                            key: &it.key,
                            val: &it.val,
                            anchor: it.anchor.as_deref(),
                            kind: it.kind.as_deref(),
                            relation: it.rel.as_ref().map(|(t, r)| (t.as_str(), r.as_str())),
                        })
                        .collect();

                    let mut store = self.open_store(scope, project_root.as_deref(), true)?;
                    let provenance = format!("mcp:{scope}");
                    let count = store.set_batch_with_metadata(
                        &batch_rules,
                        &provenance,
                        crate::store::TRUST_LOCAL,
                    )?;
                    if scope == "project" {
                        let rules_file = project_root
                            .as_ref()
                            .expect("project scope was resolved")
                            .join(".agent-rules");
                        if rules_file.exists() {
                            store.export_to_file(&rules_file)?;
                        }
                    }
                    return Ok(format!("saved [{}] {} memories in batch", scope, count));
                }

                let key = args.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'key'".into())
                })?;
                let val = args.get("val").and_then(|v| v.as_str()).ok_or_else(|| {
                    crate::error::Error::Usage("Missing required argument 'val'".into())
                })?;
                let anchor = args.get("anchor").and_then(|v| v.as_str());
                let kind = args.get("kind").and_then(|v| v.as_str());
                let rel = args.get("rel").and_then(|v| v.as_str());

                let relation = rel.map(Self::parse_relation).transpose()?;
                let mut store = self.open_store(scope, project_root.as_deref(), true)?;
                let provenance = format!("mcp:{scope}");
                store.set_batch_with_metadata(
                    &[crate::store::BatchRule {
                        key,
                        val,
                        anchor,
                        kind,
                        relation,
                    }],
                    &provenance,
                    crate::store::TRUST_LOCAL,
                )?;
                if scope == "project" {
                    let rules_file = project_root
                        .as_ref()
                        .expect("project scope was resolved")
                        .join(".agent-rules");
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
                let project_root = self.project_for_scope(args, scope)?;

                let mut lines = Vec::new();

                if (scope == "all" || scope == "project")
                    && let Some(store) = self.open_read_store("project", project_root.as_deref())?
                {
                    let results = store.find(query)?;
                    for r in results {
                        let trust_tag = Self::trust_badge(&store, &r.key)?;
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
                                "[project] {}{}{}: {}{} ({})",
                                trust_tag, kind_tag, r.key, r.val, status_tag, anchor
                            )),
                            None => lines.push(format!(
                                "[project] {}{}{}: {}{}",
                                trust_tag, kind_tag, r.key, r.val, status_tag
                            )),
                        }
                    }
                }

                if (scope == "all" || scope == "global")
                    && let Some(store) = self.open_read_store("global", None)?
                {
                    let results = store.find(query)?;
                    for r in results {
                        let trust_tag = Self::trust_badge(&store, &r.key)?;
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
                                "[global] {}{}{}: {}{} ({})",
                                trust_tag, kind_tag, r.key, r.val, status_tag, anchor
                            )),
                            None => lines.push(format!(
                                "[global] {}{}{}: {}{}",
                                trust_tag, kind_tag, r.key, r.val, status_tag
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
                let project_root = self.project_for_scope(args, scope)?;
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
                    let root = project_root.as_ref().ok_or_else(|| {
                        crate::error::Error::Usage(
                            "diff context requires project or all scope".into(),
                        )
                    })?;
                    let diff_files = crate::hook::get_diff_files(root);
                    for df in diff_files {
                        if !files.contains(&df) {
                            files.push(df);
                        }
                    }
                }

                let compact = limit > 1;

                let (project_rule_limit, mut global_rule_limit) = if scope == "all" {
                    let global_reserved = if limit >= 4 {
                        (limit / 4).clamp(2, 10)
                    } else {
                        1
                    };
                    let proj_lim = limit.saturating_sub(global_reserved).max(1);
                    (proj_lim, global_reserved)
                } else if scope == "project" {
                    (limit, 0)
                } else {
                    (0, limit)
                };

                const NOTICE_LEN: usize = 32;
                let total_byte_budget = MAX_TEXT_RESULT_BYTES.saturating_sub(NOTICE_LEN);
                let mut current_bytes = 0;

                let reserved_global_bytes = if scope == "all" { 2560 } else { 0 };
                let reserved_meta_bytes = if scope == "all" || scope == "project" {
                    1536
                } else {
                    0
                };

                let project_byte_budget = total_byte_budget
                    .saturating_sub(reserved_global_bytes)
                    .saturating_sub(reserved_meta_bytes);

                let mut lines = Vec::new();
                let mut actual_project_rule_count = 0;

                if (scope == "all" || scope == "project")
                    && let Some(store) = self.open_read_store("project", project_root.as_deref())?
                {
                    let (rules, rels, sessions) = if !files.is_empty() {
                        store.context_for_files(&files, topic, project_rule_limit)?
                    } else {
                        store.context_filtered(anchor, topic, project_rule_limit)?
                    };

                    actual_project_rule_count = rules.len();

                    let mut project_rule_lines = Vec::new();
                    for r in rules {
                        let trust_badge = Self::trust_badge(&store, &r.key)?;
                        let line = Self::format_context_rule(
                            &r.kind,
                            &r.key,
                            &r.val,
                            r.anchor.as_deref(),
                            compact,
                            &trust_badge,
                        );
                        let cost = line.len() + 1;
                        if current_bytes + cost <= project_byte_budget {
                            current_bytes += cost;
                            project_rule_lines.push(line);
                        } else if project_rule_lines.is_empty() {
                            // First rule is allowed even if long (e.g. limit=1 with large payload)
                            current_bytes += cost;
                            project_rule_lines.push(line);
                            break;
                        }
                    }

                    if !project_rule_lines.is_empty() {
                        let header = "== PROJECT RULES ==";
                        current_bytes += header.len() + 1;
                        lines.push(header.to_string());
                        lines.extend(project_rule_lines);
                    }

                    let mut rel_lines = Vec::new();
                    for rel in rels {
                        let line = format!(
                            "{} -> {} -> {}",
                            rel.source_key, rel.rel_type, rel.target_key
                        );
                        let cost = line.len() + 1;
                        if current_bytes + cost
                            <= total_byte_budget.saturating_sub(reserved_global_bytes)
                        {
                            current_bytes += cost;
                            rel_lines.push(line);
                        }
                    }
                    if !rel_lines.is_empty() {
                        let header = "== RELATIONS ==";
                        current_bytes += header.len() + 1;
                        lines.push(header.to_string());
                        lines.extend(rel_lines);
                    }

                    let mut session_lines = Vec::new();
                    for (id, summary) in sessions {
                        let line = format!("[#{}] {}", id, summary);
                        let cost = line.len() + 1;
                        if current_bytes + cost
                            <= total_byte_budget.saturating_sub(reserved_global_bytes)
                        {
                            current_bytes += cost;
                            session_lines.push(line);
                        }
                    }
                    if !session_lines.is_empty() {
                        let header = "== SESSIONS ==";
                        current_bytes += header.len() + 1;
                        lines.push(header.to_string());
                        lines.extend(session_lines);
                    }
                }

                // If scope is "all", expand global slots to claim any unused project slots
                if scope == "all" {
                    global_rule_limit = limit
                        .saturating_sub(actual_project_rule_count)
                        .max(global_rule_limit);
                }

                if global_rule_limit > 0
                    && (scope == "all" || scope == "global")
                    && let Some(store) = self.open_read_store("global", None)?
                {
                    let rules = store.dump_limited(global_rule_limit)?;
                    let mut global_lines = Vec::new();
                    for (k, v, a) in rules {
                        let trust_badge = Self::trust_badge(&store, &k)?;
                        let line = Self::format_context_rule(
                            "rule",
                            &k,
                            &v,
                            a.as_deref(),
                            compact,
                            &trust_badge,
                        );
                        let cost = line.len() + 1;
                        if current_bytes + cost <= total_byte_budget {
                            current_bytes += cost;
                            global_lines.push(line);
                        }
                    }
                    if !global_lines.is_empty() {
                        lines.push("== GLOBAL PREFERENCES ==".to_string());
                        lines.extend(global_lines);
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
                let project_root = self.project_for_scope(args, scope)?;

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

                let mut store = self.open_store(scope, project_root.as_deref(), true)?;

                let msg = match action {
                    "archive" => {
                        let reason = args.get("reason").and_then(|v| v.as_str());
                        let archived = store.archive(key, reason)?;
                        if !archived {
                            return Err(crate::error::Error::NotFound(key.to_string()));
                        }
                        if scope == "project" {
                            let rules_file = project_root
                                .as_ref()
                                .expect("project scope was resolved")
                                .join(".agent-rules");
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
                            let rules_file = project_root
                                .as_ref()
                                .expect("project scope was resolved")
                                .join(".agent-rules");
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
                            let rules_file = project_root
                                .as_ref()
                                .expect("project scope was resolved")
                                .join(".agent-rules");
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
                            let rules_file = project_root
                                .as_ref()
                                .expect("project scope was resolved")
                                .join(".agent-rules");
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
                            let rules_file = project_root
                                .as_ref()
                                .expect("project scope was resolved")
                                .join(".agent-rules");
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

    fn run_stream<R: BufRead, W: Write>(&self, mut input: R, mut output: W) -> Result<()> {
        loop {
            let mut line = String::new();
            let mut bounded = std::io::Read::take(&mut input, (MAX_REQUEST_BYTES + 1) as u64);
            let bytes_read = bounded.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }
            if line.len() > MAX_REQUEST_BYTES {
                if !line.ends_with('\n') {
                    loop {
                        let buffered = input.fill_buf()?;
                        if buffered.is_empty() {
                            break;
                        }
                        if let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') {
                            input.consume(newline + 1);
                            break;
                        }
                        let buffered_len = buffered.len();
                        input.consume(buffered_len);
                    }
                }
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32600,
                        message: format!("Request exceeds the {MAX_REQUEST_BYTES}-byte limit"),
                        data: None,
                    }),
                };
                serde_json::to_writer(&mut output, &resp).map_err(|e| {
                    crate::error::Error::Usage(format!("JSON serialization error: {e}"))
                })?;
                output.write_all(b"\n")?;
                output.flush()?;
                continue;
            }
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
                output.write_all(serialized.as_bytes())?;
                output.flush()?;
            }
        }

        Ok(())
    }

    /// Run MCP stdio with bounded, newline-delimited JSON-RPC requests.
    pub fn run_stdio(&self) -> Result<()> {
        self.run_stream(io::stdin().lock(), io::stdout())
    }
}

#[cfg(test)]
mod bounded_input_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn oversized_request_is_rejected_and_next_request_is_processed() {
        let root = std::env::temp_dir().join(format!(
            "agent_mem_mcp_bounded_{}_{}",
            std::process::id(),
            crate::store::now_epoch()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let server = McpServer::with_paths(root.clone(), root.join("global.db"));
        let mut input = vec![b'x'; MAX_REQUEST_BYTES + 32];
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let mut output = Vec::new();

        server.run_stream(Cursor::new(input), &mut output).unwrap();

        let responses: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32600);
        assert_eq!(responses[1]["result"], json!({}));
        std::fs::remove_dir_all(root).unwrap();
    }
}
