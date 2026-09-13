use crate::error::Result;
use rusqlite::{Connection, TransactionBehavior, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub type RuleEntry = (String, String, Option<String>);
pub type SessionEntry = (i64, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRecord {
    pub key: String,
    pub val: String,
    pub anchor: Option<String>,
    pub archived_at: Option<i64>,
    pub archive_reason: Option<String>,
}

impl RuleRecord {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
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

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open SQLite database with performance-optimized PRAGMAs.
    pub fn open(db_path: &Path, need_write: bool) -> Result<Self> {
        let exists = db_path.exists();
        if !exists {
            if !need_write {
                return Err(crate::error::Error::NotInitialized);
            }
            if let Some(parent) = db_path.parent() {
                fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(db_path)?;
        Self::configure_conn(&conn, need_write)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database instance (ideal for isolated unit tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_conn(&conn, true)?;
        Ok(Self { conn })
    }

    fn configure_conn(conn: &Connection, need_write: bool) -> Result<()> {
        let mut pragma_stmt = "PRAGMA busy_timeout = 10000; PRAGMA mmap_size = 268435456;";
        if need_write {
            pragma_stmt = "PRAGMA busy_timeout = 10000; PRAGMA synchronous = NORMAL; PRAGMA mmap_size = 268435456;";
        }
        conn.execute_batch(pragma_stmt)?;

        let user_version: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap_or(0);

        if user_version >= 2 {
            // Fast path: schema and migrations already initialized to v2. Bypasses DDL and table scans completely!
            return Ok(());
        }

        let has_schema: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='memories');",
            [],
            |r| r.get(0),
        )?;

        if !has_schema {
            if !need_write {
                return Err(crate::error::Error::NotInitialized);
            }

            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA temp_store = MEMORY;",
            )?;

            conn.execute(
                "CREATE TABLE IF NOT EXISTS memories (
                    key TEXT PRIMARY KEY,
                    val TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    anchor TEXT,
                    archived_at INTEGER,
                    archive_reason TEXT
                ) WITHOUT ROWID;",
                [],
            )?;

            conn.execute(
                "CREATE TABLE IF NOT EXISTS sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    summary TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
                [],
            )?;

            conn.execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    key, val, anchor, archive_reason, tokenize='porter unicode61'
                );",
                [],
            )?;

            conn.execute("PRAGMA user_version = 2;", [])?;
        } else {
            // Migrations check: ensure anchor, archived_at, and archive_reason columns exist
            let has_anchor: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'anchor';",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
                > 0;

            if !has_anchor {
                let _ = conn.execute("ALTER TABLE memories ADD COLUMN anchor TEXT;", []);
            }

            let has_archived_at: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'archived_at';",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
                > 0;

            if !has_archived_at {
                let _ = conn.execute("ALTER TABLE memories ADD COLUMN archived_at INTEGER;", []);
            }

            let has_archive_reason: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'archive_reason';",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
                > 0;

            if !has_archive_reason {
                let _ = conn.execute("ALTER TABLE memories ADD COLUMN archive_reason TEXT;", []);
            }

            let _ = conn.execute("DROP TABLE IF EXISTS memories_fts;", []);
            let _ = conn.execute(
                "CREATE VIRTUAL TABLE memories_fts USING fts5(
                    key, val, anchor, archive_reason, tokenize='porter unicode61'
                );",
                [],
            );
            let _ = conn.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason) SELECT key, val, anchor, archive_reason FROM memories;",
                [],
            );
            conn.execute("PRAGMA user_version = 2;", [])?;
        }

        Ok(())
    }

    /// Set or update a key-value memory rule with optional repo-relative code anchor.
    pub fn set_with_anchor(&mut self, key: &str, val: &str, anchor: Option<&str>) -> Result<()> {
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
        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason) VALUES (?1, ?2, ?3, ?4, NULL, NULL)
             ON CONFLICT(key) DO UPDATE SET val = excluded.val, updated_at = excluded.updated_at, anchor = excluded.anchor, archived_at = NULL, archive_reason = NULL;",
            params![trimmed_key, trimmed_val, now, trimmed_anchor],
        )?;
        tx.execute(
            "DELETE FROM memories_fts WHERE key = ?1;",
            params![trimmed_key],
        )?;
        tx.execute(
            "INSERT INTO memories_fts (key, val, anchor, archive_reason) VALUES (?1, ?2, ?3, NULL);",
            params![trimmed_key, trimmed_val, trimmed_anchor],
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Set or update a key-value memory rule.
    pub fn set(&mut self, key: &str, val: &str) -> Result<()> {
        self.set_with_anchor(key, val, None)
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
            let _ = tx.execute(
                "DELETE FROM memories_fts WHERE key = ?1;",
                params![trimmed_key],
            );
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason) 
                 SELECT key, val, anchor, archive_reason FROM memories WHERE key = ?1;",
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
            let _ = tx.execute(
                "DELETE FROM memories_fts WHERE key = ?1;",
                params![trimmed_key],
            );
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason) 
                 SELECT key, val, anchor, archive_reason FROM memories WHERE key = ?1;",
                params![trimmed_key],
            )?;
        }

        tx.commit()?;
        Ok(changes > 0)
    }

    /// Retrieve a memory rule record with anchor and archival metadata.
    pub fn get_entry(&self, key: &str) -> Result<Option<RuleRecord>> {
        if key.trim().is_empty() {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT key, val, anchor, archived_at, archive_reason FROM memories WHERE key = ?1 LIMIT 1;")?;
        let mut rows = stmt.query(params![key.trim()])?;

        if let Some(row) = rows.next()? {
            Ok(Some(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
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

    /// Delete a key-value memory rule. Returns true if key was deleted.
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

    /// Dump all memories (including archived) ordered by key.
    pub fn dump_all(&self) -> Result<Vec<RuleRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor, archived_at, archive_reason FROM memories ORDER BY key ASC;",
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
            });
        }
        Ok(list)
    }

    /// Dump only archived memories ordered by key.
    pub fn dump_archived(&self) -> Result<Vec<RuleRecord>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT key, val, anchor, archived_at, archive_reason FROM memories WHERE archived_at IS NOT NULL ORDER BY key ASC;")?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
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

    /// Search rules via BM25 full-text search with fallback to LIKE pattern if FTS fails.
    pub fn find(&self, query: &str) -> Result<Vec<RuleRecord>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = Self::sanitize_fts_query(trimmed);
        let fts_res = self.conn.prepare(
            "SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason 
             FROM memories_fts 
             JOIN memories m ON m.key = memories_fts.key 
             WHERE memories_fts MATCH ?1 
             ORDER BY rank LIMIT 10;",
        );

        if let Ok(results) = fts_res.and_then(|mut stmt| {
            let mut rows = stmt.query(params![fts_query])?;
            let mut results = Vec::new();
            while let Some(row) = rows.next()? {
                results.push(RuleRecord {
                    key: row.get(0)?,
                    val: row.get(1)?,
                    anchor: row.get(2)?,
                    archived_at: row.get(3)?,
                    archive_reason: row.get(4)?,
                });
            }
            Ok::<_, rusqlite::Error>(results)
        }) {
            return Ok(results);
        }

        // Fallback to substring matching only if FTS query failed to execute
        let escaped = trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like_pattern = format!("%{}%", escaped);
        let mut stmt = self.conn.prepare(
            "SELECT key, val, anchor, archived_at, archive_reason 
             FROM memories 
             WHERE key LIKE ?1 ESCAPE '\\' 
                OR val LIKE ?1 ESCAPE '\\' 
                OR anchor LIKE ?1 ESCAPE '\\' 
                OR archive_reason LIKE ?1 ESCAPE '\\' 
             ORDER BY key ASC LIMIT 10;",
        )?;
        let mut rows = stmt.query(params![like_pattern])?;
        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            results.push(RuleRecord {
                key: row.get(0)?,
                val: row.get(1)?,
                anchor: row.get(2)?,
                archived_at: row.get(3)?,
                archive_reason: row.get(4)?,
            });
        }

        Ok(results)
    }

    /// Record a session checkpoint with automatic ring-buffer pruning (keeps latest 20).
    pub fn session_add(&mut self, summary: &str) -> Result<i64> {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            return Err(crate::error::Error::Usage(
                "Session summary cannot be empty".into(),
            ));
        }

        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO sessions (summary, created_at) VALUES (?1, ?2);",
            params![trimmed, now],
        )?;
        let id = tx.last_insert_rowid();
        // Ring buffer: keep only the 20 most recent sessions to prevent unbounded storage bloat
        tx.execute(
            "DELETE FROM sessions WHERE id NOT IN (SELECT id FROM sessions ORDER BY id DESC LIMIT 20);",
            [],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// List recent session checkpoints.
    pub fn session_list(&self, limit: usize) -> Result<Vec<SessionEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, summary FROM sessions ORDER BY id DESC LIMIT ?1;")?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?));
        }
        Ok(list)
    }

    /// Export dense context (rules and recent sessions).
    pub fn context(&self) -> Result<(Vec<RuleEntry>, Vec<SessionEntry>)> {
        let rules = self.dump()?;
        let sessions = self.session_list(3)?;
        Ok((rules, sessions))
    }

    /// Parse plain-text rules (e.g. from .agent-rules). Supports active and [archived] rules.
    pub fn parse_rules_text(content: &str) -> Vec<RuleRecord> {
        let mut rules = Vec::new();
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

            let (is_archived, line_to_parse) =
                if let Some(stripped) = trimmed.strip_prefix("[archived]") {
                    (true, stripped.trim())
                } else if let Some(stripped) = trimmed.strip_prefix("[deprecated]") {
                    (true, stripped.trim())
                } else {
                    (false, trimmed)
                };

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
            });
        }
        rules
    }

    /// Export all rules formatted as deterministic plain text sorted by key (active first, then archived).
    pub fn export_rules_text(&self) -> Result<String> {
        let all_rules = self.dump_all()?;
        let mut out = String::new();
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
            if let Some(anchor) = &r.anchor {
                out.push_str(&format!("{} = {} (@ {})\n", r.key, r.val, anchor));
            } else {
                out.push_str(&format!("{} = {}\n", r.key, r.val));
            }
        }

        if !archived.is_empty() {
            out.push_str("\n# Archived Rules\n");
            for r in archived {
                let mut line = format!("[archived] {} = {}", r.key, r.val);
                if let Some(anchor) = &r.anchor {
                    line.push_str(&format!(" (@ {})", anchor));
                }
                if let Some(reason) = &r.archive_reason {
                    line.push_str(&format!(" --reason: {}", reason));
                }
                line.push('\n');
                out.push_str(&line);
            }
        }

        Ok(out)
    }

    /// Export current rules to a file on disk.
    pub fn export_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }
        let content = self.export_rules_text()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Reconcile SQLite database with a plain-text rules file (.agent-rules).
    pub fn sync_with_file(&mut self, path: &Path) -> Result<SyncReport> {
        if !path.exists() {
            self.export_to_file(path)?;
            let total = self.dump_all()?.len();
            return Ok(SyncReport {
                path: path.to_path_buf(),
                imported: 0,
                total,
                file_created: true,
                file_updated: false,
            });
        }

        let content = fs::read_to_string(path)?;
        let rules = Self::parse_rules_text(&content);

        struct ParsedEntry<'a> {
            val: &'a str,
            anchor: Option<&'a str>,
            archived_at: Option<i64>,
            archive_reason: Option<&'a str>,
        }

        // Deduplicate in memory: if multiple conflict markers or duplicate lines exist, last one wins
        let mut unique_rules: std::collections::BTreeMap<&str, ParsedEntry> =
            std::collections::BTreeMap::new();
        for r in &rules {
            unique_rules.insert(
                r.key.as_str(),
                ParsedEntry {
                    val: r.val.as_str(),
                    anchor: r.anchor.as_deref(),
                    archived_at: r.archived_at,
                    archive_reason: r.archive_reason.as_deref(),
                },
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM memories;", [])?;
        tx.execute("DELETE FROM memories_fts;", [])?;

        let now = now_epoch();
        {
            let mut insert_mem = tx.prepare_cached(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6);",
            )?;
            let mut insert_fts = tx.prepare_cached(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason) VALUES (?1, ?2, ?3, ?4);",
            )?;

            for (key, entry) in unique_rules {
                let arc_at = entry.archived_at.map(|_| now);
                insert_mem.execute(params![
                    key,
                    entry.val,
                    now,
                    entry.anchor,
                    arc_at,
                    entry.archive_reason
                ])?;
                insert_fts.execute(params![key, entry.val, entry.anchor, entry.archive_reason])?;
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
        let total = self.dump_all()?.len();
        Ok(SyncReport {
            path: path.to_path_buf(),
            imported: 0,
            total,
            file_created: !exists,
            file_updated: exists,
        })
    }

    /// Retrieve store diagnostics and health metrics.
    pub fn stats(&self, path: &Path) -> Result<StoreStats> {
        let journal_mode: String = self
            .conn
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))?;
        let user_version: u32 = self
            .conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))?;
        let rules_count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM memories;", [], |r| r.get(0))?;
        let active_rules_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived_at IS NULL;",
            [],
            |r| r.get(0),
        )?;
        let archived_rules_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived_at IS NOT NULL;",
            [],
            |r| r.get(0),
        )?;
        let sessions_count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM sessions;", [], |r| r.get(0))?;

        Ok(StoreStats {
            db_path: path.to_path_buf(),
            journal_mode,
            user_version,
            rules_count,
            active_rules_count,
            archived_rules_count,
            sessions_count,
        })
    }
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
