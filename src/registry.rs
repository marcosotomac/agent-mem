use crate::error::{Error, Result};
use crate::store::Store;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::fs::OpenOptions;
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
        serde_json::from_str::<Self>(&raw).map_err(|error| {
            Error::Usage(format!(
                "Project registry '{}' is invalid: {error}",
                path.display()
            ))
        })
    }

    pub fn save(&self) -> Result<()> {
        let lock = Self::lock()?;
        let result = (|| {
            let current = Self::load()?;
            if current.projects.iter().any(|existing| {
                !self.projects.iter().any(|candidate| {
                    candidate.id == existing.id
                        && candidate.canonical_path == existing.canonical_path
                })
            }) {
                return Err(Error::Usage(
                    "Project registry changed since it was loaded; reload before saving or use deregister() to remove a project".into(),
                ));
            }
            self.save_locked()
        })();
        FileExt::unlock(&lock)?;
        result
    }

    fn lock() -> Result<fs::File> {
        let dir = registry_dir();
        fs::create_dir_all(&dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("projects.json.lock"))?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn save_locked(&self) -> Result<()> {
        let dir = registry_dir();
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

    fn mutate<T>(&mut self, change: impl FnOnce(&mut Self) -> (T, bool)) -> Result<T> {
        let lock = Self::lock()?;
        let mut current = Self::load()?;
        let (value, changed) = change(&mut current);
        if changed {
            current.save_locked()?;
        }
        self.projects = current.projects;
        FileExt::unlock(&lock)?;
        Ok(value)
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

        self.mutate(|current| {
            let found_index = current.projects.iter().position(|project| {
                project.canonical_path == record.canonical_path
                    || (project.git_remote.is_some()
                        && project.git_remote == record.git_remote
                        && !Path::new(&project.canonical_path).exists())
            });
            if let Some(index) = found_index {
                current.projects[index] = record.clone();
            } else {
                current.projects.push(record.clone());
            }
            current
                .projects
                .sort_by_key(|project| std::cmp::Reverse(project.last_accessed));
            ((), true)
        })?;
        Ok(record)
    }

    /// Prune dead projects (directories or databases that no longer exist, or ephemeral temp paths in global registry)
    pub fn prune(&mut self) -> Result<usize> {
        let is_global_default = env::var("AGENT_MEM_GLOBAL_DIR").is_err();
        self.mutate(|current| {
            let before_len = current.projects.len();
            current.projects.retain(|project| {
                let path = Path::new(&project.canonical_path);
                if is_global_default && Self::is_temporary_path(path) {
                    return false;
                }
                path.exists() && path.join(".agent-mem").join("mem.db").exists()
            });
            let removed = before_len - current.projects.len();
            (removed, removed > 0)
        })
    }

    /// Remove a project by id or path
    pub fn deregister(&mut self, id_or_path: &str) -> Result<bool> {
        self.mutate(|current| {
            let before_len = current.projects.len();
            current
                .projects
                .retain(|project| project.id != id_or_path && project.canonical_path != id_or_path);
            let removed = before_len != current.projects.len();
            (removed, removed)
        })
    }
}
