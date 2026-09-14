use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClient {
    Claude,
    ClaudeCode,
    Codex,
    Cursor,
    Antigravity,
    Gemini,
    Windsurf,
    VSCode,
    Trae,
    Zed,
    OpenCode,
    RooCode,
    Cline,
    Continue,
    Kiro,
    Qwen,
    KiloCode,
}

pub const ALL_CLIENTS: &[TargetClient] = &[
    TargetClient::Antigravity,
    TargetClient::Cursor,
    TargetClient::Claude,
    TargetClient::ClaudeCode,
    TargetClient::Codex,
    TargetClient::Windsurf,
    TargetClient::VSCode,
    TargetClient::Trae,
    TargetClient::Zed,
    TargetClient::OpenCode,
    TargetClient::RooCode,
    TargetClient::Cline,
    TargetClient::Continue,
    TargetClient::Gemini,
    TargetClient::Kiro,
    TargetClient::Qwen,
    TargetClient::KiloCode,
];

impl TargetClient {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_', ' '], "").as_str() {
            "claude" | "claudedesktop" => Some(Self::Claude),
            "claudecode" | "claudecli" => Some(Self::ClaudeCode),
            "codex" | "openaicodex" => Some(Self::Codex),
            "cursor" => Some(Self::Cursor),
            "antigravity" => Some(Self::Antigravity),
            "gemini" | "geminicli" => Some(Self::Gemini),
            "windsurf" | "codeium" => Some(Self::Windsurf),
            "vscode" | "code" => Some(Self::VSCode),
            "trae" => Some(Self::Trae),
            "zed" => Some(Self::Zed),
            "opencode" => Some(Self::OpenCode),
            "roocode" | "roo" => Some(Self::RooCode),
            "cline" => Some(Self::Cline),
            "continue" | "continuedev" => Some(Self::Continue),
            "kiro" | "kiroide" => Some(Self::Kiro),
            "qwen" | "qwencode" => Some(Self::Qwen),
            "kilocode" | "kilo" => Some(Self::KiloCode),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Desktop",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "OpenAI Codex",
            Self::Cursor => "Cursor",
            Self::Antigravity => "Google Antigravity",
            Self::Gemini => "Gemini CLI",
            Self::Windsurf => "Windsurf",
            Self::VSCode => "VS Code",
            Self::Trae => "Trae",
            Self::Zed => "Zed",
            Self::OpenCode => "OpenCode",
            Self::RooCode => "Roo Code",
            Self::Cline => "Cline",
            Self::Continue => "Continue.dev",
            Self::Kiro => "Kiro IDE",
            Self::Qwen => "Qwen Code",
            Self::KiloCode => "Kilo Code",
        }
    }

    pub fn cli_id(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
            Self::Gemini => "gemini",
            Self::Windsurf => "windsurf",
            Self::VSCode => "vscode",
            Self::Trae => "trae",
            Self::Zed => "zed",
            Self::OpenCode => "opencode",
            Self::RooCode => "roocode",
            Self::Cline => "cline",
            Self::Continue => "continue",
            Self::Kiro => "kiro",
            Self::Qwen => "qwen",
            Self::KiloCode => "kilocode",
        }
    }

    pub fn config_path_with_home(&self, home_path: &Path) -> Option<PathBuf> {
        match self {
            Self::Claude => {
                #[cfg(target_os = "macos")]
                {
                    Some(
                        home_path
                            .join("Library/Application Support/Claude/claude_desktop_config.json"),
                    )
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Claude/claude_desktop_config.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Claude/claude_desktop_config.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/Claude/claude_desktop_config.json"))
                }
            }
            Self::ClaudeCode => Some(home_path.join(".claude.json")),
            Self::Codex => Some(home_path.join(".codex/config.toml")),
            Self::Cursor => Some(home_path.join(".cursor/mcp.json")),
            Self::Antigravity => {
                let gemini_cfg = home_path.join(".gemini/config/mcp_config.json");
                let gemini_path = home_path.join(".gemini/antigravity-cli/mcp_config.json");
                if gemini_cfg.exists() || gemini_cfg.parent().map(|p| p.exists()).unwrap_or(false) {
                    Some(gemini_cfg)
                } else if gemini_path.parent().map(|p| p.exists()).unwrap_or(false) {
                    Some(gemini_path)
                } else {
                    Some(home_path.join(".config/antigravity/mcp_config.json"))
                }
            }
            Self::Gemini => Some(home_path.join(".gemini/settings.json")),
            Self::Windsurf => {
                let codeium_mcp = home_path.join(".codeium/windsurf/mcp_config.json");
                if codeium_mcp.exists() || home_path.join(".codeium/windsurf").exists() {
                    Some(codeium_mcp)
                } else {
                    #[cfg(target_os = "macos")]
                    {
                        let mac_storage = home_path.join("Library/Application Support/Windsurf/User/globalStorage/codeium.windsurf/windsurf_mcp_config.json");
                        if mac_storage.parent().map(|p| p.exists()).unwrap_or(false) {
                            Some(mac_storage)
                        } else {
                            Some(codeium_mcp)
                        }
                    }
                    #[cfg(target_os = "windows")]
                    {
                        if let Ok(appdata) = env::var("APPDATA") {
                            Some(PathBuf::from(appdata).join("Windsurf/User/globalStorage/codeium.windsurf/windsurf_mcp_config.json"))
                        } else {
                            Some(codeium_mcp)
                        }
                    }
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    {
                        Some(codeium_mcp)
                    }
                }
            }
            Self::VSCode => {
                #[cfg(target_os = "macos")]
                {
                    Some(home_path.join("Library/Application Support/Code/User/mcp.json"))
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Code/User/mcp.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Code/User/mcp.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/Code/User/mcp.json"))
                }
            }
            Self::Trae => {
                #[cfg(target_os = "macos")]
                {
                    Some(home_path.join("Library/Application Support/Trae/User/mcp.json"))
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Trae/User/mcp.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Trae/User/mcp.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/Trae/User/mcp.json"))
                }
            }
            Self::Zed => {
                #[cfg(target_os = "macos")]
                {
                    let cfg_zed = home_path.join(".config/zed/settings.json");
                    let app_support =
                        home_path.join("Library/Application Support/Zed/settings.json");
                    if cfg_zed.exists() {
                        Some(cfg_zed)
                    } else if app_support.exists() {
                        Some(app_support)
                    } else if home_path.join(".config/zed").exists() {
                        Some(cfg_zed)
                    } else if home_path.join("Library/Application Support/Zed").exists() {
                        Some(app_support)
                    } else {
                        Some(cfg_zed)
                    }
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Zed/settings.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Zed/settings.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/zed/settings.json"))
                }
            }
            Self::OpenCode => {
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("opencode/opencode.json"))
                    } else {
                        Some(home_path.join(".config/opencode/opencode.json"))
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    Some(home_path.join(".config/opencode/opencode.json"))
                }
            }
            Self::RooCode => {
                #[cfg(target_os = "macos")]
                {
                    Some(home_path.join("Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/cline_mcp_settings.json"))
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/cline_mcp_settings.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/cline_mcp_settings.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/cline_mcp_settings.json"))
                }
            }
            Self::Cline => {
                #[cfg(target_os = "macos")]
                {
                    Some(home_path.join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"))
                }
                #[cfg(target_os = "windows")]
                {
                    if let Ok(appdata) = env::var("APPDATA") {
                        Some(PathBuf::from(appdata).join("Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"))
                    } else {
                        Some(home_path.join("AppData/Roaming/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"))
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Some(home_path.join(".config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"))
                }
            }
            Self::Continue => Some(home_path.join(".continue/config.json")),
            Self::Kiro => Some(home_path.join(".kiro/settings/mcp.json")),
            Self::Qwen => Some(home_path.join(".qwen/settings.json")),
            Self::KiloCode => Some(home_path.join(".config/kilo/opencode.json")),
        }
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok()?;
        self.config_path_with_home(&PathBuf::from(home))
    }

    pub fn is_installed(&self, home_path: &Path) -> bool {
        if let Some(cfg) = self.config_path_with_home(home_path)
            && cfg.exists()
        {
            return true;
        }

        match self {
            Self::Claude => {
                home_path.join("Library/Application Support/Claude").exists()
                    || home_path.join(".config/Claude").exists()
                    || Path::new("/Applications/Claude.app").exists()
            }
            Self::ClaudeCode => {
                home_path.join(".claude").exists() || home_path.join(".claude.json").exists()
            }
            Self::Codex => {
                home_path.join(".codex").exists()
                    || Path::new("/Applications/Codex.app").exists()
                    || Path::new("/Applications/ChatGPT.app").exists()
            }
            Self::Cursor => {
                home_path.join(".cursor").exists()
                    || Path::new("/Applications/Cursor.app").exists()
            }
            Self::Antigravity => {
                home_path.join(".gemini/antigravity-cli").exists()
                    || home_path.join(".config/antigravity").exists()
                    || home_path.join(".gemini").exists()
                    || Path::new("/Applications/Antigravity.app").exists()
                    || Path::new("/Applications/Antigravity IDE.app").exists()
            }
            Self::Gemini => home_path.join(".gemini").exists(),
            Self::Windsurf => {
                home_path.join(".codeium/windsurf").exists()
                    || home_path.join(".windsurf").exists()
                    || home_path.join("Library/Application Support/Windsurf").exists()
                    || Path::new("/Applications/Windsurf.app").exists()
            }
            Self::VSCode => {
                home_path.join("Library/Application Support/Code").exists()
                    || home_path.join(".config/Code").exists()
                    || Path::new("/Applications/Visual Studio Code.app").exists()
            }
            Self::Trae => {
                home_path.join("Library/Application Support/Trae").exists()
                    || home_path.join(".trae").exists()
                    || Path::new("/Applications/Trae.app").exists()
            }
            Self::Zed => {
                home_path.join(".config/zed").exists()
                    || home_path.join("Library/Application Support/Zed").exists()
                    || Path::new("/Applications/Zed.app").exists()
            }
            Self::OpenCode => {
                home_path.join(".config/opencode").exists()
                    || home_path.join("Library/Application Support/opencode").exists()
            }
            Self::RooCode => {
                home_path
                    .join("Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline")
                    .exists()
                    || home_path
                        .join(".config/Code/User/globalStorage/rooveterinaryinc.roo-cline")
                        .exists()
            }
            Self::Cline => {
                home_path
                    .join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev")
                    .exists()
                    || home_path
                        .join(".config/Code/User/globalStorage/saoudrizwan.claude-dev")
                        .exists()
            }
            Self::Continue => home_path.join(".continue").exists(),
            Self::Kiro => {
                home_path.join(".kiro").exists() || Path::new("/Applications/Kiro.app").exists()
            }
            Self::Qwen => home_path.join(".qwen").exists(),
            Self::KiloCode => home_path.join(".config/kilo").exists(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub client: TargetClient,
    pub path: PathBuf,
    pub already_configured: bool,
}

pub fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            result.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            result.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for nc in chars.by_ref() {
                if nc == '\n' {
                    result.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(bc) = chars.next() {
                if bc == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Detect best command string for agent-mem executable
fn resolve_binary_command() -> String {
    if let Ok(exe_path) = env::current_exe()
        && exe_path.exists()
        && let Ok(canonical) = exe_path.canonicalize()
    {
        // Check if agent-mem is in standard PATH
        if let Ok(path_var) = env::var("PATH") {
            for dir in env::split_paths(&path_var) {
                if dir.join("agent-mem").exists() || dir.join("agent-mem.exe").exists() {
                    return "agent-mem".to_string();
                }
            }
        }
        return canonical.to_string_lossy().to_string();
    }

    "agent-mem".to_string()
}

pub fn install_to_path(
    config_path: &std::path::Path,
    client: TargetClient,
) -> Result<InstallResult> {
    let binary_cmd = resolve_binary_command();

    if client == TargetClient::Codex {
        let (already_configured, updated_content) = if config_path.exists() {
            let raw = fs::read_to_string(config_path)?;
            let has_header = raw.contains("[mcp_servers.agent-mem]")
                || raw.contains("[mcp_servers.\"agent-mem\"]");
            if has_header {
                (true, raw)
            } else {
                let mut c = raw;
                if !c.is_empty() && !c.ends_with('\n') {
                    c.push('\n');
                }
                c.push_str(&format!(
                    "\n[mcp_servers.agent-mem]\ncommand = \"{}\"\nargs = [\"mcp\"]\n",
                    binary_cmd
                ));
                (false, c)
            }
        } else {
            let c = format!(
                "[mcp_servers.agent-mem]\ncommand = \"{}\"\nargs = [\"mcp\"]\n",
                binary_cmd
            );
            (false, c)
        };

        if !already_configured || !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(config_path, updated_content)?;
        }

        return Ok(InstallResult {
            client,
            path: config_path.to_path_buf(),
            already_configured,
        });
    }

    let mut root_json: Value = if config_path.exists() {
        let raw = fs::read_to_string(config_path)?;
        let clean = strip_json_comments(&raw);
        serde_json::from_str(&clean).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root_json.is_object() {
        root_json = json!({});
    }

    let already_configured = match client {
        TargetClient::Codex => unreachable!(),
        TargetClient::VSCode => {
            let servers = root_json
                .as_object_mut()
                .unwrap()
                .entry("servers")
                .or_insert_with(|| json!({}));
            if !servers.is_object() {
                *servers = json!({});
            }
            let servers_obj = servers.as_object_mut().unwrap();
            let new_entry = json!({
                "command": binary_cmd,
                "args": ["mcp"],
                "type": "stdio"
            });
            let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
            servers_obj.insert("agent-mem".to_string(), new_entry);
            is_configured
        }
        TargetClient::Zed => {
            let servers = root_json
                .as_object_mut()
                .unwrap()
                .entry("context_servers")
                .or_insert_with(|| json!({}));
            if !servers.is_object() {
                *servers = json!({});
            }
            let servers_obj = servers.as_object_mut().unwrap();
            let new_entry = json!({
                "command": binary_cmd,
                "args": ["mcp"]
            });
            let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
            servers_obj.insert("agent-mem".to_string(), new_entry);
            is_configured
        }
        TargetClient::OpenCode | TargetClient::KiloCode => {
            let servers = root_json
                .as_object_mut()
                .unwrap()
                .entry("mcp")
                .or_insert_with(|| json!({}));
            if !servers.is_object() {
                *servers = json!({});
            }
            let servers_obj = servers.as_object_mut().unwrap();
            let new_entry = json!({
                "type": "local",
                "command": [binary_cmd, "mcp"]
            });
            let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
            servers_obj.insert("agent-mem".to_string(), new_entry);
            is_configured
        }
        TargetClient::ClaudeCode => {
            let servers = root_json
                .as_object_mut()
                .unwrap()
                .entry("mcpServers")
                .or_insert_with(|| json!({}));
            if !servers.is_object() {
                *servers = json!({});
            }
            let servers_obj = servers.as_object_mut().unwrap();
            let new_entry = json!({
                "type": "stdio",
                "command": binary_cmd,
                "args": ["mcp"]
            });
            let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
            servers_obj.insert("agent-mem".to_string(), new_entry);
            is_configured
        }
        TargetClient::Continue => {
            let servers_val = root_json
                .as_object_mut()
                .unwrap()
                .entry("mcpServers")
                .or_insert_with(|| json!([]));
            if let Some(arr) = servers_val.as_array_mut() {
                let mut found_idx = None;
                for (i, item) in arr.iter().enumerate() {
                    if item.get("name").and_then(|n| n.as_str()) == Some("agent-mem") {
                        found_idx = Some(i);
                        break;
                    }
                }
                let new_entry = json!({
                    "name": "agent-mem",
                    "command": binary_cmd,
                    "args": ["mcp"]
                });
                if let Some(i) = found_idx {
                    let is_configured = arr[i] == new_entry;
                    arr[i] = new_entry;
                    is_configured
                } else {
                    arr.push(new_entry);
                    false
                }
            } else if let Some(servers_obj) = servers_val.as_object_mut() {
                let new_entry = json!({
                    "command": binary_cmd,
                    "args": ["mcp"]
                });
                let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
                servers_obj.insert("agent-mem".to_string(), new_entry);
                is_configured
            } else {
                *servers_val = json!([{
                    "name": "agent-mem",
                    "command": binary_cmd,
                    "args": ["mcp"]
                }]);
                false
            }
        }
        TargetClient::Claude
        | TargetClient::Cursor
        | TargetClient::Antigravity
        | TargetClient::Gemini
        | TargetClient::Windsurf
        | TargetClient::Trae
        | TargetClient::RooCode
        | TargetClient::Cline
        | TargetClient::Kiro
        | TargetClient::Qwen => {
            let servers = root_json
                .as_object_mut()
                .unwrap()
                .entry("mcpServers")
                .or_insert_with(|| json!({}));
            if !servers.is_object() {
                *servers = json!({});
            }
            let servers_obj = servers.as_object_mut().unwrap();
            let new_entry = json!({
                "command": binary_cmd,
                "args": ["mcp"]
            });
            let is_configured = servers_obj.get("agent-mem") == Some(&new_entry);
            servers_obj.insert("agent-mem".to_string(), new_entry);
            is_configured
        }
    };

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let formatted = serde_json::to_string_pretty(&root_json)
        .map_err(|e| Error::Usage(format!("Failed to format JSON: {}", e)))?;

    fs::write(config_path, format!("{}\n", formatted))?;

    Ok(InstallResult {
        client,
        path: config_path.to_path_buf(),
        already_configured,
    })
}

pub fn install_client(client: TargetClient) -> Result<InstallResult> {
    let config_path = client.config_path().ok_or_else(|| {
        Error::Usage(format!(
            "Could not determine config path for {}",
            client.display_name()
        ))
    })?;

    install_to_path(&config_path, client)
}

pub fn install_all_or_target(target: Option<&str>) -> Result<Vec<InstallResult>> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| Error::Usage("Could not determine user HOME directory".to_string()))?;

    match target {
        Some("all") | None => {
            let mut results = Vec::new();
            let mut detected_any = false;

            for &client in ALL_CLIENTS {
                let status = check_client_status_with_home(client, &home);
                if status.installed || status.configured {
                    detected_any = true;
                    results.push(install_client(client)?);
                }
            }

            if !detected_any && target.is_none() {
                // If neither was detected, install into Claude by default
                results.push(install_client(TargetClient::Claude)?);
            }

            Ok(results)
        }
        Some(name) => {
            let client = TargetClient::parse(name).ok_or_else(|| {
                Error::Usage(format!(
                    "Unknown client '{}'. Supported clients: claude, claude-code, codex, cursor, antigravity, gemini, windsurf, vscode, trae, zed, opencode, roocode, cline, continue, kiro, qwen, kilocode, all",
                    name
                ))
            })?;
            Ok(vec![install_client(client)?])
        }
    }
}

pub fn install_detected_clients() -> Result<Vec<InstallResult>> {
    if env::var("AGENT_MEM_NO_AUTO_INSTALL").is_ok() {
        return Ok(Vec::new());
    }
    if env::var("AGENT_MEM_TEST_HOME").is_err()
        && (env::var("CARGO_TARGET_TMPDIR").is_ok() || cfg!(test))
    {
        return Ok(Vec::new());
    }

    let home = env::var("AGENT_MEM_TEST_HOME")
        .or_else(|_| env::var("HOME"))
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| Error::Usage("Could not determine user HOME directory".to_string()))?;

    install_detected_clients_with_home(&home)
}

pub fn install_detected_clients_with_home(home: &Path) -> Result<Vec<InstallResult>> {
    let mut results = Vec::new();
    for &client in ALL_CLIENTS {
        let status = check_client_status_with_home(client, home);
        if (status.installed || status.configured)
            && let Some(config_path) = status.path
            && let Ok(res) = install_to_path(&config_path, client)
        {
            results.push(res);
        }
    }
    Ok(results)
}

pub struct ClientStatus {
    pub client: TargetClient,
    pub path: Option<PathBuf>,
    pub installed: bool,
    pub configured: bool,
}

pub fn check_client_status_with_home(client: TargetClient, home: &Path) -> ClientStatus {
    let path = client.config_path_with_home(home);
    let mut installed = client.is_installed(home);
    let configured = if let Some(ref p) = path
        && p.exists()
    {
        if client == TargetClient::Codex {
            if let Ok(raw) = fs::read_to_string(p) {
                raw.contains("[mcp_servers.agent-mem]")
                    || raw.contains("[mcp_servers.\"agent-mem\"]")
            } else {
                false
            }
        } else if let Ok(raw) = fs::read_to_string(p) {
            let clean = strip_json_comments(&raw);
            if let Ok(json) = serde_json::from_str::<Value>(&clean) {
                match client {
                    TargetClient::Codex => unreachable!(),
                    TargetClient::VSCode => {
                        json.get("servers")
                            .and_then(|m| m.get("agent-mem"))
                            .is_some()
                            || json
                                .get("mcpServers")
                                .and_then(|m| m.get("agent-mem"))
                                .is_some()
                    }
                    TargetClient::Zed => json
                        .get("context_servers")
                        .and_then(|m| m.get("agent-mem"))
                        .is_some(),
                    TargetClient::OpenCode | TargetClient::KiloCode => {
                        json.get("mcp").and_then(|m| m.get("agent-mem")).is_some()
                    }
                    TargetClient::Continue => {
                        if let Some(arr) = json.get("mcpServers").and_then(|m| m.as_array()) {
                            arr.iter().any(|i| {
                                i.get("name").and_then(|n| n.as_str()) == Some("agent-mem")
                            })
                        } else if let Some(obj) = json.get("mcpServers").and_then(|m| m.as_object())
                        {
                            obj.get("agent-mem").is_some()
                        } else {
                            false
                        }
                    }
                    TargetClient::Claude
                    | TargetClient::ClaudeCode
                    | TargetClient::Cursor
                    | TargetClient::Antigravity
                    | TargetClient::Gemini
                    | TargetClient::Windsurf
                    | TargetClient::Trae
                    | TargetClient::RooCode
                    | TargetClient::Cline
                    | TargetClient::Kiro
                    | TargetClient::Qwen => {
                        json.get("mcpServers")
                            .and_then(|m| m.get("agent-mem"))
                            .is_some()
                            || json
                                .get("servers")
                                .and_then(|m| m.get("agent-mem"))
                                .is_some()
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if configured {
        installed = true;
    }

    ClientStatus {
        client,
        path,
        installed,
        configured,
    }
}

pub fn check_client_status(client: TargetClient) -> ClientStatus {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    check_client_status_with_home(client, Path::new(&home))
}

pub fn check_all_clients() -> Vec<ClientStatus> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let home_path = Path::new(&home);
    ALL_CLIENTS
        .iter()
        .map(|&c| check_client_status_with_home(c, home_path))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallResult {
    pub client: TargetClient,
    pub path: PathBuf,
    pub removed: bool,
}

pub fn uninstall_from_path(config_path: &Path, client: TargetClient) -> Result<UninstallResult> {
    if !config_path.exists() {
        return Ok(UninstallResult {
            client,
            path: config_path.to_path_buf(),
            removed: false,
        });
    }

    if client == TargetClient::Codex {
        let raw = fs::read_to_string(config_path)?;
        let mut lines = Vec::new();
        let mut in_agent_mem_section = false;
        let mut removed = false;

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed == "[mcp_servers.agent-mem]" || trimmed == "[mcp_servers.\"agent-mem\"]" {
                in_agent_mem_section = true;
                removed = true;
                continue;
            } else if in_agent_mem_section && trimmed.starts_with('[') {
                in_agent_mem_section = false;
            }

            if !in_agent_mem_section {
                lines.push(line);
            }
        }

        if removed {
            let mut new_content = lines.join("\n");
            if !new_content.is_empty() && !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            fs::write(config_path, new_content)?;
        }

        return Ok(UninstallResult {
            client,
            path: config_path.to_path_buf(),
            removed,
        });
    }

    let raw = fs::read_to_string(config_path)?;
    let clean = strip_json_comments(&raw);
    let mut root_json: Value = match serde_json::from_str(&clean) {
        Ok(v) => v,
        Err(_) => {
            return Ok(UninstallResult {
                client,
                path: config_path.to_path_buf(),
                removed: false,
            });
        }
    };

    let mut removed = false;

    if let Some(obj) = root_json.as_object_mut() {
        // 1. VS Code: "servers"
        if let Some(servers) = obj.get_mut("servers").and_then(|v| v.as_object_mut())
            && servers.remove("agent-mem").is_some()
        {
            removed = true;
        }

        // 2. Zed: "context_servers"
        if let Some(servers) = obj
            .get_mut("context_servers")
            .and_then(|v| v.as_object_mut())
            && servers.remove("agent-mem").is_some()
        {
            removed = true;
        }

        // 3. OpenCode / KiloCode: "mcp"
        if let Some(servers) = obj.get_mut("mcp").and_then(|v| v.as_object_mut())
            && servers.remove("agent-mem").is_some()
        {
            removed = true;
        }

        // 4. Standard: "mcpServers" (object or array)
        if let Some(mcp_servers) = obj.get_mut("mcpServers") {
            if let Some(servers_obj) = mcp_servers.as_object_mut() {
                if servers_obj.remove("agent-mem").is_some() {
                    removed = true;
                }
            } else if let Some(arr) = mcp_servers.as_array_mut() {
                let orig_len = arr.len();
                arr.retain(|item| item.get("name").and_then(|n| n.as_str()) != Some("agent-mem"));
                if arr.len() < orig_len {
                    removed = true;
                }
            }
        }
    }

    if removed {
        let formatted =
            serde_json::to_string_pretty(&root_json).map_err(|e| Error::Usage(e.to_string()))?;
        fs::write(config_path, formatted)?;
    }

    Ok(UninstallResult {
        client,
        path: config_path.to_path_buf(),
        removed,
    })
}

pub fn uninstall_client(client: TargetClient) -> Result<UninstallResult> {
    let home = env::var("AGENT_MEM_TEST_HOME")
        .or_else(|_| env::var("HOME"))
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| Error::Usage("Could not determine user HOME directory".to_string()))?;

    let path = client
        .config_path_with_home(&home)
        .ok_or_else(|| Error::Usage(format!("No config path for {}", client.display_name())))?;

    uninstall_from_path(&path, client)
}

pub fn uninstall_clients(target: Option<&str>) -> Result<Vec<UninstallResult>> {
    let home = env::var("AGENT_MEM_TEST_HOME")
        .or_else(|_| env::var("HOME"))
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| Error::Usage("Could not determine user HOME directory".to_string()))?;

    match target {
        Some("all") | None => {
            let mut results = Vec::new();
            for &client in ALL_CLIENTS {
                let status = check_client_status_with_home(client, &home);
                if status.configured
                    && let Some(config_path) = status.path
                {
                    results.push(uninstall_from_path(&config_path, client)?);
                }
            }
            Ok(results)
        }
        Some(name) => {
            let client = TargetClient::parse(name).ok_or_else(|| {
                Error::Usage(format!(
                    "Unknown client '{}'. Supported clients: claude, claude-code, codex, cursor, antigravity, gemini, windsurf, vscode, trae, zed, opencode, roocode, cline, continue, kiro, qwen, kilocode, all",
                    name
                ))
            })?;
            Ok(vec![uninstall_client(client)?])
        }
    }
}
