use super::Store;
use crate::error::{Error, Result};
use crate::store::models::StoreStats;
use rusqlite::{Connection, TransactionBehavior};
use std::fs;
use std::path::Path;

impl Store {
    /// Open SQLite database with performance-optimized PRAGMAs.
    pub fn open(db_path: &Path, need_write: bool) -> Result<Self> {
        let exists = db_path.exists();
        if !exists {
            if !need_write {
                return Err(Error::NotInitialized);
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

    pub(crate) fn configure_conn(conn: &mut Connection, need_write: bool) -> Result<()> {
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
                return Err(Error::NotInitialized);
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

    /// Read runtime SQLite metadata and record counts.
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
