use super::{Store, now_epoch};
use crate::error::{Error, Result};
use crate::store::models::{ParsedRules, RuleRecord, SyncReport, infer_kind};
use rusqlite::{Row, TransactionBehavior, params, types::Type, types::ValueRef};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static EXPORT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const JSON_RULE: &str = "[rule-json-v1]";
const JSON_RELATION: &str = "[rel-json-v1]";

// Borrow SQLite text during export instead of allocating a String per field.
// This representation is confined to disk serialization, never MCP responses.
#[derive(serde::Serialize)]
struct ExportRule<'a> {
    key: &'a str,
    val: &'a str,
    anchor: Option<&'a str>,
    archived_at: Option<i64>,
    archive_reason: Option<&'a str>,
    kind: &'a str,
}

fn row_text<'a>(row: &'a Row<'_>, index: usize) -> rusqlite::Result<&'a str> {
    row.get_ref(index)?
        .as_str()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(e)))
}

fn optional_row_text<'a>(row: &'a Row<'_>, index: usize) -> rusqlite::Result<Option<&'a str>> {
    match row.get_ref(index)? {
        ValueRef::Null => Ok(None),
        _ => row_text(row, index).map(Some),
    }
}

fn ambiguous_text(text: &str) -> bool {
    text.trim() != text
        // A non-short-circuit reduction lets LLVM vectorize the plain-text scan.
        || text.bytes().fold(false, |found, b| found | b.is_ascii_control())
        || (text.contains('@') && (text.contains(" @ ") || text.contains(" (@ ")))
        || text.contains(" --reason")
}

fn needs_json(rule: &ExportRule<'_>) -> bool {
    ambiguous_text(rule.key)
        || rule.key.contains('=')
        || ["#", "//", ";", "[", "<<<<<<<", "=======", ">>>>>>>"]
            .iter()
            .any(|prefix| rule.key.starts_with(prefix))
        || ambiguous_text(rule.val)
        || rule
            .anchor
            .is_some_and(|s| s.is_empty() || ambiguous_text(s))
        || rule
            .archive_reason
            .is_some_and(|s| s.is_empty() || ambiguous_text(s))
        || (rule.archived_at.is_none() && rule.archive_reason.is_some())
        || rule.kind.is_empty()
        || !rule.kind.bytes().all(|b| b.is_ascii_lowercase())
}

fn append_rule(out: &mut String, rule: &ExportRule<'_>) -> Result<()> {
    if needs_json(rule) {
        return append_json(out, JSON_RULE, rule);
    }
    if rule.archived_at.is_some() {
        out.push_str("[archived] ");
    }
    if rule.kind != "rule" || infer_kind(rule.key) != "rule" {
        out.push('[');
        out.push_str(rule.kind);
        out.push_str("] ");
    }
    out.push_str(rule.key);
    out.push_str(" = ");
    out.push_str(rule.val);
    if let Some(anchor) = rule.anchor {
        out.push_str(" (@ ");
        out.push_str(anchor);
        out.push(')');
    }
    if let Some(reason) = rule.archive_reason {
        out.push_str(" --reason: ");
        out.push_str(reason);
    }
    out.push('\n');
    Ok(())
}

fn append_json(out: &mut String, prefix: &str, record: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string(record)
        .map_err(|e| Error::Usage(format!("Cannot encode shared memory: {e}")))?;
    out.push_str(prefix);
    out.push(' ');
    out.push_str(&json);
    out.push('\n');
    Ok(())
}

impl Store {
    /// Best-effort parsing for inspection. Sync rejects malformed encoded records
    /// before modifying storage; this legacy inspection API skips them.
    pub fn parse_rules_and_relations(content: &str) -> ParsedRules {
        Self::parse_rules_with_error(content).0
    }

    fn parse_rules_with_error(content: &str) -> (ParsedRules, Option<Error>) {
        let mut rules = Vec::new();
        let mut relations = Vec::new();
        let mut parse_error = None;

        for (line_number, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            let encoded = if let Some(json) = trimmed.strip_prefix(JSON_RULE) {
                Some(serde_json::from_str::<RuleRecord>(json).map(|rule| rules.push(rule)))
            } else {
                trimmed.strip_prefix(JSON_RELATION).map(|json| {
                    serde_json::from_str::<(String, String, String)>(json)
                        .map(|relation| relations.push(relation))
                })
            };
            if let Some(result) = encoded {
                if let Err(error) = result {
                    parse_error.get_or_insert_with(|| {
                        Error::Usage(format!(
                            "Invalid encoded memory at line {}: {error}",
                            line_number + 1
                        ))
                    });
                }
                continue;
            }
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

        (ParsedRules { rules, relations }, parse_error)
    }

    /// Parse plain-text rules (e.g. from .agent-rules). Supports active and [archived] rules.
    pub fn parse_rules_text(content: &str) -> Vec<RuleRecord> {
        Self::parse_rules_and_relations(content).rules
    }

    /// Export deterministically, borrowing row fields instead of materializing
    /// the entire store and partitioning owned records into temporary vectors.
    pub fn export_rules_text(&self) -> Result<String> {
        // Keep active, archived and relation sections on one WAL read snapshot.
        // Concurrent archiving must not duplicate or omit a rule between scans.
        let snapshot = self.conn.unchecked_transaction()?;
        let mut out = String::with_capacity(4096);
        out.push_str("# .agent-rules - agent-mem shared team memory\n");
        out.push_str("# Track this file in git to share rules across your team without SQLite binary conflicts.\n\n");

        self.append_rules_section(&mut out, false)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_key, rel_type, target_key FROM relations ORDER BY source_key, rel_type, target_key;",
        )?;
        let mut rows = stmt.query([])?;
        let mut header_written = false;
        while let Some(row) = rows.next()? {
            if !header_written {
                out.push_str("\n# Relations\n");
                header_written = true;
            }
            let fields = (row_text(row, 0)?, row_text(row, 1)?, row_text(row, 2)?);
            if [fields.0, fields.1, fields.2].iter().any(|s| {
                s.trim() != *s || s.contains("->") || s.bytes().any(|b| b.is_ascii_control())
            }) {
                append_json(&mut out, JSON_RELATION, &fields)?;
            } else {
                out.push_str("[rel] ");
                out.push_str(fields.0);
                out.push_str(" -> ");
                out.push_str(fields.1);
                out.push_str(" -> ");
                out.push_str(fields.2);
                out.push('\n');
            }
        }
        drop(rows);
        drop(stmt);
        self.append_rules_section(&mut out, true)?;
        snapshot.commit()?;
        Ok(out)
    }

    fn append_rules_section(&self, out: &mut String, archived: bool) -> Result<()> {
        let sql = if archived {
            "SELECT key, val, anchor, archive_reason, kind FROM memories WHERE archived_at IS NOT NULL ORDER BY key;"
        } else {
            "SELECT key, val, anchor, archive_reason, kind FROM memories WHERE archived_at IS NULL ORDER BY key;"
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        let mut rows = stmt.query([])?;
        let mut header_written = false;
        while let Some(row) = rows.next()? {
            if archived && !header_written {
                out.push_str("\n# Archived Rules\n");
                header_written = true;
            }
            append_rule(
                out,
                &ExportRule {
                    key: row_text(row, 0)?,
                    val: row_text(row, 1)?,
                    anchor: optional_row_text(row, 2)?,
                    // Stable marker: timestamps must not churn in Git.
                    archived_at: archived.then_some(1),
                    archive_reason: optional_row_text(row, 3)?,
                    kind: row_text(row, 4)?,
                },
            )?;
        }
        Ok(())
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
                conflicts_resolved: 0,
            });
        }

        let content = fs::read_to_string(path)?;
        let (parsed, parse_error) = Self::parse_rules_with_error(&content);
        if let Some(error) = parse_error {
            return Err(error);
        }

        struct ParsedEntry<'a> {
            val: &'a str,
            anchor: Option<&'a str>,
            archived_at: Option<i64>,
            archive_reason: Option<&'a str>,
            kind: &'a str,
        }

        // Deduplicate in memory: if multiple conflict markers or duplicate lines exist,
        // resolve deterministically and detect contradictions for explicit provenance.
        let mut unique_rules: BTreeMap<&str, ParsedEntry> = BTreeMap::new();
        let mut conflicts: Vec<(&str, &str, &str)> = Vec::new();

        for r in &parsed.rules {
            if let Some(prev) = unique_rules.get(r.key.as_str())
                && (prev.val != r.val.as_str()
                    || prev.kind != r.kind.as_str()
                    || prev.anchor != r.anchor.as_deref())
            {
                conflicts.push((r.key.as_str(), prev.val, r.val.as_str()));
            }
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

        let conflicts_resolved = conflicts.len();

        // 1. Begin immediate transaction for atomic reconciliation and snapshot consistency
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let now = now_epoch();

        // Record conflict provenance in session audit log
        if !conflicts.is_empty() {
            let mut insert_sess = tx.prepare_cached(
                "INSERT INTO sessions (summary, created_at) VALUES (?1, ?2);",
            )?;
            for (ckey, old_val, new_val) in &conflicts {
                let summary = format!(
                    "[conflict-resolved] '{}': superseded '{}' with '{}'",
                    ckey, old_val, new_val
                );
                insert_sess.execute(params![summary, now])?;
            }
        }

        let existing_count: usize = tx.query_row("SELECT COUNT(*) FROM memories;", [], |r| r.get(0))?;

        if existing_count == 0 {
            // Fast path for initial sync / empty database: direct inserts without diffing overhead
            let mut insert_mem = tx.prepare_cached(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);",
            )?;
            let mut insert_fts = tx.prepare_cached(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, ?5);",
            )?;
            let mut insert_rel = tx.prepare_cached(
                "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
            )?;

            for (key, entry) in &unique_rules {
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
        } else {
            // Incremental reconciliation: preserve timestamps for unmodified rules
            struct ExistingRule {
                val: String,
                anchor: Option<String>,
                archived_at: Option<i64>,
                archive_reason: Option<String>,
                kind: String,
            }

            let mut existing_map = std::collections::HashMap::with_capacity(existing_count);
            {
                let mut stmt = tx.prepare_cached(
                    "SELECT key, val, anchor, archived_at, archive_reason, kind FROM memories;",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    existing_map.insert(
                        key,
                        ExistingRule {
                            val: row.get(1)?,
                            anchor: row.get(2)?,
                            archived_at: row.get(3)?,
                            archive_reason: row.get(4)?,
                            kind: row.get(5)?,
                        },
                    );
                }
            }

            let mut insert_mem = tx.prepare_cached(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(key) DO UPDATE SET
                    val = excluded.val,
                    updated_at = excluded.updated_at,
                    anchor = excluded.anchor,
                    archived_at = excluded.archived_at,
                    archive_reason = excluded.archive_reason,
                    kind = excluded.kind;",
            )?;
            let mut delete_mem = tx.prepare_cached("DELETE FROM memories WHERE key = ?1;")?;
            let mut delete_fts = tx.prepare_cached("DELETE FROM memories_fts WHERE key = ?1;")?;
            let mut insert_fts = tx.prepare_cached(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, ?5);",
            )?;

            for (key, entry) in &unique_rules {
                match existing_map.get(*key) {
                    Some(existing) => {
                        let is_archived_match =
                            existing.archived_at.is_some() == entry.archived_at.is_some();
                        let is_unmodified = existing.val == entry.val
                            && existing.kind == entry.kind
                            && existing.anchor.as_deref() == entry.anchor
                            && is_archived_match
                            && existing.archive_reason.as_deref() == entry.archive_reason;

                        if !is_unmodified {
                            let arc = if entry.archived_at.is_some() {
                                existing.archived_at.or(Some(now))
                            } else {
                                None
                            };
                            insert_mem.execute(params![
                                key,
                                entry.val,
                                now,
                                entry.anchor,
                                arc,
                                entry.archive_reason,
                                entry.kind,
                            ])?;
                            delete_fts.execute(params![key])?;
                            insert_fts.execute(params![
                                key,
                                entry.val,
                                entry.anchor,
                                entry.archive_reason,
                                entry.kind,
                            ])?;
                        }
                    }
                    None => {
                        let arc = entry.archived_at.map(|_| now);
                        insert_mem.execute(params![
                            key,
                            entry.val,
                            now,
                            entry.anchor,
                            arc,
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
                }
            }

            // Remove deleted rules not present in file
            for key in existing_map.keys() {
                if !unique_rules.contains_key(key.as_str()) {
                    delete_mem.execute(params![key])?;
                    delete_fts.execute(params![key])?;
                }
            }

            // Reconcile relations incrementally
            let mut existing_relations = std::collections::HashSet::new();
            {
                let mut stmt = tx.prepare_cached(
                    "SELECT source_key, rel_type, target_key FROM relations;",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let s: String = row.get(0)?;
                    let r: String = row.get(1)?;
                    let t: String = row.get(2)?;
                    existing_relations.insert((s, r, t));
                }
            }

            let target_relations: std::collections::HashSet<(String, String, String)> = parsed
                .relations
                .iter()
                .cloned()
                .collect();

            if existing_relations != target_relations {
                let mut delete_rel = tx.prepare_cached(
                    "DELETE FROM relations WHERE source_key = ?1 AND rel_type = ?2 AND target_key = ?3;",
                )?;
                let mut insert_rel = tx.prepare_cached(
                    "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
                )?;

                for rel in &existing_relations {
                    if !target_relations.contains(rel) {
                        delete_rel.execute(params![rel.0, rel.1, rel.2])?;
                    }
                }
                for rel in &target_relations {
                    if !existing_relations.contains(rel) {
                        insert_rel.execute(params![rel.0, rel.1, rel.2, now])?;
                    }
                }
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
            conflicts_resolved,
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
            conflicts_resolved: 0,
        })
    }
}
