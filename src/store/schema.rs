use super::{Store, insert_memory_routes};
use crate::error::{Error, Result};
use crate::store::models::StoreStats;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::fs;
use std::path::Path;

const SCHEMA_VERSION: u32 = 6;

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
        Ok(Self {
            conn,
            #[cfg(feature = "semantic-local")]
            db_path: Some(db_path.to_path_buf()),
            #[cfg(feature = "semantic-local")]
            semantic_runtime: std::cell::RefCell::new(super::semantic::SemanticRuntime::default()),
        })
    }

    /// Open an in-memory database instance (ideal for isolated unit tests).
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure_conn(&mut conn, true)?;
        Ok(Self {
            conn,
            #[cfg(feature = "semantic-local")]
            db_path: None,
            #[cfg(feature = "semantic-local")]
            semantic_runtime: std::cell::RefCell::new(super::semantic::SemanticRuntime::default()),
        })
    }

    pub(crate) fn configure_conn(conn: &mut Connection, need_write: bool) -> Result<()> {
        conn.execute_batch(
            "PRAGMA busy_timeout = 10000;
             PRAGMA synchronous = NORMAL;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;",
        )?;

        let user_version: u32 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;

        if user_version > SCHEMA_VERSION {
            return Err(Error::Usage(format!(
                "Database schema {user_version} is newer than supported schema {SCHEMA_VERSION}; upgrade agent-mem before writing"
            )));
        }
        if user_version == SCHEMA_VERSION {
            // Fast path: schema and migrations already initialized. Bypass DDL and table scans.
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
                );
                CREATE TABLE IF NOT EXISTS memory_routes (
                    memory_key TEXT NOT NULL,
                    route_kind INTEGER NOT NULL,
                    route_hash INTEGER NOT NULL,
                    PRIMARY KEY (memory_key, route_kind, route_hash)
                ) WITHOUT ROWID;
                CREATE INDEX IF NOT EXISTS idx_memory_routes_lookup
                    ON memory_routes(route_kind, route_hash, memory_key);
                CREATE TABLE IF NOT EXISTS semantic_state (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    generation TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    dimensions INTEGER NOT NULL,
                    records_count INTEGER NOT NULL,
                    built_at INTEGER NOT NULL,
                    dirty INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS semantic_records (
                    semantic_id INTEGER PRIMARY KEY,
                    memory_key TEXT NOT NULL UNIQUE,
                    fingerprint_hi INTEGER NOT NULL,
                    fingerprint_lo INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS memory_metadata (
                    memory_key TEXT PRIMARY KEY,
                    provenance TEXT NOT NULL,
                    trust TEXT NOT NULL CHECK (trust IN ('local', 'reviewed', 'untrusted')),
                    created_at INTEGER NOT NULL,
                    reviewed_at INTEGER
                ) WITHOUT ROWID;",
            )?;
            tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;
            tx.commit()?;
        } else if user_version == 5 {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            create_metadata_schema(&tx)?;
            tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;
            tx.commit()?;
        } else if user_version == 4 {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            create_semantic_schema(&tx)?;
            create_metadata_schema(&tx)?;
            tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;
            tx.commit()?;
        } else if user_version == 3 {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            create_route_schema(&tx)?;
            backfill_routes(&tx)?;
            create_semantic_schema(&tx)?;
            create_metadata_schema(&tx)?;
            tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;
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
            create_route_schema(&tx)?;
            backfill_routes(&tx)?;
            create_semantic_schema(&tx)?;
            create_metadata_schema(&tx)?;
            tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;
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

fn create_route_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_routes (
            memory_key TEXT NOT NULL,
            route_kind INTEGER NOT NULL,
            route_hash INTEGER NOT NULL,
            PRIMARY KEY (memory_key, route_kind, route_hash)
        ) WITHOUT ROWID;",
    )?;
    Ok(())
}

fn create_semantic_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS semantic_state (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            generation TEXT NOT NULL,
            model_id TEXT NOT NULL,
            dimensions INTEGER NOT NULL,
            records_count INTEGER NOT NULL,
            built_at INTEGER NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS semantic_records (
            semantic_id INTEGER PRIMARY KEY,
            memory_key TEXT NOT NULL UNIQUE,
            fingerprint_hi INTEGER NOT NULL,
            fingerprint_lo INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn create_metadata_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_metadata (
            memory_key TEXT PRIMARY KEY,
            provenance TEXT NOT NULL,
            trust TEXT NOT NULL CHECK (trust IN ('local', 'reviewed', 'untrusted')),
            created_at INTEGER NOT NULL,
            reviewed_at INTEGER
        ) WITHOUT ROWID;
        INSERT OR IGNORE INTO memory_metadata
            (memory_key, provenance, trust, created_at, reviewed_at)
        SELECT key, 'migration:legacy', 'local', updated_at, NULL FROM memories;",
    )?;
    Ok(())
}

fn backfill_routes(tx: &Transaction<'_>) -> Result<()> {
    tx.execute("DROP INDEX IF EXISTS idx_memory_routes_lookup;", [])?;
    tx.execute("DELETE FROM memory_routes;", [])?;
    let mut select = tx.prepare("SELECT key, anchor FROM memories WHERE anchor IS NOT NULL;")?;
    let mut rows = select.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let anchor: String = row.get(1)?;
        insert_memory_routes(tx, &key, Some(&anchor))?;
    }
    drop(rows);
    drop(select);
    tx.execute(
        "CREATE INDEX idx_memory_routes_lookup
         ON memory_routes(route_kind, route_hash, memory_key);",
        [],
    )?;
    Ok(())
}
