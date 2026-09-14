use crate::error::Result;
use crate::init::resolve_git_dir;
use crate::store::Store;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Gotcha,
    Decision,
    Pattern,
    Rule,
}

impl EntityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Gotcha => "gotcha",
            EntityKind::Decision => "decision",
            EntityKind::Pattern => "pattern",
            EntityKind::Rule => "rule",
        }
    }

    pub fn default_key_prefix(&self) -> &'static str {
        match self {
            EntityKind::Gotcha => "gotcha",
            EntityKind::Decision => "decision",
            EntityKind::Pattern => "pattern",
            EntityKind::Rule => "architecture",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitInfo {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub modified_files: Vec<String>,
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
    pub relations: Vec<(String, String, String)>, // (source, rel_type, target)
    pub session_id: Option<i64>,
    pub rules_synced: bool,
    pub dry_run: bool,
}

/// Filter out agent-mem internal files, git metadata, and lockfiles from source anchors.
pub fn is_relevant_source_file(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    if p == ".agent-rules"
        || p.starts_with(".agent-mem")
        || p == ".gitignore"
        || p == ".gitattributes"
        || p.starts_with(".git/")
        || p == "AGENTS.md"
        || p == "CLAUDE.md"
        || p == ".cursorrules"
        || p == ".windsurfrules"
    {
        return false;
    }
    if p == "Cargo.lock"
        || p == "package-lock.json"
        || p == "pnpm-lock.yaml"
        || p == "yarn.lock"
        || p == "poetry.lock"
        || p == "Gemfile.lock"
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
                _ => {}
            }
        }

        // Handle continuation lines for multi-line trailer values
        if let Some(trailer_type) = continuation_trailer {
            match trailer_type {
                "decision" | "gotcha" | "rule" | "pattern" => {
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
        let key = match &scope {
            Some(sc) => format!("{}/{}", prefix, sc),
            None => format!("{}/general", prefix),
        };
        let val = trailers.explicit_content.unwrap_or_else(|| summary.clone());
        Some(EntityProposal {
            kind: explicit_kind,
            key,
            val,
        })
    } else if is_breaking {
        let key = match &scope {
            Some(sc) => format!("architecture/{}", sc),
            None => "architecture/general".to_string(),
        };
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
                let key = format!("gotcha/{}", sc);
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
                let key = format!("decision/{}", sc);
                EntityProposal {
                    kind: EntityKind::Decision,
                    key,
                    val: summary.clone(),
                }
            }),
            "refactor" | "perf" => scope.as_ref().map(|sc| {
                let key = format!("pattern/{}", sc);
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
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => Vec::new(),
    };

    Ok(Some(GitCommitInfo {
        hash,
        subject,
        body,
        modified_files,
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

    let entity_captured = parsed.entity.map(|e| HookEntity {
        key: e.key,
        val: e.val,
        kind: e.kind.as_str().to_string(),
        anchor,
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
        return Ok(Some(HookReport {
            commit_hash: commit_info.hash,
            commit_subject: commit_info.subject,
            entity_captured,
            relations,
            session_id: None,
            rules_synced: false,
            dry_run: true,
        }));
    }

    let db_path = root.join(".agent-mem").join("mem.db");
    let mut store = Store::open(&db_path, true)?;

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

    let session_id = store.capture_commit(entity_tuple, &rel_tuples, &commit_info.subject)?;

    let rules_file = root.join(".agent-rules");
    let rules_synced =
        if rules_file.exists() && (entity_captured.is_some() || !relations.is_empty()) {
            let _ = store.export_to_file(&rules_file);
            true
        } else {
            false
        };

    Ok(Some(HookReport {
        commit_hash: commit_info.hash,
        commit_subject: commit_info.subject,
        entity_captured,
        relations,
        session_id,
        rules_synced,
        dry_run: false,
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
