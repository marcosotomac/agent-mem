use crate::error::Result;
use crate::store::Store;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Default)]
pub struct InitReport {
    pub root: PathBuf,
    pub gitignore_updated: bool,
    pub gitattributes_updated: bool,
    pub hook_configured: bool,
    pub post_merge_configured: bool,
    pub trigger_target: String,
    pub trigger_updated: bool,
    pub created_rule_file: bool,
    pub rules_file_created: bool,
}

/// Find repository / project root by walking up looking for .git or .agent-mem
pub fn find_project_root_from(start: &Path) -> PathBuf {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() || current.join(".agent-mem").exists() {
            return current;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }
    start.to_path_buf()
}

pub fn find_project_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_project_root_from(&cwd)
}

pub fn global_db_path() -> PathBuf {
    if let Ok(dir) = env::var("AGENT_MEM_GLOBAL_DIR") {
        return PathBuf::from(dir).join("global.db");
    }
    if let Ok(home) = env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("agent-mem")
            .join("global.db")
    } else {
        PathBuf::from(".config").join("agent-mem").join("global.db")
    }
}

/// Initialize isolated project memory (.agent-mem/, gitignore, git post-commit hook, rules file).
pub fn init_project(root: &Path) -> Result<InitReport> {
    let mem_dir = root.join(".agent-mem");
    if !mem_dir.exists() {
        fs::create_dir_all(&mem_dir)?;
    }
    let db_path = mem_dir.join("mem.db");
    let _ = Store::open(&db_path, true)?;

    let mut report = InitReport {
        root: root.to_path_buf(),
        ..Default::default()
    };

    // 1. Update .gitignore idempotently
    let gitignore_path = root.join(".gitignore");
    if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path).unwrap_or_default();
        if !content.contains(".agent-mem") {
            let mut new_content = content;
            if !new_content.ends_with('\n') && !new_content.is_empty() {
                new_content.push('\n');
            }
            new_content.push_str(".agent-mem/\n");
            fs::write(&gitignore_path, new_content)?;
            report.gitignore_updated = true;
        }
    } else {
        fs::write(&gitignore_path, ".agent-mem/\n")?;
        report.gitignore_updated = true;
    }

    // 2. Update .gitattributes for automatic conflict-free union merges on team rules
    let gitattributes_path = root.join(".gitattributes");
    let attr_line = ".agent-rules merge=union\n";
    if gitattributes_path.exists() {
        let content = fs::read_to_string(&gitattributes_path).unwrap_or_default();
        if !content.contains(".agent-rules merge=union") {
            let mut new_content = content;
            if !new_content.ends_with('\n') && !new_content.is_empty() {
                new_content.push('\n');
            }
            new_content.push_str(attr_line);
            fs::write(&gitattributes_path, new_content)?;
            report.gitattributes_updated = true;
        }
    } else {
        fs::write(&gitattributes_path, attr_line)?;
        report.gitattributes_updated = true;
    }

    // 3. Configure .git/hooks/post-commit idempotently if this is a git repo
    let git_hooks_dir = root.join(".git").join("hooks");
    if git_hooks_dir.exists() {
        let hook_path = git_hooks_dir.join("post-commit");
        let hook_cmd = "agent-mem session add \"$(git log -1 --pretty=%s)\" 2>/dev/null || true\n";

        if hook_path.exists() {
            let content = fs::read_to_string(&hook_path).unwrap_or_default();
            if !content.contains("agent-mem session add") {
                let mut new_content = content;
                if !new_content.ends_with('\n') && !new_content.is_empty() {
                    new_content.push('\n');
                }
                new_content.push_str(hook_cmd);
                fs::write(&hook_path, new_content)?;
                report.hook_configured = true;
            }
        } else {
            let new_content = format!("#!/bin/sh\n{}", hook_cmd);
            fs::write(&hook_path, new_content)?;
            #[cfg(unix)]
            {
                let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
            }
            report.hook_configured = true;
        }

        // 3. Configure .git/hooks/post-merge idempotently for automatic team sync
        let merge_hook_path = git_hooks_dir.join("post-merge");
        let merge_hook_cmd = "agent-mem sync 2>/dev/null || true\n";

        if merge_hook_path.exists() {
            let content = fs::read_to_string(&merge_hook_path).unwrap_or_default();
            if !content.contains("agent-mem sync") {
                let mut new_content = content;
                if !new_content.ends_with('\n') && !new_content.is_empty() {
                    new_content.push('\n');
                }
                new_content.push_str(merge_hook_cmd);
                fs::write(&merge_hook_path, new_content)?;
                report.post_merge_configured = true;
            }
        } else {
            let new_content = format!("#!/bin/sh\n{}", merge_hook_cmd);
            fs::write(&merge_hook_path, new_content)?;
            #[cfg(unix)]
            {
                let _ = fs::set_permissions(&merge_hook_path, fs::Permissions::from_mode(0o755));
            }
            report.post_merge_configured = true;
        }
    }

    // 4. Initialize .agent-rules plain text file for git tracking
    let rules_path = root.join(".agent-rules");
    if !rules_path.exists() {
        let store = Store::open(&db_path, false)?;
        store.export_to_file(&rules_path)?;
        report.rules_file_created = true;
    }

    const TRIGGER_BLOCK: &str = "\
<!-- agent-mem -->
## Persistent Memory Protocol
- **On start:** Run `agent-mem context` (or MCP `mem_context`) to load project rules and recent sessions.
- **On decisions:** Run `agent-mem set <key> \"<rule>\"` (or MCP `mem_set`) to record conventions or architecture.
- **On search:** Run `agent-mem find <query>` (or MCP `mem_find`) to look up specific memories.
<!-- /agent-mem -->\n";

    // 5. Inject minimal AI trigger instruction idempotently into rules file
    let candidate_files = ["AGENTS.md", "CLAUDE.md", ".cursorrules", ".windsurfrules"];
    let trigger_target = candidate_files
        .iter()
        .map(|f| root.join(f))
        .find(|p| p.exists());

    let (target_path, created) = match trigger_target {
        Some(path) => (path, false),
        None => {
            let default_path = root.join("AGENTS.md");
            fs::write(&default_path, TRIGGER_BLOCK)?;
            (default_path, true)
        }
    };

    report.created_rule_file = created;
    report.trigger_target = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("AGENTS.md")
        .to_string();

    if !created {
        let content = fs::read_to_string(&target_path).unwrap_or_default();
        if content.contains("<!-- agent-mem -->") {
            if let Some(start) = content.find("<!-- agent-mem -->")
                && let Some(end) = content.find("<!-- /agent-mem -->")
            {
                let end_idx = end + "<!-- /agent-mem -->".len();
                let mut new_content = String::new();
                new_content.push_str(&content[..start]);
                new_content.push_str(TRIGGER_BLOCK.trim_end());
                if end_idx < content.len() {
                    new_content.push_str(&content[end_idx..]);
                }
                if new_content != content {
                    fs::write(&target_path, new_content)?;
                    report.trigger_updated = true;
                }
            }
        } else if content.contains("Memory: run `agent-mem get") {
            let new_content = content.replace(
                "Memory: run `agent-mem get <key>` to check rules, `agent-mem set <key> \"<rule>\"` to save.\n",
                TRIGGER_BLOCK,
            );
            fs::write(&target_path, new_content)?;
            report.trigger_updated = true;
        } else if !content.contains("agent-mem") {
            let mut new_content = content;
            if !new_content.ends_with('\n') && !new_content.is_empty() {
                new_content.push('\n');
            }
            new_content.push_str(TRIGGER_BLOCK);
            fs::write(&target_path, new_content)?;
            report.trigger_updated = true;
        }
    }

    Ok(report)
}
