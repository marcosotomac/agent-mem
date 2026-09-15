use crate::error::{Error, Result};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub canonical_path: String,
    pub git_remote: Option<String>,
    pub last_accessed: i64,
    pub rules_count: usize,
    pub archived_count: usize,
    pub sessions_count: usize,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectRecord>,
}

pub fn registry_dir() -> PathBuf {
    if let Ok(dir) = env::var("AGENT_MEM_GLOBAL_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        PathBuf::from(home).join(".config").join("agent-mem")
    } else {
        PathBuf::from(".config").join("agent-mem")
    }
}

pub fn registry_path() -> PathBuf {
    registry_dir().join("projects.json")
}

impl ProjectRegistry {
    pub fn load() -> Result<Self> {
        let path = registry_path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }

        match serde_json::from_str::<Self>(&raw) {
            Ok(reg) => Ok(reg),
            Err(_) => Ok(Self::default()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = registry_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        let path = registry_path();
        let tmp_path = dir.join(format!(
            "projects.json.tmp.{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let formatted = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Usage(format!("Failed to serialize project registry: {}", e)))?;

        fs::write(&tmp_path, format!("{}\n", formatted))?;
        fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    /// Read git remote URL from .git/config without spawning subshells
    pub fn extract_git_remote(root: &Path) -> Option<String> {
        let git_dir = root.join(".git");
        let config_file = if git_dir.is_dir() {
            git_dir.join("config")
        } else if git_dir.is_file() {
            let content = fs::read_to_string(&git_dir).ok()?;
            let gitdir_line = content.lines().find(|l| l.starts_with("gitdir:"))?;
            let rel_or_abs = gitdir_line.trim_start_matches("gitdir:").trim();
            let path = if Path::new(rel_or_abs).is_absolute() {
                PathBuf::from(rel_or_abs)
            } else {
                root.join(rel_or_abs)
            };
            path.join("config")
        } else {
            return None;
        };

        if let Ok(config_content) = fs::read_to_string(config_file) {
            let mut in_origin = false;
            for line in config_content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    in_origin = trimmed == "[remote \"origin\"]";
                } else if in_origin && trimmed.starts_with("url =") {
                    let url = trimmed.trim_start_matches("url =").trim();
                    if !url.is_empty() {
                        return Some(url.to_string());
                    }
                }
            }
        }

        None
    }

    /// Canonicalize path safely, resolving symlinks and cross-platform casing differences
    pub fn canonicalize_path(p: &Path) -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
    }

    /// Check if a path is located in a system temporary directory
    pub fn is_temporary_path(path: &Path) -> bool {
        let temp = std::env::temp_dir();
        let canonical_temp = temp.canonicalize().unwrap_or_else(|_| temp.clone());
        path.starts_with(&temp)
            || path.starts_with(&canonical_temp)
            || path.starts_with("/tmp")
            || path.starts_with("/var/folders")
            || path.starts_with("/private/var/folders")
            || path.starts_with("/private/tmp")
    }

    /// Generate deterministic stable ID
    pub fn generate_id(canonical_path: &str, git_remote: Option<&str>) -> String {
        let key = git_remote.unwrap_or(canonical_path);
        let mut hasher = 5381u64;
        for b in key.bytes() {
            hasher = ((hasher << 5).wrapping_add(hasher)).wrapping_add(b as u64);
        }
        format!("{:08x}", hasher & 0xFFFF_FFFF)
    }

    /// Register or update a project record idempotently
    pub fn register(&mut self, root: &Path) -> Result<ProjectRecord> {
        let canonical = Self::canonicalize_path(root);
        let canonical_str = canonical.to_string_lossy().to_string();

        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());

        let git_remote = Self::extract_git_remote(&canonical);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Query database stats if .agent-mem/mem.db exists
        let db_path = canonical.join(".agent-mem").join("mem.db");
        let (rules_count, archived_count, sessions_count, db_size_bytes) = if db_path.exists() {
            let meta = fs::metadata(&db_path).ok();
            let size = meta.map(|m| m.len()).unwrap_or(0);
            if let Ok(store) = Store::open(&db_path, true) {
                if let Ok(stats) = store.stats(&db_path) {
                    (
                        stats.active_rules_count,
                        stats.archived_rules_count,
                        stats.sessions_count,
                        size,
                    )
                } else {
                    (0, 0, 0, size)
                }
            } else {
                (0, 0, 0, size)
            }
        } else {
            (0, 0, 0, 0)
        };

        let id = Self::generate_id(&canonical_str, git_remote.as_deref());

        // Anti-collision & Deduplication check:
        // 1. Exact canonical path match
        // 2. Git remote match with a dead path (moved/renamed folder)
        let mut found_index = None;
        for (i, p) in self.projects.iter().enumerate() {
            if p.canonical_path == canonical_str {
                found_index = Some(i);
                break;
            }
            if let (Some(r1), Some(r2)) = (&p.git_remote, &git_remote)
                && r1 == r2
                && !Path::new(&p.canonical_path).exists()
            {
                found_index = Some(i);
                break;
            }
        }

        let record = ProjectRecord {
            id,
            name,
            canonical_path: canonical_str,
            git_remote,
            last_accessed: now,
            rules_count,
            archived_count,
            sessions_count,
            db_size_bytes,
        };

        // If registering to the default global registry (no AGENT_MEM_GLOBAL_DIR set)
        // and the path is within a temporary directory (e.g. from tests or /tmp),
        // do not persist it to the global projects.json.
        if env::var("AGENT_MEM_GLOBAL_DIR").is_err() && Self::is_temporary_path(&canonical) {
            return Ok(record);
        }

        if let Some(idx) = found_index {
            self.projects[idx] = record.clone();
        } else {
            self.projects.push(record.clone());
        }

        // Sort most recently accessed first
        self.projects
            .sort_by_key(|a| std::cmp::Reverse(a.last_accessed));

        self.save()?;
        Ok(record)
    }

    /// Prune dead projects (directories or databases that no longer exist, or ephemeral temp paths in global registry)
    pub fn prune(&mut self) -> Result<usize> {
        let before_len = self.projects.len();
        let is_global_default = env::var("AGENT_MEM_GLOBAL_DIR").is_err();
        self.projects.retain(|p| {
            let path = Path::new(&p.canonical_path);
            if is_global_default && Self::is_temporary_path(path) {
                return false;
            }
            path.exists() && path.join(".agent-mem").join("mem.db").exists()
        });
        let removed = before_len - self.projects.len();
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    /// Remove a project by id or path
    pub fn deregister(&mut self, id_or_path: &str) -> Result<bool> {
        let before_len = self.projects.len();
        self.projects
            .retain(|p| p.id != id_or_path && p.canonical_path != id_or_path);
        let removed = before_len != self.projects.len();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }
}
