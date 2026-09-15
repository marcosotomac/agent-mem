use super::{Store, now_epoch};
use crate::error::Result;
use crate::store::models::{SessionEntry, infer_kind};
use rusqlite::{TransactionBehavior, params};

impl Store {
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
}
