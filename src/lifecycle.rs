//! Session lifecycle hooks: automatic context injection at agent session start.
//!
//! Editors that support lifecycle hooks (ZCode, Claude Code) run
//! `agent-mem hook session-start` on SessionStart and inject its output into
//! the conversation, removing the dependency on agents obeying an AGENTS.md
//! protocol. This module owns editor configuration: registration is
//! idempotent, scoped to our own hook entries, and fully reversible.

use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Hook entries we own are identified by this subcommand marker and status prefix.
const OWNED_SUBCOMMAND: &str = "hook session-start";
const OWNED_STATUS_PREFIX: &str = "agent-mem:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleClient {
    ZCode,
    ClaudeCode,
}

impl LifecycleClient {
    pub const ALL: &'static [LifecycleClient] =
        &[LifecycleClient::ZCode, LifecycleClient::ClaudeCode];

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_', ' '], "").as_str() {
            "zcode" => Some(Self::ZCode),
            "claudecode" | "claude" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ZCode => "zcode",
            Self::ClaudeCode => "claude-code",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ZCode => "ZCode",
            Self::ClaudeCode => "Claude Code",
        }
    }

    fn config_path(&self, home: &std::path::Path) -> PathBuf {
        match self {
            Self::ZCode => home.join(".zcode").join("cli").join("config.json"),
            Self::ClaudeCode => home.join(".claude").join("settings.json"),
        }
    }

    /// ZCode wraps events under `hooks.events.<Event>` and requires
    /// `hooks.enabled: true`; Claude Code uses `hooks.<Event>` directly.
    fn hook_config<'a>(
        &self,
        client_config: &'a mut Value,
        install: bool,
    ) -> Result<&'a mut Vec<Value>> {
        let obj = client_config
            .as_object_mut()
            .ok_or_else(|| Error::Usage("client configuration is not a JSON object".into()))?;
        let hooks = obj
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| Error::Usage("'hooks' is not a JSON object".into()))?;

        if install && self == &Self::ZCode {
            hooks.insert("enabled".into(), json!(true));
        }
        let container = match self {
            Self::ZCode => hooks
                .entry("events")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| Error::Usage("'hooks.events' is not a JSON object".into()))?
                .entry("SessionStart")
                .or_insert_with(|| json!([])),
            Self::ClaudeCode => hooks.entry("SessionStart").or_insert_with(|| json!([])),
        };
        container
            .as_array_mut()
            .ok_or_else(|| Error::Usage("'SessionStart' is not an array".into()))
    }

    fn existing_hook_config<'a>(&self, client_config: &'a mut Value) -> Option<&'a mut Vec<Value>> {
        let hooks = client_config.get_mut("hooks")?.as_object_mut()?;
        let container = match self {
            Self::ZCode => hooks
                .get_mut("events")?
                .as_object_mut()?
                .get_mut("SessionStart")?,
            Self::ClaudeCode => hooks.get_mut("SessionStart")?,
        };
        container.as_array_mut()
    }

    /// Drop now-empty containers we created, preserving anything pre-existing.
    fn cleanup_empty(&self, client_config: &mut Value) {
        let Some(hooks) = client_config
            .get_mut("hooks")
            .and_then(|h| h.as_object_mut())
        else {
            return;
        };
        let events_key = match self {
            Self::ZCode => Some("events"),
            Self::ClaudeCode => None,
        };
        if let Some(events_key) = events_key
            && let Some(events) = hooks.get_mut(events_key).and_then(|e| e.as_object_mut())
        {
            events.retain(|_, v| !v.as_array().is_some_and(|a| a.is_empty()));
            if events.is_empty() {
                hooks.remove(events_key);
            }
        }
        if self == &Self::ClaudeCode {
            hooks.retain(|_, v| !v.as_array().is_some_and(|a| a.is_empty()));
        }
        // Never remove `hooks` itself or ZCode's `enabled` flag: both may be
        // owned by the user or by other hooks.
    }

    fn restore_enabled_state(&self, path: &std::path::Path, client_config: &mut Value) {
        if self != &Self::ZCode {
            return;
        }
        let has_other_events = client_config
            .pointer("/hooks/events")
            .and_then(Value::as_object)
            .is_some_and(|events| !events.is_empty());
        if has_other_events {
            return;
        }
        let previous = load_config(&crate::installer::backup_path(path))
            .ok()
            .and_then(|value| value.pointer("/hooks/enabled").cloned());
        let Some(hooks) = client_config
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
        else {
            return;
        };
        match previous {
            Some(value @ Value::Bool(_)) => {
                hooks.insert("enabled".into(), value);
            }
            _ => {
                hooks.remove("enabled");
            }
        }
    }
}

#[derive(Debug)]
pub struct LifecycleResult {
    pub client: LifecycleClient,
    pub path: PathBuf,
    pub configured: bool,
}

fn test_home() -> Result<PathBuf> {
    env::var("AGENT_MEM_TEST_HOME")
        .or_else(|_| env::var("HOME"))
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| Error::Usage("Could not determine user HOME directory".to_string()))
}

/// The command string editors will run. `resolve_binary_command` mirrors the
/// MCP installer so both features launch the same binary.
fn hook_command(client: LifecycleClient) -> String {
    let binary = crate::installer::resolve_binary_command();
    let binary = shell_quote(&binary);
    match client {
        // ZCode parses hook output as strict JSON with additionalContext.
        LifecycleClient::ZCode => format!("{binary} hook session-start --format json"),
        // Claude Code injects SessionStart stdout verbatim.
        LifecycleClient::ClaudeCode => format!("{binary} hook session-start"),
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn load_config(path: &std::path::Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&raw)
        .map_err(|e| Error::Usage(format!("invalid JSON in {}: {e}", path.display())))
}

fn save_config(path: &std::path::Path, config: &Value) -> Result<()> {
    let formatted = serde_json::to_string_pretty(config)
        .map_err(|e| Error::Usage(format!("failed to serialize configuration: {e}")))?;
    crate::installer::write_config_safely(path, &format!("{formatted}\n"))
}

fn is_owned_hook(hook: &Value) -> bool {
    let status_owned = hook
        .get("statusMessage")
        .and_then(|s| s.as_str())
        .is_some_and(|s| s.starts_with(OWNED_STATUS_PREFIX));
    let command_owned = hook
        .get("command")
        .and_then(|c| c.as_str())
        .is_some_and(|c| {
            c.contains(OWNED_SUBCOMMAND) && (c.contains("agent-mem") || c.contains("agent_mem"))
        });
    status_owned || command_owned
}

fn owned_entries(entries: &[Value]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|inner| inner.iter().any(is_owned_hook))
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn install_session_hooks(target: Option<&str>) -> Result<Vec<LifecycleResult>> {
    let home = test_home()?;
    let clients: Vec<LifecycleClient> = match target {
        None => LifecycleClient::ALL
            .iter()
            .copied()
            .filter(|c| c.config_path(&home).exists())
            .collect(),
        Some(name) => {
            let client = LifecycleClient::parse(name).ok_or_else(|| {
                Error::Usage(format!(
                    "Unknown client '{}'. Supported clients: zcode, claude-code",
                    name
                ))
            })?;
            vec![client]
        }
    };

    let mut results = Vec::new();
    for client in clients {
        let path = client.config_path(&home);
        let mut config = load_config(&path)?;
        let command = hook_command(client);

        let entries = client.hook_config(&mut config, true)?;
        let existing = owned_entries(entries);
        let already = existing.len() == 1
            && entries[existing[0]]
                .get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|inner| {
                    inner.len() == 1
                        && inner[0].get("command").and_then(|c| c.as_str()) == Some(&command)
                });

        for index in existing.iter().rev() {
            entries.remove(*index);
        }
        entries.push(json!({
            "hooks": [
                {
                    "type": "command",
                    "command": command,
                    "timeout": 10,
                    "statusMessage": "agent-mem: loading project memory"
                }
            ]
        }));
        if !already {
            save_config(&path, &config)?;
        }
        results.push(LifecycleResult {
            client,
            path,
            configured: already,
        });
    }
    Ok(results)
}

pub fn uninstall_session_hooks(target: Option<&str>) -> Result<Vec<LifecycleResult>> {
    let home = test_home()?;
    let clients: Vec<LifecycleClient> = match target {
        None | Some("all") => LifecycleClient::ALL.to_vec(),
        Some(name) => {
            let client = LifecycleClient::parse(name).ok_or_else(|| {
                Error::Usage(format!(
                    "Unknown client '{}'. Supported clients: zcode, claude-code, all",
                    name
                ))
            })?;
            vec![client]
        }
    };

    let mut results = Vec::new();
    for client in clients {
        let path = client.config_path(&home);
        if !path.exists() {
            continue;
        }
        let mut config = load_config(&path)?;
        let removed = if let Some(entries) = client.existing_hook_config(&mut config) {
            let owned = owned_entries(entries);
            let removed = !owned.is_empty();
            for index in owned.iter().rev() {
                entries.remove(*index);
            }
            removed
        } else {
            false
        };
        if removed {
            client.cleanup_empty(&mut config);
            client.restore_enabled_state(&path, &mut config);
            save_config(&path, &config)?;
            results.push(LifecycleResult {
                client,
                path,
                configured: false,
            });
        }
    }
    Ok(results)
}

pub fn session_hooks_status() -> Result<Vec<(LifecycleClient, bool, Option<PathBuf>)>> {
    let home = test_home()?;
    let mut status = Vec::new();
    for &client in LifecycleClient::ALL {
        let path = client.config_path(&home);
        let configured = path
            .exists()
            .then(|| load_config(&path).ok())
            .flatten()
            .and_then(|mut config| {
                client
                    .existing_hook_config(&mut config)
                    .map(|entries| owned_entries(entries).len())
            })
            .unwrap_or(0)
            > 0;
        status.push((client, configured, path.exists().then_some(path)));
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "agent_mem_lifecycle_{label}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn install_is_idempotent_and_uninstall_preserves_other_hooks() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = temp_home("roundtrip");
        let path = home.join(".claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"theme":"dark","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
        )
        .unwrap();
        unsafe { env::set_var("AGENT_MEM_TEST_HOME", &home) };

        let first = install_session_hooks(Some("claude-code")).unwrap();
        assert!(!first[0].configured);
        let after_first = fs::read_to_string(&path).unwrap();
        assert!(path.with_file_name("settings.json.agent-mem.bak").exists());

        let second = install_session_hooks(Some("claude-code")).unwrap();
        assert!(second[0].configured);
        assert_eq!(fs::read_to_string(&path).unwrap(), after_first);

        let removed = uninstall_session_hooks(Some("claude-code")).unwrap();
        assert_eq!(removed.len(), 1);
        let final_config: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(final_config["theme"], "dark");
        assert_eq!(
            final_config["hooks"]["SessionStart"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            final_config["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "other-tool"
        );

        let before_noop = fs::read_to_string(&path).unwrap();
        assert!(
            uninstall_session_hooks(Some("claude-code"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before_noop);

        unsafe { env::remove_var("AGENT_MEM_TEST_HOME") };
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn invalid_json_is_never_modified() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = temp_home("invalid");
        let path = home.join(".claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ broken").unwrap();
        unsafe { env::set_var("AGENT_MEM_TEST_HOME", &home) };

        assert!(install_session_hooks(Some("claude-code")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");
        assert!(!path.with_file_name("settings.json.agent-mem.bak").exists());

        unsafe { env::remove_var("AGENT_MEM_TEST_HOME") };
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn shell_command_quotes_paths_with_spaces() {
        assert_eq!(shell_quote("agent-mem"), "agent-mem");
        assert_eq!(
            shell_quote("/Applications/Agent Mem/agent-mem"),
            "'/Applications/Agent Mem/agent-mem'"
        );
        assert_eq!(shell_quote("/tmp/it's/bin"), "'/tmp/it'\"'\"'s/bin'");
    }

    #[test]
    fn owned_entries_matches_quoted_windows_and_test_binaries() {
        let entries = vec![
            json!({"hooks": [{"type": "command", "command": "other-tool hook session-start"}]}),
            json!({"hooks": [{"type": "command", "command": "'/Applications/Agent Mem/agent-mem' hook session-start"}]}),
            json!({"hooks": [{"type": "command", "command": "'C:\\Users\\dev\\.cargo\\bin\\agent-mem.exe' hook session-start"}]}),
            json!({"hooks": [{"type": "command", "command": "/target/debug/deps/agent_mem-8a99b7671c4dac8d hook session-start"}]}),
        ];
        assert_eq!(owned_entries(&entries), vec![1, 2, 3]);
    }

    #[test]
    fn zcode_uninstall_restores_previous_enabled_state() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = temp_home("zcode-enabled");
        let path = home.join(".zcode/cli/config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"hooks":{"enabled":false}}"#).unwrap();
        unsafe { env::set_var("AGENT_MEM_TEST_HOME", &home) };

        install_session_hooks(Some("zcode")).unwrap();
        let installed: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(installed["hooks"]["enabled"], true);
        uninstall_session_hooks(Some("zcode")).unwrap();
        let restored: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored["hooks"]["enabled"], false);
        assert!(restored["hooks"].get("events").is_none());

        unsafe { env::remove_var("AGENT_MEM_TEST_HOME") };
        let _ = fs::remove_dir_all(home);
    }
}
