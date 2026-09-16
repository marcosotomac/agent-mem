use std::path::PathBuf;

pub type RuleEntry = (String, String, Option<String>);
pub type SessionEntry = (i64, String);

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RuleRecord {
    pub key: String,
    pub val: String,
    pub anchor: Option<String>,
    pub archived_at: Option<i64>,
    pub archive_reason: Option<String>,
    pub kind: String,
}

impl RuleRecord {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRecord {
    pub source_key: String,
    pub rel_type: String,
    pub target_key: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedRules {
    pub rules: Vec<RuleRecord>,
    pub relations: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZombieReport {
    pub key: String,
    pub missing_paths: Vec<String>,
    pub archived: bool,
}

/// Extract file paths from an anchor string (e.g. "@ src/auth/jwt.rs:42, src/models/user.rs:10").
pub fn extract_anchor_paths(raw: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let cleaned = raw.trim().trim_start_matches('@').trim();
    for part in cleaned.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let file_part = trimmed.split(':').next().unwrap_or("").trim();
        let normalized = file_part.replace('\\', "/");
        let without_prefix = normalized.trim_start_matches("./");
        if !without_prefix.is_empty() && !paths.iter().any(|p| p == without_prefix) {
            paths.push(without_prefix.to_string());
        }
    }
    paths
}

pub fn infer_kind(key: &str) -> &'static str {
    // Prefixes are ASCII; avoid allocating a lowercase copy of every key.
    let starts_with = |prefix: &str| {
        key.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    if starts_with("decision/")
        || starts_with("adr/")
        || starts_with("decision:")
        || starts_with("adr:")
    {
        "decision"
    } else if starts_with("gotcha/")
        || starts_with("bug/")
        || starts_with("gotcha:")
        || starts_with("trap/")
        || starts_with("postmortem/")
    {
        "gotcha"
    } else if starts_with("pattern/") || starts_with("pattern:") {
        "pattern"
    } else {
        "rule"
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub path: PathBuf,
    pub imported: usize,
    pub total: usize,
    pub file_created: bool,
    pub file_updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreStats {
    pub db_path: PathBuf,
    pub journal_mode: String,
    pub user_version: u32,
    pub rules_count: usize,
    pub active_rules_count: usize,
    pub archived_rules_count: usize,
    pub sessions_count: usize,
}
