use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) const ROUTE_BASENAME: i64 = 1;
pub(crate) const ROUTE_DIRECTORY: i64 = 2;
pub(crate) const ROUTE_PATH_SUFFIX: i64 = 3;

pub(crate) fn route_hash(route: &str) -> i64 {
    // Stable FNV-1a. Hash collisions can only add a candidate: graph retrieval
    // always validates the original anchor path before returning a rule.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in route.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

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

/// Build compact lookup routes for one normalized repository-relative path.
/// The full path handles exact matches, directory ancestors handle sibling and
/// parent/child matches, and the basename supplies validated relative matches.
/// Single-segment directories are omitted because routes such as `src` or
/// `services` produce excessively broad candidates in large monorepos.
pub(crate) fn routing_entries_for_path(raw_path: &str) -> Vec<(i64, i64)> {
    let normalized = raw_path
        .trim()
        .trim_start_matches('@')
        .trim()
        .replace('\\', "/");
    let clean = normalized.trim_start_matches("./").trim_matches('/');
    if clean.is_empty() {
        return Vec::new();
    }

    let parts: Vec<&str> = clean.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return Vec::new();
    }

    let mut routes = Vec::with_capacity(parts.len() + 1);
    routes.push((ROUTE_PATH_SUFFIX, route_hash(&parts.join("/"))));

    if parts.len() >= 2 {
        let directory = &parts[..parts.len() - 1];
        for end in (2..=directory.len()).rev().take(8) {
            routes.push((ROUTE_DIRECTORY, route_hash(&directory[..end].join("/"))));
        }
    }

    routes.push((
        ROUTE_BASENAME,
        route_hash(&parts[parts.len() - 1].to_ascii_lowercase()),
    ));

    routes
}

pub(crate) fn memory_routes_for_anchor(anchor: Option<&str>) -> Vec<(i64, i64)> {
    let mut routes = BTreeSet::new();
    if let Some(anchor) = anchor {
        for path in extract_anchor_paths(anchor).into_iter().take(8) {
            routes.extend(routing_entries_for_path(&path));
            if routes.len() >= 64 {
                break;
            }
        }
    }
    routes.into_iter().take(64).collect()
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
    pub conflicts_resolved: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticStatus {
    pub enabled: bool,
    pub dirty: bool,
    pub model_id: Option<String>,
    pub records_count: usize,
    pub built_at: Option<i64>,
    pub index_path: Option<PathBuf>,
    pub index_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBuildReport {
    pub records_count: usize,
    pub elapsed_ms: u128,
    pub index_path: PathBuf,
    pub index_bytes: u64,
    pub model_id: String,
}
