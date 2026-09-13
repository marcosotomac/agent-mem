use crate::error::Result;
use rusqlite::{params, Connection, TransactionBehavior};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub type RuleEntry = (String, String, Option<String>);
pub type SessionEntry = (i64, String);

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
                let _ = fs::create_dir_all(parent);
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
        let mut pragma_stmt = "PRAGMA busy_timeout = 3000; PRAGMA mmap_size = 268435456;";
        if need_write {
            pragma_stmt = "PRAGMA busy_timeout = 3000; PRAGMA synchronous = NORMAL; PRAGMA mmap_size = 268435456;";
        }
        conn.execute_batch(pragma_stmt)?;

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
                    anchor TEXT
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
                    key, val, anchor, tokenize='porter unicode61'
                );",
                [],
            )?;

            conn.execute("PRAGMA user_version = 1;", [])?;
        } else {
            // Migration check: ensure anchor column exists in memories table
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
                let _ = conn.execute("DROP TABLE IF EXISTS memories_fts;", []);
                let _ = conn.execute(
                    "CREATE VIRTUAL TABLE memories_fts USING fts5(
                        key, val, anchor, tokenize='porter unicode61'
                    );",
                    [],
                );
                let _ = conn.execute(
                    "INSERT INTO memories_fts (key, val, anchor) SELECT key, val, anchor FROM memories;",
                    [],
                );
            }
        }

        Ok(())
    }

    /// Set or update a key-value memory rule with optional repo-relative code anchor.
    pub fn set_with_anchor(&mut self, key: &str, val: &str, anchor: Option<&str>) -> Result<()> {
        let now = now_epoch();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO memories (key, val, updated_at, anchor) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET val = excluded.val, updated_at = excluded.updated_at, anchor = excluded.anchor;",
            params![key, val, now, anchor],
        )?;
        tx.execute("DELETE FROM memories_fts WHERE key = ?1;", params![key])?;
        tx.execute(
            "INSERT INTO memories_fts (key, val, anchor) VALUES (?1, ?2, ?3);",
            params![key, val, anchor],
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Set or update a key-value memory rule.
    pub fn set(&mut self, key: &str, val: &str) -> Result<()> {
        self.set_with_anchor(key, val, None)
    }

    /// Retrieve a key-value memory rule with anchor.
    pub fn get_entry(&self, key: &str) -> Result<Option<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare("SELECT val, anchor FROM memories WHERE key = ?1 LIMIT 1;")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            let anchor: Option<String> = row.get(1)?;
            Ok(Some((val, anchor)))
        } else {
            Ok(None)
        }
    }

    /// Retrieve a key-value memory rule.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT val FROM memories WHERE key = ?1 LIMIT 1;")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Delete a key-value memory rule. Returns true if key was deleted.
    pub fn del(&mut self, key: &str) -> Result<bool> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changes = tx.execute("DELETE FROM memories WHERE key = ?1;", params![key])?;
        if changes > 0 {
            tx.execute("DELETE FROM memories_fts WHERE key = ?1;", params![key])?;
        }
        tx.commit()?;
        Ok(changes > 0)
    }

    /// Dump all memories ordered by key.
    pub fn dump(&self) -> Result<Vec<RuleEntry>> {
        let mut stmt = self.conn.prepare("SELECT key, val, anchor FROM memories ORDER BY key ASC;")?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(list)
    }

    /// Build a safe FTS5 query string from user input without breaking Porter stemming.
    fn sanitize_fts_query(raw: &str) -> String {
        let clean_tokens: Vec<String> = raw
            .split(|c: char| c.is_whitespace() || (c.is_ascii_punctuation() && c != '_' && c != '-' && c != '*'))
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
    pub fn find(&self, query: &str) -> Result<Vec<RuleEntry>> {
        let fts_query = Self::sanitize_fts_query(query);
        let fts_res = self.conn.prepare(
            "SELECT m.key, m.val, m.anchor FROM memories_fts JOIN memories m ON m.key = memories_fts.key WHERE memories_fts MATCH ?1 ORDER BY rank LIMIT 10;",
        );

        if let Ok(results) = fts_res.and_then(|mut stmt| {
            let mut rows = stmt.query(params![fts_query])?;
            let mut results = Vec::new();
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let val: String = row.get(1)?;
                let anchor: Option<String> = row.get(2)?;
                results.push((key, val, anchor));
            }
            Ok::<_, rusqlite::Error>(results)
        }) {
            return Ok(results);
        }

        // Fallback to substring matching only if FTS query failed to execute
        let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let like_pattern = format!("%{}%", escaped);
        let mut stmt = self.conn.prepare(
            "SELECT key, val, anchor FROM memories WHERE key LIKE ?1 ESCAPE '\\' OR val LIKE ?1 ESCAPE '\\' OR anchor LIKE ?1 ESCAPE '\\' ORDER BY key ASC LIMIT 10;",
        )?;
        let mut rows = stmt.query(params![like_pattern])?;
        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }

        Ok(results)
    }

    /// Record a session checkpoint with automatic ring-buffer pruning (keeps latest 20).
    pub fn session_add(&mut self, summary: &str) -> Result<i64> {
        let now = now_epoch();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO sessions (summary, created_at) VALUES (?1, ?2);",
            params![summary, now],
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
        let mut stmt = self.conn.prepare("SELECT id, summary FROM sessions ORDER BY id DESC LIMIT ?1;")?;
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
}
