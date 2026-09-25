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
    pub post_checkout_configured: bool,
    pub post_rewrite_configured: bool,
    pub trigger_target: String,
    pub trigger_updated: bool,
    pub created_rule_file: bool,
    pub rules_file_created: bool,
    pub configured_clients: Vec<crate::installer::InstallResult>,
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

/// Resolves the actual git directory for standard repositories, git worktrees, or submodules.
pub fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file()
        && let Ok(content) = fs::read_to_string(&dot_git)
    {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("gitdir:") {
                let gitdir_path = rest.trim();
                let path = PathBuf::from(gitdir_path);
                let resolved = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                let canonical = fs::canonicalize(&resolved).unwrap_or(resolved);
                if canonical.exists() {
                    return Some(canonical);
                }
            }
        }
    }
    None
}

/// Resolves the hooks directory for git repositories, worktrees, and submodules.
/// In Git worktrees, checks for `commondir` pointing back to the primary repository hooks.
/// Automatically creates the hooks directory if it does not already exist.
pub fn resolve_git_hooks_dir(root: &Path) -> Option<PathBuf> {
    let git_dir = resolve_git_dir(root)?;
    let commondir_file = git_dir.join("commondir");
    let target_git_dir = if commondir_file.is_file() {
        if let Ok(commondir_content) = fs::read_to_string(&commondir_file) {
            let common_path = PathBuf::from(commondir_content.trim());
            let resolved = if common_path.is_absolute() {
                common_path
            } else {
                git_dir.join(common_path)
            };
            let canonical = fs::canonicalize(&resolved).unwrap_or(resolved);
            if canonical.exists() {
                canonical
            } else {
                git_dir
            }
        } else {
            git_dir
        }
    } else {
        git_dir
    };

    let hooks_dir = target_git_dir.join("hooks");
    if !hooks_dir.exists() {
        let _ = fs::create_dir_all(&hooks_dir);
    }
    Some(hooks_dir)
}

/// Initialize isolated project memory (.agent-mem/, gitignore, git post-commit hook, rules file).
pub fn init_project(root: &Path) -> Result<InitReport> {
    let mem_dir = root.join(".agent-mem");
    if !mem_dir.exists() {
        fs::create_dir_all(&mem_dir)?;
    }
    let db_path = mem_dir.join("mem.db");
    let mut store = Store::open(&db_path, true)?;

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

    fn install_git_hook(dir: &Path, name: &str, cmd: &str) -> Result<bool> {
        let hook_path = dir.join(name);
        let hook_line = format!("{}\n", cmd.trim());
        if hook_path.exists() {
            let content = fs::read_to_string(&hook_path).unwrap_or_default();
            if cmd == "agent-mem sync || true"
                && content.contains("agent-mem sync 2>/dev/null || true")
            {
                fs::write(
                    &hook_path,
                    content.replace("agent-mem sync 2>/dev/null || true", cmd),
                )?;
                return Ok(true);
            }
            if !content.contains(cmd.trim()) {
                let mut new_content = content;
                if !new_content.ends_with('\n') && !new_content.is_empty() {
                    new_content.push('\n');
                }
                new_content.push_str(&hook_line);
                fs::write(&hook_path, new_content)?;
                return Ok(true);
            }
        } else {
            let new_content = format!("#!/bin/sh\n{}", hook_line);
            fs::write(&hook_path, new_content)?;
            #[cfg(unix)]
            {
                let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
            }
            return Ok(true);
        }
        Ok(false)
    }

    // 3. Configure git hooks idempotently (post-commit, post-merge, post-checkout, post-rewrite)
    if let Some(git_hooks_dir) = resolve_git_hooks_dir(root) {
        let session_cmd = "agent-mem hook post-commit 2>/dev/null || true";
        let sync_cmd = "agent-mem sync || true";

        report.hook_configured = install_git_hook(&git_hooks_dir, "post-commit", session_cmd)?;
        report.post_merge_configured = install_git_hook(&git_hooks_dir, "post-merge", sync_cmd)?;
        report.post_checkout_configured =
            install_git_hook(&git_hooks_dir, "post-checkout", sync_cmd)?;
        report.post_rewrite_configured =
            install_git_hook(&git_hooks_dir, "post-rewrite", sync_cmd)?;
    }

    // 4. Initialize .agent-rules plain text file for git tracking
    let rules_path = root.join(".agent-rules");
    if !rules_path.exists() {
        store.export_to_file(&rules_path)?;
        report.rules_file_created = true;
    } else {
        // The tracked file is authoritative, including after cloning or retrying init.
        // Import before any subsequent write can export an empty/stale local cache.
        store.sync_with_file(&rules_path)?;
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

    // Auto-register project in the global canonical registry
    crate::registry::ProjectRegistry::load()?.register(root)?;

    // MCP client configuration is intentionally opt-in. Project initialization must never
    // mutate user-level editor configuration; use `agent-mem mcp install [client]` explicitly.

    Ok(report)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDoctorReport {
    pub is_git_repo: bool,
    pub root: PathBuf,
    pub gitignore_active: bool,
    pub gitattributes_active: bool,
    pub post_commit_active: bool,
    pub post_merge_active: bool,
    pub post_checkout_active: bool,
    pub post_rewrite_active: bool,
    pub rules_file_exists: bool,
    pub rules_count: usize,
}

pub fn inspect_git_health(root: &Path) -> GitDoctorReport {
    let is_git_repo = resolve_git_dir(root).is_some();
    let gitignore_path = root.join(".gitignore");
    let gitignore_active = gitignore_path.exists()
        && fs::read_to_string(&gitignore_path)
            .map(|c| c.contains(".agent-mem"))
            .unwrap_or(false);

    let gitattributes_path = root.join(".gitattributes");
    let gitattributes_active = gitattributes_path.exists()
        && fs::read_to_string(&gitattributes_path)
            .map(|c| c.contains(".agent-rules merge=union"))
            .unwrap_or(false);

    let hooks_dir = resolve_git_hooks_dir(root);
    let check_hook = |name: &str, pattern: &str| -> bool {
        hooks_dir
            .as_ref()
            .map(|d| d.join(name))
            .filter(|p| p.exists())
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|c| c.contains(pattern))
            .unwrap_or(false)
    };

    let post_commit_active = check_hook("post-commit", "agent-mem hook post-commit")
        || check_hook("post-commit", "agent-mem session add");
    let post_merge_active = check_hook("post-merge", "agent-mem sync");
    let post_checkout_active = check_hook("post-checkout", "agent-mem sync");
    let post_rewrite_active = check_hook("post-rewrite", "agent-mem sync");

    let rules_path = root.join(".agent-rules");
    let rules_file_exists = rules_path.exists();
    let rules_count = if rules_file_exists {
        fs::read_to_string(&rules_path)
            .map(|c| Store::parse_rules_text(&c).len())
            .unwrap_or(0)
    } else {
        0
    };

    GitDoctorReport {
        is_git_repo,
        root: root.to_path_buf(),
        gitignore_active,
        gitattributes_active,
        post_commit_active,
        post_merge_active,
        post_checkout_active,
        post_rewrite_active,
        rules_file_exists,
        rules_count,
    }
}
