use crate::error::Result;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open SQLite database with performance-optimized PRAGMAs.
    pub fn open(db_path: &Path, need_write: bool) -> Result<Self> {
        let exists = db_path.exists();
        if !exists {
            if let Some(parent) = db_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
        }

        let conn = Connection::open(db_path)?;
        Self::configure_conn(&conn, exists, need_write)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database instance (ideal for isolated unit tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_conn(&conn, false, true)?;
        Ok(Self { conn })
    }

    fn configure_conn(conn: &Connection, exists: bool, need_write: bool) -> Result<()> {
        if !exists {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA temp_store = MEMORY;
                 PRAGMA busy_timeout = 3000;
                 PRAGMA mmap_size = 268435456;",
            )?;

            conn.execute(
                "CREATE TABLE IF NOT EXISTS memories (
                    key TEXT PRIMARY KEY,
                    val TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
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

            let _ = conn.execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    key, val, tokenize='porter unicode61'
                );",
                [],
            );
        } else {
            let mut pragma_stmt = "PRAGMA busy_timeout = 3000; PRAGMA mmap_size = 268435456;";
            if need_write {
                pragma_stmt = "PRAGMA busy_timeout = 3000; PRAGMA synchronous = NORMAL; PRAGMA mmap_size = 268435456;";
            }
            conn.execute_batch(pragma_stmt)?;
        }
        Ok(())
    }

    /// Set or update a key-value memory rule.
    pub fn set(&self, key: &str, val: &str) -> Result<()> {
        let now = now_epoch();
        self.conn.execute(
            "INSERT INTO memories (key, val, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET val = excluded.val, updated_at = excluded.updated_at;",
            params![key, val, now],
        )?;

        let _ = self.conn.execute("DELETE FROM memories_fts WHERE key = ?1;", params![key]);
        let _ = self.conn.execute(
            "INSERT INTO memories_fts (key, val) VALUES (?1, ?2);",
            params![key, val],
        );

        Ok(())
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
    pub fn del(&self, key: &str) -> Result<bool> {
        let changes = self.conn.execute("DELETE FROM memories WHERE key = ?1;", params![key])?;
        let _ = self.conn.execute("DELETE FROM memories_fts WHERE key = ?1;", params![key]);
        Ok(changes > 0)
    }

    /// Dump all memories ordered by key.
    pub fn dump(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT key, val FROM memories ORDER BY key ASC;")?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?));
        }
        Ok(list)
    }

    /// Build a safe FTS5 query string from user input.
    fn sanitize_fts_query(raw: &str) -> String {
        let clean_tokens: Vec<String> = raw
            .split_whitespace()
            .map(|token| {
                let sanitized: String = token.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
                format!("\"{}\"*", sanitized)
            })
            .filter(|t| t != "\"\"*")
            .collect();

        if clean_tokens.is_empty() {
            format!("\"{}\"", raw.replace('"', "\"\""))
        } else {
            clean_tokens.join(" ")
        }
    }

    /// Search rules via BM25 full-text search with fallback to LIKE pattern.
    pub fn find(&self, query: &str) -> Result<Vec<(String, String)>> {
        let fts_query = Self::sanitize_fts_query(query);
        let fts_res = self.conn.prepare(
            "SELECT key, val FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY rank LIMIT 10;",
        );

        if let Ok(mut stmt) = fts_res {
            if let Ok(mut rows) = stmt.query(params![fts_query]) {
                let mut results = Vec::new();
                while let Ok(Some(row)) = rows.next() {
                    if let (Ok(key), Ok(val)) = (row.get(0), row.get(1)) {
                        results.push((key, val));
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Fallback to substring matching
        let like_pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT key, val FROM memories WHERE key LIKE ?1 OR val LIKE ?1 ORDER BY key ASC LIMIT 10;",
        )?;
        let mut rows = stmt.query(params![like_pattern])?;
        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }

        Ok(results)
    }

    /// Record a session checkpoint.
    pub fn session_add(&self, summary: &str) -> Result<i64> {
        let now = now_epoch();
        self.conn.execute(
            "INSERT INTO sessions (summary, created_at) VALUES (?1, ?2);",
            params![summary, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List recent session checkpoints.
    pub fn session_list(&self, limit: usize) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, summary FROM sessions ORDER BY id DESC LIMIT ?1;")?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut list = Vec::new();

        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?));
        }
        Ok(list)
    }

    /// Export dense context (rules and recent sessions).
    pub fn context(&self) -> Result<(Vec<(String, String)>, Vec<(i64, String)>)> {
        let rules = self.dump()?;
        let sessions = self.session_list(3)?;
        Ok((rules, sessions))
    }
}
