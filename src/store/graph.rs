use super::{Store, now_epoch};
use crate::error::Result;
use crate::store::models::{
    RelationRecord, RuleEntry, RuleRecord, SessionEntry, ZombieReport, extract_anchor_paths,
};
use rusqlite::{TransactionBehavior, params};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

impl Store {
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

    pub(crate) fn get_relations_touching(
        &self,
        keys: &[&str],
        limit: usize,
    ) -> Result<Vec<RelationRecord>> {
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

    pub(crate) fn archive_zombies_tx(&mut self, zombies: &[ZombieReport]) -> Result<()> {
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
        let mut direct_key_set = HashSet::new();
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
        let mut related_keys = BTreeSet::new();
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
        let mut seen_keys = HashSet::new();
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
}
