use crate::error::Result;
use crate::security::{MAX_PROVENANCE_BYTES, reject_secret};
use rusqlite::{Connection, Transaction, TransactionBehavior, params, params_from_iter};
#[cfg(feature = "semantic-local")]
use std::cell::RefCell;
use std::collections::HashSet;
#[cfg(feature = "semantic-local")]
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub mod graph;
pub mod models;
pub use models::*;
pub mod schema;
#[cfg(feature = "semantic-local")]
pub mod semantic;
pub mod session;
pub mod sync;

pub const MAX_KEY_BYTES: usize = 512;
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_ANCHOR_BYTES: usize = 4 * 1024;
pub const MAX_KIND_BYTES: usize = 64;
pub const MAX_RELATION_TYPE_BYTES: usize = 64;
pub const TRUST_LOCAL: &str = "local";
pub const TRUST_REVIEWED: &str = "reviewed";
pub const TRUST_UNTRUSTED: &str = "untrusted";

pub struct Store {
    pub(crate) conn: Connection,
    #[cfg(feature = "semantic-local")]
    pub(crate) db_path: Option<PathBuf>,
    #[cfg(feature = "semantic-local")]
    pub(crate) semantic_runtime: RefCell<semantic::SemanticRuntime>,
}

pub(crate) fn mark_semantic_dirty(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(
        "UPDATE semantic_state SET dirty = 1 WHERE singleton = 1;",
        [],
    )?;
    Ok(())
}

fn route_insert_sql(route_count: usize) -> &'static str {
    static SQL: OnceLock<Vec<String>> = OnceLock::new();
    &SQL.get_or_init(|| {
        (0..=64)
            .map(|count| {
                let values = (0..count)
                    .map(|index| {
                        let kind_param = index * 2 + 2;
                        let hash_param = kind_param + 1;
                        format!("(?1, ?{kind_param}, ?{hash_param})")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "INSERT OR IGNORE INTO memory_routes (memory_key, route_kind, route_hash) VALUES {values};"
                )
            })
            .collect()
    })[route_count]
}

pub(crate) fn insert_memory_routes(
    tx: &Transaction<'_>,
    key: &str,
    anchor: Option<&str>,
) -> Result<()> {
    let routes = memory_routes_for_anchor(anchor);
    if routes.is_empty() {
        return Ok(());
    }
    let mut values = Vec::with_capacity(routes.len() * 2 + 1);
    values.push(rusqlite::types::Value::Text(key.to_owned()));
    for (kind, hash) in &routes {
        values.push(rusqlite::types::Value::Integer(*kind));
        values.push(rusqlite::types::Value::Integer(*hash));
    }
    let mut stmt = tx.prepare_cached(route_insert_sql(routes.len()))?;
    stmt.execute(params_from_iter(values))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRule<'a> {
    pub key: &'a str,
    pub val: &'a str,
    pub anchor: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub relation: Option<(&'a str, &'a str)>,
}

pub(crate) fn reject_oversized(field: &str, value: &str, max: usize) -> Result<()> {
    if value.len() > max {
        return Err(crate::error::Error::Usage(format!(
            "Memory {field} exceeds the {max}-byte limit"
        )));
    }
    Ok(())
}

pub(crate) fn validate_batch_rule(rule: &BatchRule<'_>) -> Result<()> {
    if rule.key.trim().is_empty() {
        return Err(crate::error::Error::Usage(
            "Memory key cannot be empty".into(),
        ));
    }
    if rule.val.trim().is_empty() {
        return Err(crate::error::Error::Usage(
            "Memory value cannot be empty".into(),
        ));
    }
    reject_oversized("key", rule.key, MAX_KEY_BYTES)?;
    reject_secret(rule.key)?;
    reject_oversized("value", rule.val, MAX_VALUE_BYTES)?;
    reject_secret(rule.val)?;
    if let Some(anchor) = rule.anchor {
        reject_oversized("anchor", anchor, MAX_ANCHOR_BYTES)?;
        reject_secret(anchor)?;
    }
    if let Some(kind) = rule.kind {
        reject_oversized("kind", kind, MAX_KIND_BYTES)?;
        reject_secret(kind)?;
    }
    if let Some((rel_type, target)) = rule.relation {
        if rel_type.trim().is_empty() || target.trim().is_empty() {
            return Err(crate::error::Error::Usage(
                "Relation type and target cannot be empty".into(),
            ));
        }
        reject_oversized("relation type", rel_type, MAX_RELATION_TYPE_BYTES)?;
        reject_oversized("relation target", target, MAX_KEY_BYTES)?;
        reject_secret(rel_type)?;
        reject_secret(target)?;
    }
    Ok(())
}

impl Store {
    /// Set or update a key-value memory rule with optional repo-relative code anchor and entity kind.
    pub fn set_entry(
        &mut self,
        key: &str,
        val: &str,
        anchor: Option<&str>,
        kind: Option<&str>,
    ) -> Result<()> {
        self.set_entry_with_relation(key, val, anchor, kind, None)
    }

    /// Atomically insert or update a batch of memory rules in a single SQLite transaction.
    pub fn set_batch<'a>(&mut self, rules: &[BatchRule<'a>]) -> Result<usize> {
        self.set_batch_with_metadata(rules, "local:api", TRUST_LOCAL)
    }

    /// Atomically insert memories and their audit metadata. Callers may provide
    /// a compact provenance label, but trust is constrained to the closed set.
    pub fn set_batch_with_metadata<'a>(
        &mut self,
        rules: &[BatchRule<'a>],
        provenance: &str,
        trust: &str,
    ) -> Result<usize> {
        if rules.is_empty() {
            return Ok(0);
        }

        let provenance = provenance.trim();
        if provenance.is_empty() || provenance.len() > MAX_PROVENANCE_BYTES {
            return Err(crate::error::Error::Usage(format!(
                "Memory provenance must be between 1 and {MAX_PROVENANCE_BYTES} bytes"
            )));
        }
        reject_secret(provenance)?;
        if !matches!(trust, TRUST_LOCAL | TRUST_REVIEWED | TRUST_UNTRUSTED) {
            return Err(crate::error::Error::Usage(format!(
                "Invalid trust state '{trust}'"
            )));
        }

        // Validate all rules up front before any transaction or I/O
        for rule in rules {
            validate_batch_rule(rule)?;
        }

        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let empty_store: bool =
            tx.query_row("SELECT COUNT(*) = 0 FROM memories;", [], |row| row.get(0))?;
        let mut seen_batch_keys = HashSet::with_capacity(rules.len());
        let unique_initial_load = empty_store
            && rules
                .iter()
                .all(|rule| seen_batch_keys.insert(rule.key.trim()));
        let rebuild_route_lookup = rules.len() >= 1_024 && empty_store;
        if rebuild_route_lookup {
            tx.execute("DROP INDEX IF EXISTS idx_memory_routes_lookup;", [])?;
        }

        {
            let mut insert_mem = tx.prepare_cached(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)
                 ON CONFLICT(key) DO UPDATE SET val = excluded.val, updated_at = excluded.updated_at, anchor = excluded.anchor, archived_at = NULL, archive_reason = NULL, kind = excluded.kind;",
            )?;
            let mut delete_fts = tx.prepare_cached("DELETE FROM memories_fts WHERE key = ?1;")?;
            let mut insert_fts = tx.prepare_cached(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) VALUES (?1, ?2, ?3, NULL, ?4);",
            )?;
            let mut insert_rel = tx.prepare_cached(
                "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
            )?;
            let mut delete_routes =
                tx.prepare_cached("DELETE FROM memory_routes WHERE memory_key = ?1;")?;
            let mut upsert_metadata = tx.prepare_cached(
                "INSERT INTO memory_metadata (memory_key, provenance, trust, created_at, reviewed_at)
                 VALUES (?1, ?2, ?3, ?4, CASE WHEN ?3 = 'reviewed' THEN ?4 ELSE NULL END)
                 ON CONFLICT(memory_key) DO UPDATE SET
                    provenance = excluded.provenance,
                    trust = CASE
                        WHEN memory_metadata.trust = 'reviewed' AND excluded.trust = 'local'
                            THEN 'reviewed'
                        ELSE excluded.trust
                    END,
                    reviewed_at = CASE
                        WHEN memory_metadata.trust = 'reviewed' AND excluded.trust = 'local'
                            THEN memory_metadata.reviewed_at
                        ELSE excluded.reviewed_at
                    END;",
            )?;

            for r in rules {
                let trimmed_key = r.key.trim();
                let trimmed_val = r.val.trim();
                let trimmed_anchor = r.anchor.map(|a| a.trim()).filter(|a| !a.is_empty());
                let effective_kind = r
                    .kind
                    .map(|k| k.trim())
                    .filter(|k| !k.is_empty())
                    .unwrap_or_else(|| infer_kind(trimmed_key));

                insert_mem.execute(params![
                    trimmed_key,
                    trimmed_val,
                    now,
                    trimmed_anchor,
                    effective_kind
                ])?;
                if !unique_initial_load {
                    delete_fts.execute(params![trimmed_key])?;
                    delete_routes.execute(params![trimmed_key])?;
                }
                insert_fts.execute(params![
                    trimmed_key,
                    trimmed_val,
                    trimmed_anchor,
                    effective_kind
                ])?;
                insert_memory_routes(&tx, trimmed_key, trimmed_anchor)?;
                upsert_metadata.execute(params![trimmed_key, provenance, trust, now])?;

                if let Some((rel_type, target)) = r.relation {
                    insert_rel.execute(params![
                        trimmed_key,
                        rel_type.trim(),
                        target.trim(),
                        now
                    ])?;
                }
            }
        }

        if rebuild_route_lookup {
            tx.execute(
                "CREATE INDEX idx_memory_routes_lookup
                 ON memory_routes(route_kind, route_hash, memory_key);",
                [],
            )?;
        }

        mark_semantic_dirty(&tx)?;

        tx.commit()?;
        Ok(rules.len())
    }

    /// Atomically set a memory and, optionally, one directed relation.
    pub fn set_entry_with_relation(
        &mut self,
        key: &str,
        val: &str,
        anchor: Option<&str>,
        kind: Option<&str>,
        relation: Option<(&str, &str)>,
    ) -> Result<()> {
        self.set_batch(&[BatchRule {
            key,
            val,
            anchor,
            kind,
            relation,
        }])?;
        Ok(())
    }

    pub fn metadata(&self, key: &str) -> Result<Option<MemoryMetadata>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT memory_key, provenance, trust, created_at, reviewed_at
             FROM memory_metadata WHERE memory_key = ?1 LIMIT 1;",
        )?;
        let mut rows = stmt.query(params![key.trim()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(MemoryMetadata {
                key: row.get(0)?,
                provenance: row.get(1)?,
                trust: row.get(2)?,
                created_at: row.get(3)?,
                reviewed_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Current value followed by its retained previous revisions, newest first.
    pub fn history(&self, key: &str) -> Result<Vec<MemoryRevision>> {
        let key = key.trim();
        let mut history = Vec::new();
        let mut current = self.conn.prepare_cached(
            "SELECT m.val, m.anchor, m.kind, m.archived_at, m.archive_reason,
                        m.updated_at, d.provenance, d.trust
                 FROM memories m LEFT JOIN memory_metadata d ON d.memory_key = m.key
                 WHERE m.key = ?1;",
        )?;
        let mut rows = current.query(params![key])?;
        if let Some(row) = rows.next()? {
            history.push(MemoryRevision {
                val: row.get(0)?,
                anchor: row.get(1)?,
                kind: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                updated_at: row.get(5)?,
                superseded_at: None,
                provenance: row.get(6)?,
                trust: row.get(7)?,
                change_type: "current".into(),
            });
        }
        drop(rows);
        drop(current);
        let mut revisions = self.conn.prepare_cached(
            "SELECT val, anchor, kind, archived_at, archive_reason,
                    updated_at, superseded_at, provenance, trust, change_type
             FROM memory_revisions WHERE memory_key = ?1 ORDER BY id DESC LIMIT 20;",
        )?;
        let rows = revisions.query_map(params![key], |row| {
            Ok(MemoryRevision {
                val: row.get(0)?,
                anchor: row.get(1)?,
                kind: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                updated_at: row.get(5)?,
                superseded_at: row.get(6)?,
                provenance: row.get(7)?,
                trust: row.get(8)?,
                change_type: row.get(9)?,
            })
        })?;
        for revision in rows {
            history.push(revision?);
        }
        Ok(history)
    }

    /// Trust transitions are deliberately exposed through the native CLI only;
    /// an MCP model cannot promote its own untrusted input.
    pub fn set_trust(&mut self, key: &str, trust: &str) -> Result<bool> {
        if !matches!(trust, TRUST_REVIEWED | TRUST_UNTRUSTED) {
            return Err(crate::error::Error::Usage(
                "Trust must be 'reviewed' or 'untrusted'".into(),
            ));
        }
        let now = now_epoch();
        let changed = self.conn.execute(
            "UPDATE memory_metadata
             SET trust = ?2, reviewed_at = CASE WHEN ?2 = 'reviewed' THEN ?3 ELSE NULL END
             WHERE memory_key = ?1;",
            params![key.trim(), trust, now],
        )?;
        Ok(changed > 0)
    }

    /// Set or update a key-value memory rule with optional repo-relative code anchor.
    pub fn set_with_anchor(&mut self, key: &str, val: &str, anchor: Option<&str>) -> Result<()> {
        self.set_entry(key, val, anchor, None)
    }

    /// Set or update a key-value memory rule.
    pub fn set(&mut self, key: &str, val: &str) -> Result<()> {
        self.set_entry(key, val, None, None)
    }

    /// Archive a key-value memory rule with an optional deprecation or migration reason.
    pub fn archive(&mut self, key: &str, reason: Option<&str>) -> Result<bool> {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty() {
            return Ok(false);
        }

        let now = now_epoch();
        let trimmed_reason = reason.map(|r| r.trim()).filter(|r| !r.is_empty());

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let changes = tx.execute(
            "UPDATE memories SET archived_at = ?1, archive_reason = ?2, updated_at = ?3 WHERE key = ?4;",
            params![now, trimmed_reason, now, trimmed_key],
        )?;

        if changes > 0 {
            tx.execute(
                "DELETE FROM memories_fts WHERE key = ?1;",
                params![trimmed_key],
            )?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) 
                 SELECT key, val, anchor, archive_reason, kind FROM memories WHERE key = ?1;",
                params![trimmed_key],
            )?;
            mark_semantic_dirty(&tx)?;
        }

        tx.commit()?;
        Ok(changes > 0)
    }

    /// Unarchive / reactivate a previously archived rule.
    pub fn unarchive(&mut self, key: &str) -> Result<bool> {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty() {
            return Ok(false);
        }

        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let changes = tx.execute(
            "UPDATE memories SET archived_at = NULL, archive_reason = NULL, updated_at = ?1 WHERE key = ?2;",
            params![now, trimmed_key],
        )?;

        if changes > 0 {
            tx.execute(
                "DELETE FROM memories_fts WHERE key = ?1;",
                params![trimmed_key],
            )?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) 
                 SELECT key, val, anchor, archive_reason, kind FROM memories WHERE key = ?1;",
                params![trimmed_key],
            )?;
            mark_semantic_dirty(&tx)?;
        }

        tx.commit()?;
        Ok(changes > 0)
    }

    /// Dump active (non-archived) memories ordered by key.
    pub fn dump_active(&self) -> Result<Vec<RuleRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor, archived_at, archive_reason, kind
             FROM memories
             WHERE archived_at IS NULL
             ORDER BY key ASC;",
        )?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();
        while let Some(row) = rows.next()? {
            list.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                kind: row.get(5)?,
            });
        }
        Ok(list)
    }

    /// Retrieve a memory rule record with anchor, archival metadata, and entity kind.
    pub fn get_entry(&self, key: &str) -> Result<Option<RuleRecord>> {
        if key.trim().is_empty() {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT key, val, anchor, archived_at, archive_reason, kind FROM memories WHERE key = ?1 LIMIT 1;")?;
        let mut rows = stmt.query(params![key.trim()])?;

        if let Some(row) = rows.next()? {
            Ok(Some(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                kind: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Retrieve a key-value memory rule value.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        if key.trim().is_empty() {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT val FROM memories WHERE key = ?1 LIMIT 1;")?;
        let mut rows = stmt.query(params![key.trim()])?;

        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Delete a key-value memory rule and any connected relations. Returns true if key was deleted.
    pub fn del(&mut self, key: &str) -> Result<bool> {
        if key.trim().is_empty() {
            return Ok(false);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changes = tx.execute("DELETE FROM memories WHERE key = ?1;", params![key.trim()])?;
        if changes > 0 {
            tx.execute(
                "DELETE FROM memories_fts WHERE key = ?1;",
                params![key.trim()],
            )?;
            tx.execute(
                "DELETE FROM memory_routes WHERE memory_key = ?1;",
                params![key.trim()],
            )?;
            tx.execute(
                "DELETE FROM memory_metadata WHERE memory_key = ?1;",
                params![key.trim()],
            )?;
            tx.execute(
                "DELETE FROM relations WHERE source_key = ?1 OR target_key = ?1;",
                params![key.trim()],
            )?;
            mark_semantic_dirty(&tx)?;
        }
        tx.commit()?;
        Ok(changes > 0)
    }

    /// Dump all ACTIVE memories ordered by key.
    pub fn dump(&self) -> Result<Vec<RuleEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor FROM memories WHERE archived_at IS NULL ORDER BY key ASC;",
        )?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(list)
    }

    /// Dump at most `limit` active memories ordered by key without materializing the full store.
    pub fn dump_limited(&self, limit: usize) -> Result<Vec<RuleEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql_limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor FROM memories
             WHERE archived_at IS NULL ORDER BY key ASC LIMIT ?1;",
        )?;
        let mut rows = stmt.query(params![sql_limit])?;
        let mut list = Vec::with_capacity(limit.min(64));
        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(list)
    }

    /// Dump all memories (including archived) ordered by key.
    pub fn dump_all(&self) -> Result<Vec<RuleRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor, archived_at, archive_reason, kind FROM memories ORDER BY key ASC;",
        )?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                kind: row.get(5)?,
            });
        }
        Ok(list)
    }

    /// Dump only archived memories ordered by key.
    pub fn dump_archived(&self) -> Result<Vec<RuleRecord>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT key, val, anchor, archived_at, archive_reason, kind FROM memories WHERE archived_at IS NOT NULL ORDER BY key ASC;")?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                kind: row.get(5)?,
            });
        }
        Ok(list)
    }

    /// Build bounded, safe FTS5 terms from user input without breaking Porter stemming.
    fn sanitized_fts_terms(raw: &str) -> Vec<String> {
        const STOP_WORDS: &[&str] = &[
            "a", "an", "and", "are", "as", "at", "by", "for", "from", "in", "of", "on", "or",
            "out", "the", "to", "with", "con", "de", "del", "el", "en", "la", "las", "los", "para",
            "por", "que", "un", "una",
        ];
        let terms: Vec<(String, bool)> = raw
            .split(|c: char| {
                c.is_whitespace() || (c.is_ascii_punctuation() && c != '_' && c != '-' && c != '*')
            })
            .filter_map(|token| {
                let is_prefix = token.ends_with('*');
                let trimmed = if is_prefix {
                    token.trim_end_matches('*')
                } else {
                    token
                };
                let sanitized: String = trimmed
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .take(64)
                    .collect();

                if sanitized.is_empty() {
                    None
                } else if is_prefix {
                    Some((format!("\"{}\"*", sanitized), false))
                } else {
                    let stop_word = STOP_WORDS
                        .iter()
                        .any(|word| sanitized.eq_ignore_ascii_case(word));
                    Some((format!("\"{}\"", sanitized), stop_word))
                }
            })
            .take(16)
            .collect();
        let has_content_term = terms.iter().any(|(_, stop_word)| !stop_word);
        terms
            .into_iter()
            .filter(|(_, stop_word)| !has_content_term || !stop_word)
            .map(|(term, _)| term)
            .collect()
    }

    fn sanitize_fts_query(raw: &str) -> String {
        let terms = Self::sanitized_fts_terms(raw);
        if terms.is_empty() {
            let bounded: String = raw.chars().take(256).collect();
            format!("\"{}\"", bounded.replace('"', "\"\""))
        } else {
            terms.join(" ")
        }
    }

    fn relaxed_fts_query(raw: &str) -> Option<String> {
        let terms = Self::sanitized_fts_terms(raw);
        (terms.len() > 1).then(|| terms.join(" OR "))
    }

    fn find_fts(&self, fts_query: &str) -> Result<Vec<RuleRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason, m.kind
             FROM memories_fts
             JOIN memories m ON m.key = memories_fts.key
             WHERE memories_fts MATCH ?1
               AND memories_fts.rank MATCH 'bm25(8.0, 2.0, 4.0, 0.25, 1.0)'
             ORDER BY memories_fts.rank
             LIMIT 10;",
        )?;
        let mut rows = stmt.query(params![fts_query])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
                kind: row.get(5)?,
            });
        }
        Ok(results)
    }

    /// Search rules via BM25 full-text search.
    pub fn find(&self, query: &str) -> Result<Vec<RuleRecord>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = Self::sanitize_fts_query(trimmed);
        let strict_results = self.find_fts(&fts_query)?;
        if !strict_results.is_empty() {
            return Ok(strict_results);
        }

        let relaxed_results = match Self::relaxed_fts_query(trimmed) {
            Some(relaxed) if relaxed != fts_query => self.find_fts(&relaxed)?,
            _ => Vec::new(),
        };

        #[cfg(feature = "semantic-local")]
        if !Self::has_strong_lexical_evidence(trimmed, &relaxed_results)
            && let Ok(semantic_results) = self.find_semantic(trimmed)
            && !semantic_results.is_empty()
        {
            return Ok(Self::merge_semantic_results(
                semantic_results,
                relaxed_results,
            ));
        }

        Ok(relaxed_results)
    }

    #[cfg(feature = "semantic-local")]
    fn lexical_terms(raw: &str) -> Vec<String> {
        const STOP_WORDS: &[&str] = &[
            "and", "are", "con", "como", "del", "for", "las", "los", "para", "por", "que", "the",
            "una", "use", "using", "with",
        ];
        let mut terms = Vec::new();
        for token in raw.split(|c: char| !c.is_alphanumeric()) {
            let normalized = token.to_lowercase();
            if normalized.chars().count() >= 3
                && !STOP_WORDS.contains(&normalized.as_str())
                && !terms.contains(&normalized)
            {
                terms.push(normalized);
            }
        }
        terms
    }

    #[cfg(feature = "semantic-local")]
    fn has_strong_lexical_evidence(query: &str, results: &[RuleRecord]) -> bool {
        if results.is_empty() {
            return false;
        }
        let terms = Self::lexical_terms(query);
        if terms.len() <= 1 {
            return true;
        }
        results.iter().take(3).any(|rule| {
            let haystack = format!(
                "{} {} {} {}",
                rule.key,
                rule.val,
                rule.anchor.as_deref().unwrap_or_default(),
                rule.kind
            )
            .to_lowercase();
            let matched = terms
                .iter()
                .filter(|term| haystack.contains(term.as_str()))
                .count();
            matched >= 2 && matched * 2 >= terms.len()
        })
    }

    #[cfg(feature = "semantic-local")]
    fn merge_semantic_results(
        semantic: Vec<RuleRecord>,
        lexical: Vec<RuleRecord>,
    ) -> Vec<RuleRecord> {
        let mut seen = HashSet::with_capacity(10);
        semantic
            .into_iter()
            .chain(lexical)
            .filter(|rule| seen.insert(rule.key.clone()))
            .take(10)
            .collect()
    }
}
