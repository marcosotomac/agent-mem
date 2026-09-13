use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClient {
    Claude,
    Cursor,
    Antigravity,
}

impl TargetClient {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-desktop" => Some(Self::Claude),
            "cursor" => Some(Self::Cursor),
            "antigravity" | "gemini" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::Antigravity => "Google Antigravity",
        }
    }

    pub fn config_path_with_home(&self, home_path: &std::path::Path) -> Option<PathBuf> {
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
        }
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok()?;
        self.config_path_with_home(&PathBuf::from(home))
    }
}

pub struct InstallResult {
    pub client: TargetClient,
    pub path: PathBuf,
    pub already_configured: bool,
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
        let content = fs::read_to_string(config_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root_json.is_object() {
        root_json = json!({});
    }

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

    let already_configured = servers_obj.get("agent-mem") == Some(&new_entry);
    servers_obj.insert("agent-mem".to_string(), new_entry);

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
    match target {
        Some("all") | None => {
            let clients = [
                TargetClient::Claude,
                TargetClient::Cursor,
                TargetClient::Antigravity,
            ];
            let mut results = Vec::new();
            let mut detected_any = false;

            for client in clients {
                if let Some(path) = client.config_path() {
                    let dir_exists = path.parent().map(|p| p.exists()).unwrap_or(false);
                    if path.exists() || dir_exists {
                        detected_any = true;
                        results.push(install_client(client)?);
                    }
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
                    "Unknown client '{}'. Supported clients: claude, cursor, antigravity, all",
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
    pub configured: bool,
}

pub fn check_client_status(client: TargetClient) -> ClientStatus {
    let path = client.config_path();
    let configured = if let Some(ref p) = path {
        if p.exists() {
            if let Ok(content) = fs::read_to_string(p) {
                if let Ok(json) = serde_json::from_str::<Value>(&content) {
                    json.get("mcpServers")
                        .and_then(|m| m.get("agent-mem"))
                        .is_some()
                } else {
                    false
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
    ClientStatus {
        client,
        path,
        configured,
    }
}

pub fn check_all_clients() -> Vec<ClientStatus> {
    vec![
        check_client_status(TargetClient::Antigravity),
        check_client_status(TargetClient::Cursor),
        check_client_status(TargetClient::Claude),
    ]
}
