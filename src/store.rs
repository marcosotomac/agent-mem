use crate::error::Result;
use rusqlite::{Connection, TransactionBehavior, params};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static EXPORT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

pub fn infer_kind(key: &str) -> &'static str {
    let lower = key.to_lowercase();
    if lower.starts_with("decision/")
        || lower.starts_with("adr/")
        || lower.starts_with("decision:")
        || lower.starts_with("adr:")
    {
        "decision"
    } else if lower.starts_with("gotcha/")
        || lower.starts_with("bug/")
        || lower.starts_with("gotcha:")
        || lower.starts_with("trap/")
        || lower.starts_with("postmortem/")
    {
        "gotcha"
    } else if lower.starts_with("pattern/") || lower.starts_with("pattern:") {
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

        let mut conn = Connection::open(db_path)?;
        Self::configure_conn(&mut conn, need_write)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database instance (ideal for isolated unit tests).
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure_conn(&mut conn, true)?;
        Ok(Self { conn })
    }

    fn configure_conn(conn: &mut Connection, need_write: bool) -> Result<()> {
        conn.execute_batch(
            "PRAGMA busy_timeout = 10000;
             PRAGMA synchronous = NORMAL;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;",
        )?;

        let user_version: u32 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;

        if user_version >= 3 {
            // Fast path: schema and migrations already initialized to v3. Bypasses DDL and table scans completely!
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

            conn.execute_batch("PRAGMA journal_mode = WAL;")?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS memories (
                    key TEXT PRIMARY KEY,
                    val TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    anchor TEXT,
                    archived_at INTEGER,
                    archive_reason TEXT,
                    kind TEXT NOT NULL DEFAULT 'rule'
                ) WITHOUT ROWID;
                CREATE INDEX IF NOT EXISTS idx_memories_anchor ON memories(anchor);
                CREATE TABLE IF NOT EXISTS relations (
                    source_key TEXT NOT NULL,
                    rel_type TEXT NOT NULL,
                    target_key TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    PRIMARY KEY (source_key, rel_type, target_key)
                ) WITHOUT ROWID;
                CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_key, rel_type);
                CREATE TABLE IF NOT EXISTS sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    summary TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    key, val, anchor, archive_reason, kind, tokenize='porter unicode61'
                );",
            )?;
            tx.execute("PRAGMA user_version = 3;", [])?;
            tx.commit()?;
        } else {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // Migrations check: ensure anchor, archived_at, archive_reason, and kind columns exist
            let has_anchor: bool = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'anchor';",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0;

            if !has_anchor {
                tx.execute("ALTER TABLE memories ADD COLUMN anchor TEXT;", [])?;
            }

            let has_archived_at: bool = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'archived_at';",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0;

            if !has_archived_at {
                tx.execute("ALTER TABLE memories ADD COLUMN archived_at INTEGER;", [])?;
            }

            let has_archive_reason: bool = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'archive_reason';",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0;

            if !has_archive_reason {
                tx.execute("ALTER TABLE memories ADD COLUMN archive_reason TEXT;", [])?;
            }

            let has_kind: bool = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'kind';",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0;

            if !has_kind {
                tx.execute(
                    "ALTER TABLE memories ADD COLUMN kind TEXT NOT NULL DEFAULT 'rule';",
                    [],
                )?;
            }

            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_memories_anchor ON memories(anchor);",
                [],
            )?;

            tx.execute(
                "CREATE TABLE IF NOT EXISTS relations (
                    source_key TEXT NOT NULL,
                    rel_type TEXT NOT NULL,
                    target_key TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    PRIMARY KEY (source_key, rel_type, target_key)
                ) WITHOUT ROWID;",
                [],
            )?;

            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_key, rel_type);",
                [],
            )?;

            tx.execute("DROP TABLE IF EXISTS memories_fts;", [])?;
            tx.execute(
                "CREATE VIRTUAL TABLE memories_fts USING fts5(
                    key, val, anchor, archive_reason, kind, tokenize='porter unicode61'
                );",
                [],
            )?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) SELECT key, val, anchor, archive_reason, kind FROM memories;",
                [],
            )?;
            tx.execute("PRAGMA user_version = 3;", [])?;
            tx.commit()?;
        }

        Ok(())
    }

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

    /// Create a directed relation between two memories (e.g. source mitigates target, or source depends_on target).
    pub fn relate(&mut self, source_key: &str, rel_type: &str, target_key: &str) -> Result<()> {
        let s = source_key.trim();
        let r = rel_type.trim();
        let t = target_key.trim();
        if s.is_empty() || r.is_empty() || t.is_empty() {
            return Err(crate::error::Error::Usage(
                "Relation source, type, and target cannot be empty".into(),
            ));
        }
        let now = now_epoch();
        self.conn.execute(
            "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
            params![s, r, t, now],
        )?;
        Ok(())
    }

    /// Delete a directed relation between two memories.
    pub fn unrelate(&mut self, source_key: &str, rel_type: &str, target_key: &str) -> Result<bool> {
        let changes = self.conn.execute(
            "DELETE FROM relations WHERE source_key = ?1 AND rel_type = ?2 AND target_key = ?3;",
            params![source_key.trim(), rel_type.trim(), target_key.trim()],
        )?;
        Ok(changes > 0)
    }

    /// Retrieve all direct relations originating from a source key.
    pub fn get_relations(&self, key: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT rel_type, target_key FROM relations WHERE source_key = ?1 ORDER BY rel_type, target_key;",
        )?;
        let mut rows = stmt.query(params![key.trim()])?;
        let mut list = Vec::new();
        while let Some(row) = rows.next()? {
            list.push((row.get(0)?, row.get(1)?));
        }
        Ok(list)
    }

    /// Retrieve all relations across the project.
    pub fn get_all_relations(&self) -> Result<Vec<RelationRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_key, rel_type, target_key, created_at FROM relations ORDER BY source_key, rel_type, target_key;",
        )?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();
        while let Some(row) = rows.next()? {
            list.push(RelationRecord {
                source_key: row.get(0)?,
                rel_type: row.get(1)?,
                target_key: row.get(2)?,
                created_at: row.get(3)?,
            });
        }
        Ok(list)
    }

    fn get_relations_touching(&self, keys: &[&str], limit: usize) -> Result<Vec<RelationRecord>> {
        if keys.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let sql_limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_key, rel_type, target_key, created_at
             FROM relations
             WHERE source_key = ?1 OR target_key = ?1
             ORDER BY source_key, rel_type, target_key
             LIMIT ?2;",
        )?;
        let mut relations = Vec::with_capacity(limit.min(64));
        for key in keys {
            let mut rows = stmt.query(params![key, sql_limit])?;
            while let Some(row) = rows.next()? {
                relations.push(RelationRecord {
                    source_key: row.get(0)?,
                    rel_type: row.get(1)?,
                    target_key: row.get(2)?,
                    created_at: row.get(3)?,
                });
            }
        }

        relations.sort_unstable_by(|a, b| {
            (&a.source_key, &a.rel_type, &a.target_key).cmp(&(
                &b.source_key,
                &b.rel_type,
                &b.target_key,
            ))
        });
        relations.dedup_by(|a, b| {
            a.source_key == b.source_key && a.rel_type == b.rel_type && a.target_key == b.target_key
        });
        relations.truncate(limit);
        Ok(relations)
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

    fn archive_zombies_tx(&mut self, zombies: &[ZombieReport]) -> Result<()> {
        if zombies.is_empty() {
            return Ok(());
        }
        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        for z in zombies {
            let reason = format!("file_deleted: {}", z.missing_paths.join(", "));
            tx.execute(
                "UPDATE memories SET archived_at = ?1, archive_reason = ?2, updated_at = ?3 WHERE key = ?4;",
                params![now, reason, now, z.key],
            )?;
            tx.execute("DELETE FROM memories_fts WHERE key = ?1;", params![z.key])?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) 
                 SELECT key, val, anchor, archive_reason, kind FROM memories WHERE key = ?1;",
                params![z.key],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Inspect all active anchored memories against project filesystem.
    /// If an anchored file no longer exists, archives the memory with reason `file_deleted: <paths>`.
    /// If dry_run is true, returns which memories would be archived without mutating the database.
    pub fn clean_zombies(
        &mut self,
        project_root: &Path,
        dry_run: bool,
    ) -> Result<Vec<ZombieReport>> {
        let active = self.dump_active()?;
        let mut zombies = Vec::new();

        for m in active {
            if let Some(anchor_raw) = m.anchor {
                let paths = extract_anchor_paths(&anchor_raw);
                if paths.is_empty() {
                    continue;
                }

                let missing: Vec<String> = paths
                    .into_iter()
                    .filter(|p| !project_root.join(p).exists())
                    .collect();

                if !missing.is_empty() {
                    zombies.push(ZombieReport {
                        key: m.key.clone(),
                        missing_paths: missing,
                        archived: !dry_run,
                    });
                }
            }
        }

        if !dry_run {
            self.archive_zombies_tx(&zombies)?;
        }

        Ok(zombies)
    }

    /// Prune active memories whose anchors reference any of the specified deleted files.
    pub fn clean_deleted_files(
        &mut self,
        deleted_files: &[String],
        dry_run: bool,
    ) -> Result<Vec<ZombieReport>> {
        if deleted_files.is_empty() {
            return Ok(Vec::new());
        }

        let active = self.dump_active()?;
        let mut zombies = Vec::new();

        for m in active {
            if let Some(anchor_raw) = m.anchor {
                let paths = extract_anchor_paths(&anchor_raw);
                if paths.is_empty() {
                    continue;
                }

                let missing: Vec<String> = paths
                    .into_iter()
                    .filter(|p| deleted_files.iter().any(|d| d == p))
                    .collect();

                if !missing.is_empty() {
                    zombies.push(ZombieReport {
                        key: m.key.clone(),
                        missing_paths: missing,
                        archived: !dry_run,
                    });
                }
            }
        }

        if !dry_run {
            self.archive_zombies_tx(&zombies)?;
        }

        Ok(zombies)
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

    /// Atomically capture a git commit: optionally upsert an entity, relations, and append to sessions ring buffer in a single transaction.
    pub fn capture_commit(
        &mut self,
        entity: Option<(&str, &str, Option<&str>, Option<&str>)>,
        relations: &[(&str, &str, &str)],
        session_summary: &str,
    ) -> Result<Option<i64>> {
        if let Some((key, val, _, _)) = entity {
            if key.trim().is_empty() {
                return Err(crate::error::Error::Usage(
                    "Memory key cannot be empty".into(),
                ));
            }
            if val.trim().is_empty() {
                return Err(crate::error::Error::Usage(
                    "Memory value cannot be empty".into(),
                ));
            }
        }
        for (source, rel_type, target) in relations {
            if source.trim().is_empty() || rel_type.trim().is_empty() || target.trim().is_empty() {
                return Err(crate::error::Error::Usage(
                    "Relation source, type, and target cannot be empty".into(),
                ));
            }
        }

        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some((key, val, anchor, kind)) = entity {
            let trimmed_key = key.trim();
            let trimmed_val = val.trim();
            let trimmed_anchor = anchor.map(|a| a.trim()).filter(|a| !a.is_empty());
            let effective_kind = kind
                .map(|k| k.trim())
                .filter(|k| !k.is_empty())
                .unwrap_or_else(|| infer_kind(trimmed_key));

            tx.execute(
                "INSERT INTO memories (key, val, updated_at, anchor, archived_at, archive_reason, kind)
                 VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)
                 ON CONFLICT(key) DO UPDATE SET
                     val = excluded.val,
                     updated_at = excluded.updated_at,
                     anchor = excluded.anchor,
                     archived_at = NULL,
                     archive_reason = NULL,
                     kind = excluded.kind;",
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
        }

        for (source, rel_type, target) in relations {
            let s = source.trim();
            let r = rel_type.trim();
            let t = target.trim();
            tx.execute(
                "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
                params![s, r, t, now],
            )?;
        }

        let mut session_id = None;
        let trimmed_session = session_summary.trim();
        if !trimmed_session.is_empty() {
            tx.execute(
                "INSERT INTO sessions (summary, created_at) VALUES (?1, ?2);",
                params![trimmed_session, now],
            )?;
            session_id = Some(tx.last_insert_rowid());
            tx.execute(
                "DELETE FROM sessions WHERE id NOT IN (SELECT id FROM sessions ORDER BY id DESC LIMIT 20);",
                [],
            )?;
        }

        tx.commit()?;
        Ok(session_id)
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

    /// Retrieve context filtered by code anchor or topic prefix, expanding connected graph relations.
    pub fn context_filtered(
        &self,
        anchor: Option<&str>,
        topic: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<RuleRecord>, Vec<RelationRecord>, Vec<SessionEntry>)> {
        let max_limit = if limit == 0 { 20 } else { limit };

        let rules = if let Some(a) = anchor.filter(|s| !s.trim().is_empty()) {
            let clean_a = a.trim();
            // Match memories directly anchored to this path and 1-hop related memories
            let mut stmt = self.conn.prepare_cached(
                "WITH direct_anchors AS (
                    SELECT key, val, anchor, archived_at, archive_reason, kind
                    FROM memories
                    WHERE (anchor = ?1 OR anchor LIKE ?2 OR ?1 LIKE anchor || '%')
                      AND archived_at IS NULL
                ),
                related AS (
                    SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason, m.kind
                    FROM relations r
                    JOIN memories m ON r.target_key = m.key
                    WHERE r.source_key IN (SELECT key FROM direct_anchors)
                      AND m.archived_at IS NULL
                )
                SELECT key, val, anchor, archived_at, archive_reason, kind FROM direct_anchors
                UNION
                SELECT key, val, anchor, archived_at, archive_reason, kind FROM related
                LIMIT ?3;",
            )?;
            let like_pattern = format!("{}%", clean_a);
            let mut rows = stmt.query(params![clean_a, like_pattern, max_limit as i64])?;
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
            list
        } else if let Some(t) = topic.filter(|s| !s.trim().is_empty()) {
            let clean_t = t.trim();
            let pattern = format!("{}%", clean_t);
            let mut stmt = self.conn.prepare_cached(
                "SELECT key, val, anchor, archived_at, archive_reason, kind
                 FROM memories
                 WHERE (key LIKE ?1 OR key = ?2) AND archived_at IS NULL
                 ORDER BY key ASC LIMIT ?3;",
            )?;
            let mut rows = stmt.query(params![pattern, clean_t, max_limit as i64])?;
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
            list
        } else {
            let mut stmt = self.conn.prepare_cached(
                "SELECT key, val, anchor, archived_at, archive_reason, kind
                 FROM memories
                 WHERE archived_at IS NULL
                 ORDER BY key ASC LIMIT ?1;",
            )?;
            let mut rows = stmt.query(params![max_limit as i64])?;
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
            list
        };

        // Fetch relations relevant to the retrieved rules
        let mut rels = Vec::new();
        if !rules.is_empty() {
            let keys: Vec<&str> = rules.iter().map(|r| r.key.as_str()).collect();
            rels = self.get_relations_touching(&keys, max_limit.saturating_mul(4))?;
        }

        let sessions = self.session_list(3)?;
        Ok((rules, rels, sessions))
    }

    /// Retrieve context specifically tailored to a set of active or modified files.
    /// Matches rules anchored to any of the files, expands their 1-hop graph relations,
    /// and includes universal rules (rules without an anchor or tagged as core architecture/rule).
    pub fn context_for_files(
        &self,
        files: &[String],
        topic: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<RuleRecord>, Vec<RelationRecord>, Vec<SessionEntry>)> {
        if files.is_empty() {
            return self.context_filtered(None, topic, limit);
        }

        let max_limit = if limit == 0 { 20 } else { limit };

        // Normalize input file paths and extract basenames
        let clean_files: Vec<(String, String)> = files
            .iter()
            .map(|f| {
                let p = f.replace('\\', "/");
                let trimmed = p.trim_start_matches("./").to_string();
                let basename = std::path::Path::new(&trimmed)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| trimmed.clone());
                (trimmed, basename)
            })
            .collect();

        // 1. Scan only lightweight routing fields. Full values are fetched for the
        // bounded result set after direct/related/universal keys are selected.
        let mut stmt = self.conn.prepare_cached(
            "SELECT key, anchor
             FROM memories
             WHERE archived_at IS NULL
             ORDER BY key ASC;",
        )?;
        let mut rows = stmt.query([])?;
        let mut direct_keys = Vec::with_capacity(max_limit.min(64));
        let mut direct_key_set = std::collections::HashSet::new();
        let mut universal_keys = Vec::with_capacity(max_limit.min(64));
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let anchor: Option<String> = row.get(1)?;
            let has_topic_match = if let Some(t) = topic.filter(|s| !s.trim().is_empty()) {
                let clean_t = t.trim();
                key.starts_with(clean_t) || key == clean_t
            } else {
                true
            };
            if !has_topic_match {
                continue;
            }

            let is_universal = anchor
                .as_deref()
                .is_none_or(|value| value.trim().is_empty());

            let mut is_matched = false;
            if let Some(anchor_raw) = &anchor {
                let anchor_norm = anchor_raw.replace('\\', "/");
                for (file_path, file_basename) in &clean_files {
                    let anchor_path_part = anchor_norm
                        .trim_start_matches('@')
                        .trim()
                        .split(':')
                        .next()
                        .unwrap_or("");
                    if anchor_norm.contains(file_path)
                        || (!anchor_path_part.is_empty() && file_path.contains(anchor_path_part))
                        || anchor_norm.contains(file_basename)
                    {
                        is_matched = true;
                        break;
                    }
                }
            }

            if is_matched && direct_keys.len() < max_limit {
                direct_key_set.insert(key.clone());
                direct_keys.push(key);
            } else if is_universal && universal_keys.len() < max_limit {
                universal_keys.push(key);
            }
        }
        drop(rows);
        drop(stmt);

        // 2. Expand one graph hop through indexed source/target lookups.
        let mut related_keys = std::collections::BTreeSet::new();
        if !direct_keys.is_empty() {
            let keys: Vec<&str> = direct_keys.iter().map(String::as_str).collect();
            let touching = self.get_relations_touching(&keys, max_limit.saturating_mul(4))?;
            for rel in touching {
                if direct_key_set.contains(&rel.source_key) {
                    related_keys.insert(rel.target_key);
                } else if direct_key_set.contains(&rel.target_key) {
                    related_keys.insert(rel.source_key);
                }
            }
        }

        // 3. Select a bounded, deterministic key set: direct, related, universal.
        let mut selected_keys = Vec::with_capacity(max_limit.min(64));
        let mut seen_keys = std::collections::HashSet::new();
        for key in direct_keys
            .into_iter()
            .chain(related_keys)
            .chain(universal_keys)
        {
            if seen_keys.insert(key.clone()) {
                selected_keys.push(key);
                if selected_keys.len() == max_limit {
                    break;
                }
            }
        }

        // 4. Materialize only selected records.
        let mut combined_rules = Vec::with_capacity(selected_keys.len());
        let mut fetch = self.conn.prepare_cached(
            "SELECT key, val, anchor, archived_at, archive_reason, kind
             FROM memories WHERE key = ?1 AND archived_at IS NULL LIMIT 1;",
        )?;
        for key in &selected_keys {
            let mut rows = fetch.query(params![key])?;
            if let Some(row) = rows.next()? {
                combined_rules.push(RuleRecord {
                    key: row.get(0)?,
                    val: row.get(1)?,
                    anchor: row.get(2)?,
                    archived_at: row.get(3)?,
                    archive_reason: row.get(4)?,
                    kind: row.get(5)?,
                });
            }
        }
        drop(fetch);

        let mut rels = Vec::new();
        if !combined_rules.is_empty() {
            let keys: Vec<&str> = combined_rules.iter().map(|r| r.key.as_str()).collect();
            rels = self.get_relations_touching(&keys, max_limit.saturating_mul(4))?;
        }

        let sessions = self.session_list(3)?;
        Ok((combined_rules, rels, sessions))
    }

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
        let mut unique_rules: std::collections::BTreeMap<&str, ParsedEntry> =
            std::collections::BTreeMap::new();
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
