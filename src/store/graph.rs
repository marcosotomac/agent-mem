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
        let clean_anchor = anchor
            .map(|a| {
                a.trim()
                    .replace('\\', "/")
                    .trim_start_matches('@')
                    .trim()
                    .trim_start_matches("./")
                    .to_string()
            })
            .filter(|s| !s.is_empty());

        let clean_topic = topic.map(str::trim).filter(|s| !s.is_empty());

        let rules = match (clean_anchor, clean_topic) {
            (Some(a), maybe_topic) => {
                // When anchor is specified, match direct anchors (priority 1) and 1-hop related memories (priority 2).
                // If topic is also specified, apply it across both without discarding either filter.
                let (topic_pat, topic_exact) = match maybe_topic {
                    Some(t) => (Some(format!("{}%", t)), Some(t.to_string())),
                    None => (None, None),
                };
                let like_prefix = format!("{}:%", a);
                let like_exact = format!("{}%", a);
                let like_comma = format!("%, {}%", a);

                let mut stmt = self.conn.prepare_cached(
                    "WITH direct_anchors AS (
                        SELECT key, val, anchor, archived_at, archive_reason, kind, 1 AS priority
                        FROM memories
                        WHERE (anchor = ?1 OR anchor LIKE ?2 OR anchor LIKE ?3 OR ?1 LIKE anchor || '%' OR anchor LIKE ?6)
                          AND archived_at IS NULL
                    ),
                    related AS (
                        SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason, m.kind, 2 AS priority
                        FROM relations r
                        JOIN memories m ON r.target_key = m.key
                        WHERE r.source_key IN (SELECT key FROM direct_anchors)
                          AND m.archived_at IS NULL
                    ),
                    combined AS (
                        SELECT key, val, anchor, archived_at, archive_reason, kind, MIN(priority) AS prio
                        FROM (
                            SELECT * FROM direct_anchors
                            UNION ALL
                            SELECT * FROM related
                        )
                        WHERE (?4 IS NULL OR key LIKE ?4 OR key = ?5)
                        GROUP BY key, val, anchor, archived_at, archive_reason, kind
                    )
                    SELECT key, val, anchor, archived_at, archive_reason, kind
                    FROM combined
                    ORDER BY prio ASC, key ASC
                    LIMIT ?7;",
                )?;

                let mut rows = stmt.query(params![
                    a,
                    like_prefix,
                    like_exact,
                    topic_pat,
                    topic_exact,
                    like_comma,
                    max_limit as i64
                ])?;

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
            }
            (None, Some(t)) => {
                let pattern = format!("{}%", t);
                let mut stmt = self.conn.prepare_cached(
                    "SELECT key, val, anchor, archived_at, archive_reason, kind
                     FROM memories
                     WHERE (key LIKE ?1 OR key = ?2) AND archived_at IS NULL
                     ORDER BY key ASC LIMIT ?3;",
                )?;
                let mut rows = stmt.query(params![pattern, t, max_limit as i64])?;
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
            }
            (None, None) => {
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
            }
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
    /// Matches rules anchored to any of the files, ranked by precision relevance
    /// (exact path > relative path > directory sibling > directory hierarchy),
    /// expands their 1-hop graph relations, and includes universal unanchored rules.
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

        // Normalize input file paths and extract basenames/directory components
        let clean_path_strings: Vec<String> = files
            .iter()
            .map(|f| {
                let p = f.replace('\\', "/");
                p.trim_start_matches('@')
                    .trim()
                    .trim_start_matches("./")
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        if clean_path_strings.is_empty() {
            return self.context_filtered(None, topic, limit);
        }

        let normalized_queries: Vec<NormalizedQueryFile<'_>> = clean_path_strings
            .iter()
            .map(|p| {
                let (dir, base) = match p.rsplit_once('/') {
                    Some((d, b)) => (Some(d), b),
                    None => (None, p.as_str()),
                };
                NormalizedQueryFile {
                    clean_path: p.as_str(),
                    dir,
                    base,
                    is_generic: is_generic_filename(base),
                }
            })
            .collect();

        let clean_topic = topic.map(str::trim).filter(|s| !s.is_empty());

        // 1. Scan routing fields. Full values are fetched for the bounded result set.
        // If topic is specified, use the key index to prune unrelated rows early.
        let mut stmt = if clean_topic.is_some() {
            self.conn.prepare_cached(
                "SELECT key, anchor
                 FROM memories
                 WHERE archived_at IS NULL AND (key LIKE ?1 OR key = ?2)
                 ORDER BY key ASC;",
            )?
        } else {
            self.conn.prepare_cached(
                "SELECT key, anchor
                 FROM memories
                 WHERE archived_at IS NULL
                 ORDER BY key ASC;",
            )?
        };

        let mut rows = if let Some(t) = clean_topic {
            let pattern = format!("{}%", t);
            stmt.query(params![pattern, t])?
        } else {
            stmt.query([])?
        };

        let mut scored_direct: Vec<(u32, String)> = Vec::new();
        let mut universal_keys = Vec::new();

        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let anchor: Option<String> = row.get(1)?;

            if let Some(anchor_raw) = anchor.filter(|a| !a.trim().is_empty()) {
                let paths = extract_anchor_paths(&anchor_raw);
                let mut best_score: Option<u32> = None;
                for query in &normalized_queries {
                    for path in &paths {
                        if let Some(score) = score_single_anchor(query, path) {
                            best_score = Some(best_score.map_or(score, |curr| curr.max(score)));
                        }
                    }
                }
                if let Some(score) = best_score {
                    scored_direct.push((score, key));
                }
            } else {
                universal_keys.push(key);
            }
        }
        drop(rows);
        drop(stmt);

        // Sort candidate direct keys by: score DESC, key ASC
        scored_direct.sort_unstable_by(|(s1, k1), (s2, k2)| {
            s2.cmp(s1).then_with(|| k1.cmp(k2))
        });

        let mut direct_keys = Vec::with_capacity(max_limit.min(scored_direct.len()));
        let mut direct_key_set = HashSet::with_capacity(max_limit.min(scored_direct.len()));
        for (_score, key) in scored_direct.into_iter().take(max_limit) {
            direct_key_set.insert(key.clone());
            direct_keys.push(key);
        }

        // 2. Expand one graph hop through indexed source/target lookups.
        let mut related_keys = BTreeSet::new();
        if !direct_keys.is_empty() {
            let keys: Vec<&str> = direct_keys.iter().map(String::as_str).collect();
            let touching = self.get_relations_touching(&keys, max_limit.saturating_mul(4))?;
            for rel in touching {
                let candidate = if direct_key_set.contains(&rel.source_key) {
                    Some(rel.target_key)
                } else if direct_key_set.contains(&rel.target_key) {
                    Some(rel.source_key)
                } else {
                    None
                };
                if let Some(cand) = candidate {
                    // Check topic match on related keys if topic was specified
                    let topic_ok = match clean_topic {
                        Some(t) => cand.starts_with(t) || cand == t,
                        None => true,
                    };
                    if topic_ok {
                        related_keys.insert(cand);
                    }
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

struct NormalizedQueryFile<'a> {
    clean_path: &'a str,
    dir: Option<&'a str>,
    base: &'a str,
    is_generic: bool,
}

fn is_generic_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "mod.rs"
            | "lib.rs"
            | "main.rs"
            | "index.ts"
            | "index.js"
            | "index.tsx"
            | "index.jsx"
            | "types.ts"
            | "utils.ts"
    )
}

fn score_single_anchor(query: &NormalizedQueryFile<'_>, anchor_clean: &str) -> Option<u32> {
    // 1. Exact match (e.g. "src/api/mod.rs" == "src/api/mod.rs")
    if query.clean_path == anchor_clean {
        return Some(1000);
    }

    // 2. Relative suffix match (e.g. query "api/mod.rs" matches anchor "src/api/mod.rs"
    //    or query "src/api/mod.rs" matches anchor "api/mod.rs")
    if query.clean_path.ends_with(&format!("/{}", anchor_clean)) {
        return Some(800);
    }
    if anchor_clean.ends_with(&format!("/{}", query.clean_path)) {
        return Some(800);
    }

    let (anchor_dir, anchor_base) = match anchor_clean.rsplit_once('/') {
        Some((d, b)) => (Some(d), b),
        None => (None, anchor_clean),
    };

    if let (Some(qd), Some(ad)) = (query.dir, anchor_dir) {
        if qd == ad {
            // Sibling in the same directory (e.g. "src/api/routes.rs" vs "src/api/mod.rs")
            return Some(500);
        }

        // Parent/child subdirectory
        if qd.starts_with(&format!("{}/", ad)) || ad.starts_with(&format!("{}/", qd)) {
            let q_parts: Vec<&str> = qd.split('/').collect();
            let a_parts: Vec<&str> = ad.split('/').collect();
            let shared = q_parts
                .iter()
                .zip(a_parts.iter())
                .take_while(|(q, a)| q == a)
                .count();
            return Some(200 + (shared as u32 * 20));
        }

        // Both have directories, but directories don't match (e.g. "src/db" vs "src/api")
        if query.base.eq_ignore_ascii_case(anchor_base) {
            if query.is_generic {
                // NEVER match generic files across distinct directories!
                return None;
            } else {
                return Some(50);
            }
        }
    } else {
        // Query was unqualified (e.g. query was just "auth.rs" or "user.rs")
        if query.base.eq_ignore_ascii_case(anchor_base) {
            return Some(100);
        }
    }

    None
}
