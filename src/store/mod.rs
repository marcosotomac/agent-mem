use crate::error::Result;
use rusqlite::{Connection, TransactionBehavior, params};
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
pub mod session;
pub mod sync;

pub struct Store {
    pub(crate) conn: Connection,
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

    /// Atomically set a memory and, optionally, one directed relation.
    pub fn set_entry_with_relation(
        &mut self,
        key: &str,
        val: &str,
        anchor: Option<&str>,
        kind: Option<&str>,
        relation: Option<(&str, &str)>,
    ) -> Result<()> {
        let trimmed_key = key.trim();
        let trimmed_val = val.trim();
        if trimmed_key.is_empty() {
            return Err(crate::error::Error::Usage(
                "Memory key cannot be empty".into(),
            ));
        }
        if trimmed_val.is_empty() {
            return Err(crate::error::Error::Usage(
                "Memory value cannot be empty".into(),
            ));
        }

        let trimmed_anchor = anchor.map(|a| a.trim()).filter(|a| !a.is_empty());
        let effective_kind = kind
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| infer_kind(trimmed_key));
        let relation = match relation {
            Some((rel_type, target)) => {
                let rel_type = rel_type.trim();
                let target = target.trim();
                if rel_type.is_empty() || target.is_empty() {
                    return Err(crate::error::Error::Usage(
                        "Relation type and target cannot be empty".into(),
                    ));
                }
                Some((rel_type, target))
            }
            None => None,
        };

        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)
             ON CONFLICT(key) DO UPDATE SET val = excluded.val, updated_at = excluded.updated_at, anchor = excluded.anchor, archived_at = NULL, archive_reason = NULL, kind = excluded.kind;",
            params![trimmed_key, trimmed_val, now, trimmed_anchor, effective_kind],
        )?;
        tx.execute(
            "DELETE FROM memories_fts WHERE key = ?1;",
            params![trimmed_key],
        )?;
        tx.execute(
            "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) VALUES (?1, ?2, ?3, NULL, ?4);",
            params![trimmed_key, trimmed_val, trimmed_anchor, effective_kind],
        )?;
        if let Some((rel_type, target)) = relation {
            tx.execute(
                "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
                params![trimmed_key, rel_type, target, now],
            )?;
        }
        tx.commit()?;

        Ok(())
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
                "DELETE FROM relations WHERE source_key = ?1 OR target_key = ?1;",
                params![key.trim()],
            )?;
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

    /// Build a safe FTS5 query string from user input without breaking Porter stemming.
    fn sanitize_fts_query(raw: &str) -> String {
        let clean_tokens: Vec<String> = raw
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
                    .collect();

                if sanitized.is_empty() {
                    None
                } else if is_prefix {
                    Some(format!("\"{}\"*", sanitized))
                } else {
                    Some(format!("\"{}\"", sanitized))
                }
            })
            .collect();

        if clean_tokens.is_empty() {
            format!("\"{}\"", raw.replace('"', "\"\""))
        } else {
            clean_tokens.join(" ")
        }
    }

    /// Search rules via BM25 full-text search.
    pub fn find(&self, query: &str) -> Result<Vec<RuleRecord>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = Self::sanitize_fts_query(trimmed);
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason, m.kind 
             FROM memories_fts 
             JOIN memories m ON m.key = memories_fts.key 
             WHERE memories_fts MATCH ?1 
             ORDER BY rank LIMIT 10;",
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
}
