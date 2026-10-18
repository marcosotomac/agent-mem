use crate::error::Result;
use crate::init::resolve_git_dir;
use crate::store::Store;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    Gotcha,
    Decision,
    Pattern,
    Rule,
    Custom(String),
}

impl EntityKind {
    pub fn as_str(&self) -> &str {
        match self {
            EntityKind::Gotcha => "gotcha",
            EntityKind::Decision => "decision",
            EntityKind::Pattern => "pattern",
            EntityKind::Rule => "rule",
            EntityKind::Custom(s) => s.as_str(),
        }
    }

    pub fn default_key_prefix(&self) -> &str {
        match self {
            EntityKind::Gotcha => "gotcha",
            EntityKind::Decision => "decision",
            EntityKind::Pattern => "pattern",
            EntityKind::Rule => "architecture",
            EntityKind::Custom(s) => s.as_str(),
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "gotcha" => Self::Gotcha,
            "decision" => Self::Decision,
            "pattern" => Self::Pattern,
            "rule" => Self::Rule,
            other => Self::Custom(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitInfo {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub modified_files: Vec<String>,
    pub deleted_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityProposal {
    pub kind: EntityKind,
    pub key: String,
    pub val: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommit {
    pub commit_type: Option<String>,
    pub scope: Option<String>,
    pub is_breaking: bool,
    pub summary: String,
    pub entity: Option<EntityProposal>,
    pub relations: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedTrailers {
    pub explicit_kind: Option<EntityKind>,
    pub explicit_content: Option<String>,
    pub explicit_key: Option<String>,
    pub relations: Vec<(String, String)>, // (rel_type, target_key)
    pub breaking_change: Option<String>,
    pub non_trailer_body: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEntity {
    pub key: String,
    pub val: String,
    pub kind: String,
    pub anchor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReport {
    pub commit_hash: String,
    pub commit_subject: String,
    pub entity_captured: Option<HookEntity>,
    pub skip_reason: Option<&'static str>,
    pub relations: Vec<(String, String, String)>, // (source, rel_type, target)
    pub session_id: Option<i64>,
    pub rules_synced: bool,
    pub dry_run: bool,
    pub zombies: Vec<crate::store::ZombieReport>,
}

fn transient_summary(summary: &str, has_body: bool) -> bool {
    let lower = summary.trim().to_ascii_lowercase();
    let words = lower.split_whitespace().count();
    let first_word = lower.split(|c: char| !c.is_ascii_alphanumeric()).next();
    (words < 3 && !has_body)
        || matches!(
            first_word,
            Some("wip" | "tmp" | "temp" | "temporary" | "experiment" | "debug" | "revert" | "try")
        )
        || lower.contains("work in progress")
        || lower.contains("for now")
}

fn assess_entity(
    store: Option<&Store>,
    parsed: &ParsedCommit,
    body: &str,
) -> Result<(Option<EntityProposal>, Option<&'static str>)> {
    let Some(entity) = parsed.entity.as_ref() else {
        return Ok((None, None));
    };
    let trailers = parse_trailers(body);
    let explicit =
        trailers.explicit_kind.is_some() || trailers.explicit_key.is_some() || parsed.is_breaking;
    if !explicit && transient_summary(&parsed.summary, !trailers.non_trailer_body.is_empty()) {
        return Ok((None, Some("transient commit summary")));
    }
    if let Some(store) = store {
        if let Some(existing) = store.get_entry(&entity.key)? {
            if existing.archived_at.is_some() && !explicit {
                return Ok((None, Some("archived memory needs explicit replacement")));
            }
            if existing.archived_at.is_none()
                && existing.val.eq_ignore_ascii_case(&entity.val)
                && parsed.relations.is_empty()
            {
                return Ok((None, Some("duplicate memory")));
            }
            if existing.archived_at.is_none()
                && !explicit
                && !existing.val.eq_ignore_ascii_case(&entity.val)
            {
                return Ok((None, Some("existing key has different content")));
            }
        }
        if let Some(key) = store.active_duplicate(entity.kind.as_str(), &entity.val)?
            && key != entity.key
        {
            return Ok((None, Some("duplicate memory")));
        }
    }
    Ok((Some(entity.clone()), None))
}

/// Filter out agent-mem internal files, git metadata, and lockfiles from source anchors.
pub fn is_relevant_source_file(path: &str) -> bool {
    let p = path.trim().trim_start_matches("./");
    if p.is_empty()
        || p.starts_with(".agent-mem")
        || p.starts_with(".git/")
        || p.starts_with(".atl/")
    {
        return false;
    }

    let base = p.rsplit(['/', '\\']).next().unwrap_or(p);
    if base.is_empty()
        || base.starts_with(".agent-mem")
        || matches!(
            base,
            ".agent-rules"
                | ".gitignore"
                | ".gitattributes"
                | ".gitmodules"
                | "AGENTS.md"
                | "CLAUDE.md"
                | ".cursorrules"
                | ".windsurfrules"
                | ".clinerules"
                | "copilot-instructions.md"
                | "go.sum"
                | "npm-shrinkwrap.json"
                | "packages.lock.json"
        )
    {
        return false;
    }

    let lower = base.as_bytes();
    if base.ends_with(".lock")
        || base.ends_with(".lockb")
        || base.ends_with("-lock.json")
        || base.ends_with("-lock.yaml")
        || base.ends_with("-lock.yml")
        || (lower.len() > 5 && base.to_ascii_lowercase().ends_with(".lock"))
    {
        return false;
    }

    true
}

/// Extract up to 3 modified files from git diff-tree and format as anchors "@ path:1".
pub fn extract_anchors(modified_files: &[String]) -> Option<String> {
    let source_files: Vec<String> = modified_files
        .iter()
        .map(|f| f.trim())
        .filter(|f| is_relevant_source_file(f))
        .take(3)
        .map(|f| format!("{}:1", f))
        .collect();

    if source_files.is_empty() {
        None
    } else {
        Some(source_files.join(", "))
    }
}

/// Parse explicit trailers and body text from commit body.
pub fn parse_trailers(body: &str) -> ParsedTrailers {
    let mut trailers = ParsedTrailers::default();
    let mut continuation_trailer: Option<&'static str> = None;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continuation_trailer = None;
            continue;
        }

        if let Some((raw_key, raw_val)) = trimmed.split_once(':') {
            let key_norm = raw_key.trim().to_lowercase();
            let val_trim = raw_val.trim();

            match key_norm.as_str() {
                "decision" => {
                    trailers.explicit_kind = Some(EntityKind::Decision);
                    trailers.explicit_content = Some(val_trim.to_string());
                    continuation_trailer = Some("decision");
                    continue;
                }
                "gotcha" => {
                    trailers.explicit_kind = Some(EntityKind::Gotcha);
                    trailers.explicit_content = Some(val_trim.to_string());
                    continuation_trailer = Some("gotcha");
                    continue;
                }
                "rule" => {
                    trailers.explicit_kind = Some(EntityKind::Rule);
                    trailers.explicit_content = Some(val_trim.to_string());
                    continuation_trailer = Some("rule");
                    continue;
                }
                "pattern" => {
                    trailers.explicit_kind = Some(EntityKind::Pattern);
                    trailers.explicit_content = Some(val_trim.to_string());
                    continuation_trailer = Some("pattern");
                    continue;
                }
                "key" | "memory-key" | "memory_key" => {
                    trailers.explicit_key = Some(val_trim.to_string());
                    continuation_trailer = None;
                    continue;
                }
                "breaking change" | "breaking-change" => {
                    trailers.breaking_change = Some(val_trim.to_string());
                    continuation_trailer = Some("breaking");
                    continue;
                }
                "relates-to" | "relates_to" => {
                    for target in val_trim.split(',') {
                        let t = target.trim();
                        if !t.is_empty() {
                            trailers
                                .relations
                                .push(("relates_to".to_string(), t.to_string()));
                        }
                    }
                    continuation_trailer = None;
                    continue;
                }
                "mitigates" => {
                    for target in val_trim.split(',') {
                        let t = target.trim();
                        if !t.is_empty() {
                            trailers
                                .relations
                                .push(("mitigates".to_string(), t.to_string()));
                        }
                    }
                    continuation_trailer = None;
                    continue;
                }
                "depends-on" | "depends_on" => {
                    for target in val_trim.split(',') {
                        let t = target.trim();
                        if !t.is_empty() {
                            trailers
                                .relations
                                .push(("depends_on".to_string(), t.to_string()));
                        }
                    }
                    continuation_trailer = None;
                    continue;
                }
                "supersedes" => {
                    for target in val_trim.split(',') {
                        let t = target.trim();
                        if !t.is_empty() {
                            trailers
                                .relations
                                .push(("supersedes".to_string(), t.to_string()));
                        }
                    }
                    continuation_trailer = None;
                    continue;
                }
                other => {
                    if is_valid_custom_trailer_key(other) && !val_trim.is_empty() {
                        trailers.explicit_kind = Some(EntityKind::parse(other));
                        trailers.explicit_content = Some(val_trim.to_string());
                        continuation_trailer = Some("content");
                        continue;
                    }
                }
            }
        }

        // Handle continuation lines for multi-line trailer values
        if let Some(trailer_type) = continuation_trailer {
            match trailer_type {
                "decision" | "gotcha" | "rule" | "pattern" | "content" => {
                    if let Some(existing) = trailers.explicit_content.as_mut() {
                        if !existing.is_empty() {
                            existing.push(' ');
                        }
                        existing.push_str(trimmed);
                    }
                }
                "breaking" => {
                    if let Some(existing) = trailers.breaking_change.as_mut() {
                        if !existing.is_empty() {
                            existing.push(' ');
                        }
                        existing.push_str(trimmed);
                    }
                }
                _ => {}
            }
        } else {
            trailers.non_trailer_body.push(trimmed.to_string());
        }
    }

    trailers
}

fn is_valid_custom_trailer_key(s: &str) -> bool {
    !matches!(
        s,
        "signed-off-by"
            | "co-authored-by"
            | "reviewed-by"
            | "tested-by"
            | "acked-by"
            | "cc"
            | "fixes"
            | "closes"
            | "see-also"
    ) && s.len() >= 3
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn trim_comment_prefix(s: &str) -> &str {
    let trimmed = s.trim_start();
    for prefix in &[
        "///", "//! ", "//", "/*", "*", "#", "--", ";;", "\"\"\"", "'''",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("rem ") {
        return rest.trim_start();
    }
    trimmed
}

fn is_valid_directive_tag(tag: &str) -> bool {
    tag.len() >= 3
        && tag.len() <= 20
        && tag.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        && tag != "TODO"
}

/// Dynamically scan diff for developer directives in added comment lines.
/// Matches any added line with a comment token followed by `[A-Z0-9_]{3,20}: <message>`.
pub fn scan_diff_for_directives(diff_text: &str) -> Option<(EntityProposal, Option<String>)> {
    let mut current_file: Option<String> = None;
    let mut current_line: usize = 0;

    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let file = rest.trim();
            if is_relevant_source_file(file) {
                current_file = Some(file.to_string());
            } else {
                current_file = None;
            }
            continue;
        }

        if line.starts_with("@@ ") {
            if let Some(plus_idx) = line.find('+') {
                let rest = &line[plus_idx + 1..];
                let num_str = rest
                    .split(|c: char| c == ',' || c == ' ' || !c.is_ascii_digit())
                    .next()
                    .unwrap_or("0");
                current_line = num_str.parse::<usize>().unwrap_or(1);
            }
            continue;
        }

        if let Some(stripped) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                continue;
            }
            current_line = current_line.saturating_add(1);

            let Some(ref file) = current_file else {
                continue;
            };

            let trimmed = stripped.trim();
            if trimmed.is_empty() {
                continue;
            }

            let comment_body = trim_comment_prefix(trimmed);
            if comment_body.is_empty() {
                continue;
            }

            if let Some((tag, msg)) = comment_body.split_once(':') {
                let tag_trimmed = tag.trim();
                let msg_trimmed = msg.trim();
                if is_valid_directive_tag(tag_trimmed)
                    && msg_trimmed.split_whitespace().count() >= 3
                {
                    let kind = EntityKind::parse(tag_trimmed);
                    let scope = file
                        .split(['/', '\\'])
                        .find(|part| {
                            !part.is_empty() && *part != "src" && *part != "lib" && *part != "pkg"
                        })
                        .unwrap_or("core");
                    let clean_scope = scope.split('.').next().unwrap_or(scope);
                    let key = derive_entity_key(
                        kind.default_key_prefix(),
                        Some(clean_scope),
                        msg_trimmed,
                        None,
                    );
                    let anchor = format!("{}:{}", file, current_line.saturating_sub(1).max(1));
                    return Some((
                        EntityProposal {
                            kind,
                            key,
                            val: msg_trimmed.to_string(),
                        },
                        Some(anchor),
                    ));
                }
            }
        }
    }
    None
}

pub fn extract_diff_directives(
    root: &Path,
    hash: &str,
) -> Result<Option<(EntityProposal, Option<String>)>> {
    if hash.is_empty() {
        return Ok(None);
    }
    let diff_output = Command::new("git")
        .args(["show", "-U0", "--no-color", hash])
        .current_dir(root)
        .output();

    let diff_str = match diff_output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return Ok(None),
    };

    Ok(scan_diff_for_directives(&diff_str))
}

#[inline]
fn fold_latin_diacritic(ch: char) -> Option<char> {
    match ch {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'Á' | 'À' | 'Ä' | 'Â' | 'Ã' | 'Å' => {
            Some('a')
        }
        'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => Some('e'),
        'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => Some('i'),
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' | 'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' | 'Ø' => {
            Some('o')
        }
        'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => Some('u'),
        'ñ' | 'Ñ' => Some('n'),
        'ç' | 'Ç' => Some('c'),
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => Some('y'),
        _ => None,
    }
}

/// Generate a concise, token-efficient slug from a commit summary.
/// Bounded to at most 4 significant words and 32 characters, using zero intermediate allocations.
pub fn slugify(s: &str) -> String {
    let mut slug = String::with_capacity(32);
    let mut word_count = 0;
    let mut in_word = false;

    for ch in s.chars() {
        let normalized = fold_latin_diacritic(ch).unwrap_or(ch);
        if normalized.is_alphanumeric() {
            if !in_word {
                if word_count >= 4 || slug.len() >= 26 {
                    break;
                }
                if !slug.is_empty() {
                    slug.push('-');
                }
                word_count += 1;
                in_word = true;
            }
            if normalized.is_ascii() {
                slug.push(normalized.to_ascii_lowercase());
            } else {
                for lower in normalized.to_lowercase() {
                    if slug.len() + lower.len_utf8() <= 32 {
                        slug.push(lower);
                    }
                }
            }
        } else {
            in_word = false;
        }
    }

    if slug.is_empty() {
        "general".to_string()
    } else {
        slug
    }
}

fn derive_entity_key(
    prefix: &str,
    scope: Option<&str>,
    summary: &str,
    explicit_key: Option<&str>,
) -> String {
    if let Some(ek) = explicit_key.map(str::trim).filter(|k| !k.is_empty()) {
        return ek.to_string();
    }
    let slug = slugify(summary);
    match scope.map(str::trim).filter(|s| !s.is_empty()) {
        Some(sc) => format!("{}/{}/{}", prefix, sc, slug),
        None => format!("{}/{}", prefix, slug),
    }
}

/// Parse conventional commit subject and body deterministically in pure Rust.
pub fn parse_conventional_commit(subject: &str, body: &str) -> ParsedCommit {
    let clean_subject = subject.trim();
    let trailers = parse_trailers(body);

    let (commit_type, scope, is_breaking_subj, summary, breaking_text_in_subject) =
        if let Some(rest) = clean_subject.strip_prefix("BREAKING CHANGE:") {
            (
                None,
                None,
                true,
                rest.trim().to_string(),
                Some(rest.trim().to_string()),
            )
        } else if let Some(rest) = clean_subject.strip_prefix("BREAKING-CHANGE:") {
            (
                None,
                None,
                true,
                rest.trim().to_string(),
                Some(rest.trim().to_string()),
            )
        } else if let Some((header, summary_part)) = clean_subject.split_once(':') {
            let sum = summary_part.trim().to_string();
            let header_trim = header.trim();
            let (header_clean, breaking_bang) =
                if let Some(stripped) = header_trim.strip_suffix('!') {
                    (stripped.trim(), true)
                } else {
                    (header_trim, false)
                };

            if header_clean.ends_with(')')
                && let Some(open_idx) = header_clean.find('(')
            {
                let c_type = header_clean[..open_idx].trim().to_lowercase();
                let raw_scope = header_clean[open_idx + 1..header_clean.len() - 1].trim();
                let sc = if raw_scope.is_empty() {
                    None
                } else {
                    Some(raw_scope.to_lowercase())
                };
                (Some(c_type), sc, breaking_bang, sum, None)
            } else {
                let c_type = header_clean.trim().to_lowercase();
                (Some(c_type), None, breaking_bang, sum, None)
            }
        } else {
            (None, None, false, clean_subject.to_string(), None)
        };

    let is_breaking = is_breaking_subj
        || trailers.breaking_change.is_some()
        || breaking_text_in_subject.is_some();

    // Determine hypergraph entity
    let entity = if let Some(explicit_kind) = trailers.explicit_kind {
        let prefix = explicit_kind.default_key_prefix();
        let key = derive_entity_key(
            prefix,
            scope.as_deref(),
            &summary,
            trailers.explicit_key.as_deref(),
        );
        let val = trailers.explicit_content.unwrap_or_else(|| summary.clone());
        Some(EntityProposal {
            kind: explicit_kind,
            key,
            val,
        })
    } else if is_breaking {
        let key = derive_entity_key(
            "architecture",
            scope.as_deref(),
            &summary,
            trailers.explicit_key.as_deref(),
        );
        let val = trailers
            .breaking_change
            .or(breaking_text_in_subject)
            .unwrap_or_else(|| summary.clone());
        Some(EntityProposal {
            kind: EntityKind::Rule,
            key,
            val,
        })
    } else if let Some(ref c_type) = commit_type {
        match c_type.as_str() {
            "chore" | "docs" | "ci" | "test" | "tests" | "style" | "build" => None,
            "fix" | "bugfix" => scope.as_ref().map(|sc| {
                let key = derive_entity_key(
                    "gotcha",
                    Some(sc),
                    &summary,
                    trailers.explicit_key.as_deref(),
                );
                let val = if !trailers.non_trailer_body.is_empty() {
                    let desc = trailers.non_trailer_body.join(" ");
                    format!("{}: {}", summary, desc)
                } else {
                    summary.clone()
                };
                EntityProposal {
                    kind: EntityKind::Gotcha,
                    key,
                    val,
                }
            }),
            "feat" => scope.as_ref().map(|sc| {
                let key = derive_entity_key(
                    "decision",
                    Some(sc),
                    &summary,
                    trailers.explicit_key.as_deref(),
                );
                EntityProposal {
                    kind: EntityKind::Decision,
                    key,
                    val: summary.clone(),
                }
            }),
            "refactor" | "perf" => scope.as_ref().map(|sc| {
                let key = derive_entity_key(
                    "pattern",
                    Some(sc),
                    &summary,
                    trailers.explicit_key.as_deref(),
                );
                EntityProposal {
                    kind: EntityKind::Pattern,
                    key,
                    val: summary.clone(),
                }
            }),
            _ => None,
        }
    } else {
        None
    };

    ParsedCommit {
        commit_type,
        scope,
        is_breaking,
        summary,
        entity,
        relations: trailers.relations,
    }
}

/// Extract git metadata via git log and git diff-tree.
/// Returns Ok(None) gracefully for non-git directories or empty repositories without commits.
pub fn extract_git_commit_info(root: &Path) -> Result<Option<GitCommitInfo>> {
    if resolve_git_dir(root).is_none() {
        return Ok(None);
    }

    let log_output = Command::new("git")
        .args(["log", "-1", "--format=%H%x1f%s%x1f%b"])
        .current_dir(root)
        .output();

    let log_output = match log_output {
        Ok(out) if out.status.success() => out,
        _ => return Ok(None),
    };

    let raw_log = String::from_utf8_lossy(&log_output.stdout);
    let trimmed_log = raw_log.trim();
    if trimmed_log.is_empty() {
        return Ok(None);
    }

    let mut split = trimmed_log.splitn(3, '\x1f');
    let hash = match split.next() {
        Some(h) if !h.trim().is_empty() => h.trim().to_string(),
        _ => return Ok(None),
    };
    let subject = match split.next() {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Ok(None),
    };
    let body = split.next().unwrap_or("").trim().to_string();

    let deleted_output = Command::new("git")
        .args([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "--diff-filter=D",
            "-r",
            "HEAD",
        ])
        .current_dir(root)
        .output();

    let deleted_files: Vec<String> = match deleted_output {
        Ok(out) if out.status.success() => {
            let diff_str = String::from_utf8_lossy(&out.stdout);
            diff_str
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => Vec::new(),
    };

    let diff_output = Command::new("git")
        .args([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "-r",
            "HEAD",
        ])
        .current_dir(root)
        .output();

    let modified_files = match diff_output {
        Ok(out) if out.status.success() => {
            let diff_str = String::from_utf8_lossy(&out.stdout);
            diff_str
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !deleted_files.contains(l))
                .collect()
        }
        _ => Vec::new(),
    };

    Ok(Some(GitCommitInfo {
        hash,
        subject,
        body,
        modified_files,
        deleted_files,
    }))
}

/// Execute post-commit hook logic:
/// - Extracts metadata from git
/// - Parses conventional commits and explicit trailers
/// - Resolves code anchors
/// - Upserts to hypergraph and records session in single SQLite transaction
/// - Syncs to .agent-rules if present
/// - Respects dry_run flag
pub fn run_post_commit(root: &Path, dry_run: bool) -> Result<Option<HookReport>> {
    let commit_info = match extract_git_commit_info(root)? {
        Some(info) => info,
        None => return Ok(None),
    };

    let parsed = parse_conventional_commit(&commit_info.subject, &commit_info.body);
    let anchor = extract_anchors(&commit_info.modified_files);
    let db_path = root.join(".agent-mem").join("mem.db");
    let mut store = if dry_run {
        db_path
            .exists()
            .then(|| Store::open(&db_path, false))
            .transpose()?
    } else {
        Some(Store::open(&db_path, true)?)
    };
    let (mut entity, mut skip_reason) = assess_entity(store.as_ref(), &parsed, &commit_info.body)?;
    let mut resolved_anchor = anchor;

    if entity.is_none()
        && !commit_info.hash.is_empty()
        && let Ok(Some((diff_entity, diff_anchor))) =
            extract_diff_directives(root, &commit_info.hash)
    {
        let diff_parsed = ParsedCommit {
            commit_type: None,
            scope: None,
            is_breaking: false,
            summary: diff_entity.val.clone(),
            entity: Some(diff_entity),
            relations: Vec::new(),
        };
        let (approved, reason) = assess_entity(store.as_ref(), &diff_parsed, "")?;
        if let Some(approved_entity) = approved {
            entity = Some(approved_entity);
            skip_reason = None;
            if diff_anchor.is_some() {
                resolved_anchor = diff_anchor;
            }
        } else if reason.is_some() {
            skip_reason = reason;
        }
    }

    let entity_captured = entity.map(|e| HookEntity {
        key: e.key,
        val: e.val,
        kind: e.kind.as_str().to_string(),
        anchor: resolved_anchor,
    });

    let relations: Vec<(String, String, String)> = if let Some(ent) = &entity_captured {
        parsed
            .relations
            .into_iter()
            .map(|(rel_type, target)| (ent.key.clone(), rel_type, target))
            .collect()
    } else {
        Vec::new()
    };

    if dry_run {
        let zombies = if let Some(store) = store
            .as_mut()
            .filter(|_| !commit_info.deleted_files.is_empty())
        {
            store.clean_deleted_files(&commit_info.deleted_files, true)?
        } else {
            Vec::new()
        };

        return Ok(Some(HookReport {
            commit_hash: commit_info.hash,
            commit_subject: commit_info.subject,
            entity_captured,
            skip_reason,
            relations,
            session_id: None,
            rules_synced: false,
            dry_run: true,
            zombies,
        }));
    }

    let mut store = store.expect("writable store is open for a live hook");

    let entity_tuple = entity_captured.as_ref().map(|e| {
        (
            e.key.as_str(),
            e.val.as_str(),
            e.anchor.as_deref(),
            Some(e.kind.as_str()),
        )
    });

    let rel_tuples: Vec<(&str, &str, &str)> = relations
        .iter()
        .map(|(s, r, t)| (s.as_str(), r.as_str(), t.as_str()))
        .collect();

    let short_hash = &commit_info.hash[..7.min(commit_info.hash.len())];
    let session_summary = if !short_hash.is_empty() {
        format!("[{}] {}", short_hash, commit_info.subject)
    } else {
        commit_info.subject.clone()
    };
    let already_recorded = store
        .session_list(20)?
        .iter()
        .any(|session| session.1 == session_summary);
    let session_id = store.capture_commit(
        entity_tuple,
        &rel_tuples,
        if already_recorded {
            ""
        } else {
            &session_summary
        },
    )?;

    let zombies = if !commit_info.deleted_files.is_empty() {
        store.clean_deleted_files(&commit_info.deleted_files, false)?
    } else {
        Vec::new()
    };
    let zombies_cleaned = !zombies.is_empty();

    let rules_file = crate::store::resolve_rules_path(root);
    let rules_synced = if rules_file.exists()
        && (entity_captured.is_some() || !relations.is_empty() || zombies_cleaned)
    {
        store.export_to_path(&rules_file)?;
        true
    } else {
        false
    };

    Ok(Some(HookReport {
        commit_hash: commit_info.hash,
        commit_subject: commit_info.subject,
        entity_captured,
        skip_reason,
        relations,
        session_id,
        rules_synced,
        dry_run: false,
        zombies,
    }))
}

/// Extract all files modified in the working tree (staged, unstaged, untracked)
/// or fallback to files touched in HEAD commit.
pub fn get_diff_files(root: &Path) -> Vec<String> {
    if resolve_git_dir(root).is_none() {
        return Vec::new();
    }

    let mut files = Vec::new();

    // 1. Inspect uncommitted changes: git status --porcelain -uall
    if let Ok(out) = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(root)
        .output()
        && out.status.success()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.len() > 3 {
                let path_part = line[3..].trim();
                let clean_path = if let Some((_, new_p)) = path_part.split_once(" -> ") {
                    new_p.trim()
                } else {
                    path_part
                };
                let trimmed = clean_path.trim_matches('"');
                if !trimmed.is_empty() && !files.iter().any(|f| f == trimmed) {
                    files.push(trimmed.to_string());
                }
            }
        }
    }

    // 2. If working tree is clean, fallback to files touched in HEAD commit
    if files.is_empty()
        && let Ok(out) = Command::new("git")
            .args([
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-only",
                "-r",
                "HEAD",
            ])
            .current_dir(root)
            .output()
        && out.status.success()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let trimmed = line.trim().trim_matches('"');
            if !trimmed.is_empty() && !files.iter().any(|f| f == trimmed) {
                files.push(trimmed.to_string());
            }
        }
    }

    files
}
