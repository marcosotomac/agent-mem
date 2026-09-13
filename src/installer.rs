use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClient {
    Claude,
    Cursor,
    Antigravity,
    Windsurf,
    VSCode,
    Zed,
    OpenCode,
    RooCode,
    Cline,
}

pub const ALL_CLIENTS: &[TargetClient] = &[
    TargetClient::Antigravity,
    TargetClient::Cursor,
    TargetClient::Claude,
    TargetClient::Windsurf,
    TargetClient::VSCode,
    TargetClient::Zed,
    TargetClient::OpenCode,
    TargetClient::RooCode,
    TargetClient::Cline,
];

impl TargetClient {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_', ' '], "").as_str() {
            "claude" | "claudedesktop" => Some(Self::Claude),
            "cursor" => Some(Self::Cursor),
            "antigravity" | "gemini" => Some(Self::Antigravity),
            "windsurf" | "codeium" => Some(Self::Windsurf),
            "vscode" | "code" => Some(Self::VSCode),
            "zed" => Some(Self::Zed),
            "opencode" => Some(Self::OpenCode),
            "roocode" | "roo" => Some(Self::RooCode),
            "cline" => Some(Self::Cline),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::Antigravity => "Google Antigravity",
            Self::Windsurf => "Windsurf",
            Self::VSCode => "VS Code",
            Self::Zed => "Zed",
            Self::OpenCode => "OpenCode",
            Self::RooCode => "Roo Code",
            Self::Cline => "Cline",
        }
    }

    pub fn cli_id(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
            Self::Windsurf => "windsurf",
            Self::VSCode => "vscode",
            Self::Zed => "zed",
            Self::OpenCode => "opencode",
            Self::RooCode => "roocode",
            Self::Cline => "cline",
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
            Self::Cursor => Some(home_path.join(".cursor/mcp.json")),
            Self::Antigravity => {
                let gemini_path = home_path.join(".gemini/antigravity-cli/mcp_config.json");
                if gemini_path.parent().map(|p| p.exists()).unwrap_or(false) {
                    Some(gemini_path)
                } else {
                    Some(home_path.join(".config/antigravity/mcp_config.json"))
                }
            }
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
            Self::Cursor => {
                home_path.join(".cursor").exists()
                    || Path::new("/Applications/Cursor.app").exists()
            }
            Self::Antigravity => {
                home_path.join(".gemini/antigravity-cli").exists()
                    || home_path.join(".config/antigravity").exists()
                    || Path::new("/Applications/Antigravity.app").exists()
                    || Path::new("/Applications/Antigravity IDE.app").exists()
            }
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
        }
    }
}

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
        TargetClient::OpenCode => {
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
        TargetClient::Claude
        | TargetClient::Cursor
        | TargetClient::Antigravity
        | TargetClient::Windsurf
        | TargetClient::RooCode
        | TargetClient::Cline => {
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
                    "Unknown client '{}'. Supported clients: claude, cursor, antigravity, windsurf, vscode, zed, opencode, roocode, cline, all",
                    name
                ))
            })?;
            Ok(vec![install_client(client)?])
        }
    }
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
        if let Ok(raw) = fs::read_to_string(p) {
            let clean = strip_json_comments(&raw);
            if let Ok(json) = serde_json::from_str::<Value>(&clean) {
                match client {
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
                    TargetClient::OpenCode => {
                        json.get("mcp").and_then(|m| m.get("agent-mem")).is_some()
                    }
                    TargetClient::Claude
                    | TargetClient::Cursor
                    | TargetClient::Antigravity
                    | TargetClient::Windsurf
                    | TargetClient::RooCode
                    | TargetClient::Cline => {
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
