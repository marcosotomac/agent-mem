use super::{Store, now_epoch};
use crate::error::Result;
use crate::store::models::{ParsedRules, RuleRecord, SyncReport, infer_kind};
use rusqlite::{TransactionBehavior, params};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static EXPORT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Store {
    /// Parse plain-text rules and graph relations (from .agent-rules).
    pub fn parse_rules_and_relations(content: &str) -> ParsedRules {
        let mut rules = Vec::new();
        let mut relations = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("//")
                || trimmed.starts_with(';')
                || trimmed.starts_with("<<<<<<<")
                || trimmed.starts_with("=======")
                || trimmed.starts_with(">>>>>>>")
            {
                continue;
            }

            // Check for relation: [rel] source -> rel_type -> target
            if let Some(rel_str) = trimmed.strip_prefix("[rel]") {
                let parts: Vec<&str> = rel_str.split("->").map(|s| s.trim()).collect();
                if parts.len() == 3
                    && !parts[0].is_empty()
                    && !parts[1].is_empty()
                    && !parts[2].is_empty()
                {
                    relations.push((
                        parts[0].to_string(),
                        parts[1].to_string(),
                        parts[2].to_string(),
                    ));
                    continue;
                }
            }

            let (is_archived, mut line_to_parse) =
                if let Some(stripped) = trimmed.strip_prefix("[archived]") {
                    (true, stripped.trim())
                } else if let Some(stripped) = trimmed.strip_prefix("[deprecated]") {
                    (true, stripped.trim())
                } else {
                    (false, trimmed)
                };

            // Check for kind prefix: [decision], [gotcha], [pattern], [rule], etc.
            let mut explicit_kind = None;
            if line_to_parse.starts_with('[')
                && let Some(end_bracket) = line_to_parse.find(']')
            {
                let k = line_to_parse[1..end_bracket].trim();
                if !k.is_empty() {
                    explicit_kind = Some(k.to_lowercase());
                    line_to_parse = line_to_parse[end_bracket + 1..].trim();
                }
            }

            let (key, rest) = if let Some((k, r)) = line_to_parse.split_once('=') {
                (k.trim(), r.trim())
            } else if let Some((k, r)) = line_to_parse.split_once(": ") {
                (k.trim(), r.trim())
            } else {
                continue;
            };

            if key.is_empty() {
                continue;
            }

            let kind = explicit_kind.unwrap_or_else(|| infer_kind(key).to_string());

            // Check for reason: " --reason: <reason>" or " --reason <reason>"
            let (rest_without_reason, archive_reason) = if let Some(idx) = rest.rfind(" --reason:")
            {
                let r_part = rest[idx + 10..].trim();
                let reason = if r_part.is_empty() {
                    None
                } else {
                    Some(r_part.to_string())
                };
                (rest[..idx].trim(), reason)
            } else if let Some(idx) = rest.rfind(" --reason ") {
                let r_part = rest[idx + 10..].trim();
                let reason = if r_part.is_empty() {
                    None
                } else {
                    Some(r_part.to_string())
                };
                (rest[..idx].trim(), reason)
            } else {
                (rest, None)
            };

            // Check for anchor: "val (@ path:line)" or "val @ path:line"
            let (val, anchor) =
                if rest_without_reason.ends_with(')') && rest_without_reason.contains(" (@ ") {
                    if let Some(idx) = rest_without_reason.rfind(" (@ ") {
                        let v = rest_without_reason[..idx].trim();
                        let a = rest_without_reason[idx + 4..rest_without_reason.len() - 1].trim();
                        (
                            v.to_string(),
                            if a.is_empty() {
                                None
                            } else {
                                Some(a.to_string())
                            },
                        )
                    } else {
                        (rest_without_reason.to_string(), None)
                    }
                } else if let Some(idx) = rest_without_reason.rfind(" @ ") {
                    let v = rest_without_reason[..idx].trim();
                    let a = rest_without_reason[idx + 3..].trim();
                    (
                        v.to_string(),
                        if a.is_empty() {
                            None
                        } else {
                            Some(a.to_string())
                        },
                    )
                } else {
                    (rest_without_reason.to_string(), None)
                };

            rules.push(RuleRecord {
                key: key.to_string(),
                val,
                anchor,
                archived_at: if is_archived { Some(1) } else { None },
                archive_reason,
                kind,
            });
        }

        ParsedRules { rules, relations }
    }

    /// Parse plain-text rules (e.g. from .agent-rules). Supports active and [archived] rules.
    pub fn parse_rules_text(content: &str) -> Vec<RuleRecord> {
        Self::parse_rules_and_relations(content).rules
    }

    /// Export all rules and relations formatted as deterministic plain text sorted by key.
    pub fn export_rules_text(&self) -> Result<String> {
        let all_rules = self.dump_all()?;
        let all_relations = self.get_all_relations()?;
        let estimated_bytes = 128
            + all_rules
                .iter()
                .map(|rule| {
                    rule.key.len()
                        + rule.val.len()
                        + rule.anchor.as_ref().map_or(0, String::len)
                        + rule.archive_reason.as_ref().map_or(0, String::len)
                        + rule.kind.len()
                        + 24
                })
                .sum::<usize>()
            + all_relations
                .iter()
                .map(|relation| {
                    relation.source_key.len()
                        + relation.rel_type.len()
                        + relation.target_key.len()
                        + 16
                })
                .sum::<usize>();
        let mut out = String::with_capacity(estimated_bytes);
        out.push_str("# .agent-rules - agent-mem shared team memory\n");
        out.push_str("# Track this file in git to share rules across your team without SQLite binary conflicts.\n\n");

        let mut active = Vec::new();
        let mut archived = Vec::new();

        for r in all_rules {
            if r.is_archived() {
                archived.push(r);
            } else {
                active.push(r);
            }
        }

        for r in active {
            if r.kind != "rule" {
                out.push('[');
                out.push_str(&r.kind);
                out.push_str("] ");
            }
            out.push_str(&r.key);
            out.push_str(" = ");
            out.push_str(&r.val);
            if let Some(anchor) = &r.anchor {
                out.push_str(" (@ ");
                out.push_str(anchor);
                out.push(')');
            }
            out.push('\n');
        }

        if !all_relations.is_empty() {
            out.push_str("\n# Relations\n");
            for rel in all_relations {
                out.push_str("[rel] ");
                out.push_str(&rel.source_key);
                out.push_str(" -> ");
                out.push_str(&rel.rel_type);
                out.push_str(" -> ");
                out.push_str(&rel.target_key);
                out.push('\n');
            }
        }

        if !archived.is_empty() {
            out.push_str("\n# Archived Rules\n");
            for r in archived {
                out.push_str("[archived]");
                if r.kind != "rule" {
                    out.push_str(" [");
                    out.push_str(&r.kind);
                    out.push(']');
                }
                out.push(' ');
                out.push_str(&r.key);
                out.push_str(" = ");
                out.push_str(&r.val);
                if let Some(anchor) = &r.anchor {
                    out.push_str(" (@ ");
                    out.push_str(anchor);
                    out.push(')');
                }
                if let Some(reason) = &r.archive_reason {
                    out.push_str(" --reason: ");
                    out.push_str(reason);
                }
                out.push('\n');
            }
        }

        Ok(out)
    }

    /// Export current rules to a file on disk.
    pub fn export_to_file(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
        let content = self.export_rules_text()?;
        let file_name = path.file_name().ok_or_else(|| {
            crate::error::Error::Usage(format!("Export path '{}' has no file name", path.display()))
        })?;

        let (mut temp, temp_path) = loop {
            let mut temp_name = OsString::from(".");
            temp_name.push(file_name);
            temp_name.push(format!(
                ".tmp-{}-{}",
                std::process::id(),
                EXPORT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let candidate = parent.join(temp_name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (file, candidate),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };

        let result = (|| -> Result<()> {
            temp.write_all(content.as_bytes())?;
            drop(temp);
            fs::rename(&temp_path, path)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }

    /// Reconcile SQLite database with a plain-text rules file (.agent-rules).
    pub fn sync_with_file(&mut self, path: &Path) -> Result<SyncReport> {
        if !path.exists() {
            self.export_to_file(path)?;
            let total = self
                .conn
                .query_row("SELECT COUNT(*) FROM memories;", [], |row| row.get(0))?;
            return Ok(SyncReport {
                path: path.to_path_buf(),
                imported: 0,
                total,
                file_created: true,
                file_updated: false,
            });
        }

        let content = fs::read_to_string(path)?;
        let parsed = Self::parse_rules_and_relations(&content);

        struct ParsedEntry<'a> {
            val: &'a str,
            anchor: Option<&'a str>,
            archived_at: Option<i64>,
            archive_reason: Option<&'a str>,
            kind: &'a str,
        }

        // Deduplicate in memory: if multiple conflict markers or duplicate lines exist, last one wins
        let mut unique_rules: BTreeMap<&str, ParsedEntry> = BTreeMap::new();
        for r in &parsed.rules {
            unique_rules.insert(
                r.key.as_str(),
                ParsedEntry {
                    val: r.val.as_str(),
                    anchor: r.anchor.as_deref(),
                    archived_at: r.archived_at,
                    archive_reason: r.archive_reason.as_deref(),
                    kind: r.kind.as_str(),
                },
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM memories;", [])?;
        tx.execute("DELETE FROM memories_fts;", [])?;
        tx.execute("DELETE FROM relations;", [])?;

        let now = now_epoch();
        {
            let mut insert_mem = tx.prepare_cached(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);",
            )?;
            let mut insert_fts = tx.prepare_cached(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, ?5);",
            )?;
            let mut insert_rel = tx.prepare_cached(
                "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4);",
            )?;

            for (key, entry) in unique_rules {
                let arc_at = entry.archived_at.map(|_| now);
                insert_mem.execute(params![
                    key,
                    entry.val,
                    now,
                    entry.anchor,
                    arc_at,
                    entry.archive_reason,
                    entry.kind,
                ])?;
                insert_fts.execute(params![
                    key,
                    entry.val,
                    entry.anchor,
                    entry.archive_reason,
                    entry.kind,
                ])?;
            }

            for (source, rel_type, target) in &parsed.relations {
                insert_rel.execute(params![source, rel_type, target, now])?;
            }
        }
        tx.commit()?;

        let total: usize = self
            .conn
            .query_row("SELECT count(*) FROM memories;", [], |r| r.get(0))?;

        Ok(SyncReport {
            path: path.to_path_buf(),
            imported: total,
            total,
            file_created: false,
            file_updated: false,
        })
    }

    /// Explicitly export SQLite memories to a file.
    pub fn sync_export(&self, path: &Path) -> Result<SyncReport> {
        let exists = path.exists();
        self.export_to_file(path)?;
        let total = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories;", [], |row| row.get(0))?;
        Ok(SyncReport {
            path: path.to_path_buf(),
            imported: 0,
            total,
            file_created: !exists,
            file_updated: exists,
        })
    }
}
