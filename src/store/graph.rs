use super::{Store, mark_semantic_dirty, now_epoch};
use crate::error::Result;
use crate::store::models::{
    ROUTE_PATH_SUFFIX, RelationRecord, RuleEntry, RuleRecord, SessionEntry, ZombieReport,
    extract_anchor_paths, route_hash, routing_entries_for_path,
};
use rusqlite::{TransactionBehavior, params};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCandidate {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SuggestedRelation {
    pub source_key: String,
    pub rel_type: String,
    pub target_key: String,
    pub confidence: u8,
    pub reason: String,
}

pub struct TaskContext {
    pub rules: Vec<RuleRecord>,
    pub relations: Vec<RelationRecord>,
    pub sessions: Vec<SessionEntry>,
    /// Bounded candidate pool in ranking order; callers can compare it with
    /// emitted rules to explain selection and limit omissions.
    pub candidates: Vec<ContextCandidate>,
}

pub struct ContextExplanation {
    selected: HashSet<String>,
    candidates: Vec<ContextCandidate>,
}

impl TaskContext {
    pub fn clone_explanation(&self) -> ContextExplanation {
        ContextExplanation {
            selected: self.rules.iter().map(|rule| rule.key.clone()).collect(),
            candidates: self.candidates.clone(),
        }
    }
}

impl ContextExplanation {
    pub fn explanation_lines(
        &self,
        emitted: &HashSet<String>,
        scope: &str,
    ) -> (Vec<String>, Vec<String>) {
        let mut included = Vec::new();
        let mut omitted = Vec::new();
        for candidate in &self.candidates {
            let line = format!("[{scope}] {}: {}", candidate.key, candidate.reason);
            if emitted.contains(&candidate.key) {
                included.push(line);
            } else if !self.selected.contains(&candidate.key) {
                omitted.push(line);
            }
        }
        (included, omitted)
    }
}

fn extract_key_namespace<'a>(key: &'a str, kind: &str) -> Option<&'a str> {
    let stripped = if let Some(rest) = key.strip_prefix(kind).and_then(|r| r.strip_prefix('/')) {
        rest
    } else {
        key
    };
    if stripped.contains('/') {
        stripped.split('/').next().filter(|s| !s.is_empty())
    } else {
        None
    }
}

impl Store {
    fn context_key_is_trusted(&self, key: &str) -> Result<bool> {
        let trust = self.conn.query_row(
            "SELECT trust FROM memory_metadata WHERE memory_key = ?1;",
            params![key],
            |row| row.get::<_, String>(0),
        );
        match trust {
            Ok(value) => Ok(value != super::TRUST_UNTRUSTED),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    /// Remove quarantined memories and relations that touch them before
    /// context is injected into an agent prompt. Explicit search/get APIs keep
    /// exposing these records so a human can review and promote them.
    pub fn filter_trusted_context(
        &self,
        rules: Vec<RuleRecord>,
        relations: Vec<RelationRecord>,
    ) -> Result<(Vec<RuleRecord>, Vec<RelationRecord>)> {
        let mut trusted_keys = HashSet::with_capacity(rules.len());
        let mut trusted_rules = Vec::with_capacity(rules.len());
        for rule in rules {
            if self.context_key_is_trusted(&rule.key)? {
                trusted_keys.insert(rule.key.clone());
                trusted_rules.push(rule);
            }
        }

        let mut endpoint_trust = HashMap::new();
        let mut trusted_relations = Vec::with_capacity(relations.len());
        for relation in relations {
            let source_trusted = if trusted_keys.contains(&relation.source_key) {
                true
            } else if let Some(value) = endpoint_trust.get(&relation.source_key) {
                *value
            } else {
                let value = self.context_key_is_trusted(&relation.source_key)?;
                endpoint_trust.insert(relation.source_key.clone(), value);
                value
            };
            let target_trusted = if trusted_keys.contains(&relation.target_key) {
                true
            } else if let Some(value) = endpoint_trust.get(&relation.target_key) {
                *value
            } else {
                let value = self.context_key_is_trusted(&relation.target_key)?;
                endpoint_trust.insert(relation.target_key.clone(), value);
                value
            };
            if source_trusted && target_trusted {
                trusted_relations.push(relation);
            }
        }
        Ok((trusted_rules, trusted_relations))
    }

    fn select_diverse_context_keys(
        direct: Vec<String>,
        related: Vec<String>,
        universal: Vec<String>,
        limit: usize,
    ) -> Vec<String> {
        if limit == 0 {
            return Vec::new();
        }

        // Reserve bounded space for graph evidence and repository-wide rules so
        // a large exact-path cluster cannot starve the information most likely
        // to change an agent decision. Empty classes return their slots.
        let direct_reserve = usize::from(!direct.is_empty());
        let non_direct_budget = limit.saturating_sub(direct_reserve);
        let related_target = if related.is_empty() {
            0
        } else {
            (limit / 4).max(1).min(related.len()).min(non_direct_budget)
        };
        let universal_budget = non_direct_budget.saturating_sub(related_target);
        let universal_target = if universal.is_empty() {
            0
        } else {
            (limit / 10)
                .max(1)
                .min(universal.len())
                .min(universal_budget)
        };
        let direct_target = limit.saturating_sub(related_target + universal_target);

        let mut selected = Vec::with_capacity(limit.min(64));
        let mut seen = HashSet::with_capacity(limit.min(64));
        let mut append = |keys: &[String], take: usize| {
            for key in keys.iter().take(take) {
                if seen.insert(key.clone()) {
                    selected.push(key.clone());
                }
            }
        };
        append(&direct, direct_target);
        append(&related, related_target);
        append(&universal, universal_target);

        // Deterministic backfill ensures sparse classes never reduce recall.
        for keys in [&direct, &related, &universal] {
            for key in keys {
                if selected.len() == limit {
                    return selected;
                }
                if seen.insert(key.clone()) {
                    selected.push(key.clone());
                }
            }
        }
        selected
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
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for key in [s, t] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE key = ?1);",
                params![key],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(crate::error::Error::NotFound(key.to_string()));
            }
        }
        if r == "supersedes" {
            if s == t {
                return Err(crate::error::Error::Usage(
                    "A memory cannot supersede itself".into(),
                ));
            }
            let creates_cycle: bool = tx.query_row(
                "WITH RECURSIVE chain(key) AS (
                    SELECT target_key FROM relations
                    WHERE source_key = ?1 AND rel_type = 'supersedes'
                    UNION
                    SELECT relations.target_key FROM relations
                    JOIN chain ON relations.source_key = chain.key
                    WHERE relations.rel_type = 'supersedes'
                 )
                 SELECT EXISTS(SELECT 1 FROM chain WHERE key = ?2);",
                params![t, s],
                |row| row.get(0),
            )?;
            if creates_cycle {
                return Err(crate::error::Error::Usage(
                    "A supersedes relation cannot create a cycle".into(),
                ));
            }
        }
        tx.execute(
            "INSERT INTO relations (source_key, rel_type, target_key, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source_key, rel_type, target_key) DO NOTHING;",
            params![s, r, t, now],
        )?;
        if r == "supersedes" {
            tx.execute(
                "UPDATE memories
                 SET archived_at = ?1, archive_reason = ?2, updated_at = ?1
                 WHERE key = ?3 AND archived_at IS NULL;",
                params![now, format!("superseded_by: {s}"), t],
            )?;
            tx.execute("DELETE FROM memories_fts WHERE key = ?1;", params![t])?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind)
                 SELECT key, val, anchor, archive_reason, kind FROM memories WHERE key = ?1;",
                params![t],
            )?;
            mark_semantic_dirty(&tx)?;
        }
        tx.commit()?;
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

    fn get_outgoing_targets(&self, keys: &[&str], limit: usize) -> Result<Vec<String>> {
        if keys.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT target_key FROM relations
             WHERE source_key = ?1
             ORDER BY rel_type, target_key
             LIMIT ?2;",
        )?;
        let sql_limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut targets = BTreeSet::new();
        for key in keys {
            let mut rows = stmt.query(params![key, sql_limit])?;
            while let Some(row) = rows.next()? {
                targets.insert(row.get(0)?);
                if targets.len() == limit {
                    return Ok(targets.into_iter().collect());
                }
            }
        }
        Ok(targets.into_iter().collect())
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

    /// Discover suggested relations between a memory and existing memories based on anchor co-location
    /// and namespace hierarchy topology.
    pub fn suggest_relations(
        &self,
        key: &str,
        _val: &str,
        anchor: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<SuggestedRelation>> {
        let clean_key = key.trim();
        let (db_anchor, db_kind) = if anchor.is_none() || kind.is_none() {
            self.conn
                .query_row(
                    "SELECT anchor, kind FROM memories WHERE key = ?1;",
                    params![clean_key],
                    |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
                )
                .ok()
                .map(|(a, k)| (a, Some(k)))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        let resolved_anchor = anchor.or(db_anchor.as_deref());
        let effective_kind = kind
            .or(db_kind.as_deref())
            .unwrap_or_else(|| crate::store::infer_kind(clean_key));
        let anchor_file = resolved_anchor
            .map(|a| a.split(':').next().unwrap_or(a).trim())
            .filter(|a| !a.is_empty());

        let key_namespace = extract_key_namespace(clean_key, effective_kind);

        let mut stmt = self.conn.prepare_cached(
            "SELECT key, val, anchor, kind FROM memories WHERE archived_at IS NULL AND key != ?1 LIMIT 250;",
        )?;
        let mut rows = stmt.query(params![clean_key])?;

        let mut existing_relations = HashSet::new();
        let mut rel_stmt = self.conn.prepare_cached(
            "SELECT source_key, target_key FROM relations WHERE source_key = ?1 OR target_key = ?1;",
        )?;
        let mut rel_rows = rel_stmt.query(params![clean_key])?;
        while let Some(r) = rel_rows.next()? {
            let s: String = r.get(0)?;
            let t: String = r.get(1)?;
            existing_relations.insert((s, t));
        }

        let mut suggestions = Vec::new();

        while let Some(row) = rows.next()? {
            let other_key: String = row.get(0)?;
            let _other_val: String = row.get(1)?;
            let other_anchor: Option<String> = row.get(2)?;
            let other_kind: String = row.get(3)?;

            let other_anchor_file = other_anchor
                .as_deref()
                .map(|a| a.split(':').next().unwrap_or(a).trim())
                .filter(|a| !a.is_empty());

            let other_namespace = extract_key_namespace(&other_key, &other_kind);

            let same_anchor_file = anchor_file.is_some()
                && other_anchor_file.is_some()
                && anchor_file == other_anchor_file;

            let same_namespace = key_namespace.is_some()
                && other_namespace.is_some()
                && key_namespace == other_namespace;

            if same_anchor_file {
                let file_name = anchor_file.unwrap();
                let exact_anchor_match =
                    anchor.is_some() && other_anchor.is_some() && anchor == other_anchor.as_deref();

                if effective_kind == "decision" && other_kind == "gotcha" {
                    if !existing_relations.contains(&(clean_key.to_string(), other_key.clone())) {
                        suggestions.push(SuggestedRelation {
                            source_key: clean_key.to_string(),
                            rel_type: "mitigates".to_string(),
                            target_key: other_key.clone(),
                            confidence: 85,
                            reason: format!("Decision and gotcha both anchor to {file_name}"),
                        });
                    }
                } else if effective_kind == "gotcha" && other_kind == "decision" {
                    if !existing_relations.contains(&(other_key.clone(), clean_key.to_string())) {
                        suggestions.push(SuggestedRelation {
                            source_key: other_key.clone(),
                            rel_type: "mitigates".to_string(),
                            target_key: clean_key.to_string(),
                            confidence: 85,
                            reason: format!("Decision and gotcha both anchor to {file_name}"),
                        });
                    }
                } else if effective_kind == other_kind {
                    if exact_anchor_match {
                        if !existing_relations.contains(&(clean_key.to_string(), other_key.clone()))
                        {
                            suggestions.push(SuggestedRelation {
                                source_key: clean_key.to_string(),
                                rel_type: "supersedes".to_string(),
                                target_key: other_key.clone(),
                                confidence: 90,
                                reason: format!(
                                    "Both {effective_kind}s anchor to exact same code location"
                                ),
                            });
                        }
                    } else if !existing_relations
                        .contains(&(clean_key.to_string(), other_key.clone()))
                    {
                        suggestions.push(SuggestedRelation {
                            source_key: clean_key.to_string(),
                            rel_type: "relates_to".to_string(),
                            target_key: other_key.clone(),
                            confidence: 80,
                            reason: format!("Both {effective_kind}s anchor to {file_name}"),
                        });
                    }
                }
            } else if same_namespace {
                let ns = key_namespace.unwrap();
                if (effective_kind == "decision" && other_kind == "gotcha")
                    || (effective_kind == "gotcha" && other_kind == "decision")
                {
                    let (decision, gotcha) = if effective_kind == "decision" {
                        (clean_key.to_string(), other_key.clone())
                    } else {
                        (other_key.clone(), clean_key.to_string())
                    };
                    if !existing_relations.contains(&(decision.clone(), gotcha.clone())) {
                        suggestions.push(SuggestedRelation {
                            source_key: decision,
                            rel_type: "mitigates".to_string(),
                            target_key: gotcha,
                            confidence: 75,
                            reason: format!("Decision and gotcha share namespace '{ns}'"),
                        });
                    }
                } else if effective_kind == other_kind
                    && !existing_relations.contains(&(clean_key.to_string(), other_key.clone()))
                {
                    suggestions.push(SuggestedRelation {
                        source_key: clean_key.to_string(),
                        rel_type: "relates_to".to_string(),
                        target_key: other_key.clone(),
                        confidence: 65,
                        reason: format!("Both {effective_kind}s share namespace '{ns}'"),
                    });
                }
            }
        }

        suggestions.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        Ok(suggestions)
    }

    /// Automatically establish high-confidence relations across recent active memories.
    pub fn auto_relate_high_confidence(
        &mut self,
        min_confidence: u8,
    ) -> Result<Vec<SuggestedRelation>> {
        let keys: Vec<(String, String, Option<String>, String)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT key, val, anchor, kind FROM memories WHERE archived_at IS NULL ORDER BY updated_at DESC LIMIT 100;",
            )?;
            let mut rows = stmt.query([])?;
            let mut list = Vec::new();
            while let Some(row) = rows.next()? {
                list.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
            }
            list
        };

        let mut applied = Vec::new();
        for (key, val, anchor, kind) in &keys {
            let suggestions = self.suggest_relations(key, val, anchor.as_deref(), Some(kind))?;
            for s in suggestions {
                if s.confidence >= min_confidence
                    && self
                        .relate(&s.source_key, &s.rel_type, &s.target_key)
                        .is_ok()
                {
                    applied.push(s);
                }
            }
        }
        Ok(applied)
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

        let selected: HashSet<&str> = keys.iter().copied().collect();
        relations.sort_unstable_by(|a, b| {
            let a_internal = selected.contains(a.source_key.as_str())
                && selected.contains(a.target_key.as_str());
            let b_internal = selected.contains(b.source_key.as_str())
                && selected.contains(b.target_key.as_str());
            b_internal.cmp(&a_internal).then_with(|| {
                (&a.source_key, &a.rel_type, &a.target_key).cmp(&(
                    &b.source_key,
                    &b.rel_type,
                    &b.target_key,
                ))
            })
        });
        relations.dedup_by(|a, b| {
            a.source_key == b.source_key && a.rel_type == b.rel_type && a.target_key == b.target_key
        });
        relations.truncate(limit);
        Ok(relations)
    }

    pub(crate) fn repair_zombies_tx(&mut self, zombies: &[ZombieReport]) -> Result<()> {
        if zombies.is_empty() {
            return Ok(());
        }
        let now = now_epoch();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        for z in zombies {
            if z.remaining_paths.is_empty() {
                let reason = format!("file_deleted: {}", z.missing_paths.join(", "));
                tx.execute(
                    "UPDATE memories SET archived_at = ?1, archive_reason = ?2, updated_at = ?3 WHERE key = ?4;",
                    params![now, reason, now, z.key],
                )?;
            } else {
                let anchor = z.remaining_paths.join(", ");
                tx.execute(
                    "UPDATE memories SET anchor = ?1, updated_at = ?2 WHERE key = ?3;",
                    params![anchor, now, z.key],
                )?;
                tx.execute(
                    "DELETE FROM memory_routes WHERE memory_key = ?1;",
                    params![z.key],
                )?;
                super::insert_memory_routes(&tx, &z.key, Some(&anchor))?;
            }
            tx.execute("DELETE FROM memories_fts WHERE key = ?1;", params![z.key])?;
            tx.execute(
                "INSERT INTO memories_fts (key, val, anchor, archive_reason, kind) 
                 SELECT key, val, anchor, archive_reason, kind FROM memories WHERE key = ?1;",
                params![z.key],
            )?;
        }

        mark_semantic_dirty(&tx)?;

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
                    .iter()
                    .filter(|p| !project_root.join(p).exists())
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    let remaining = paths
                        .into_iter()
                        .filter(|p| !missing.contains(p))
                        .collect::<Vec<_>>();
                    zombies.push(ZombieReport {
                        key: m.key.clone(),
                        missing_paths: missing,
                        archived: !dry_run && remaining.is_empty(),
                        remaining_paths: remaining,
                    });
                }
            }
        }

        if !dry_run {
            self.repair_zombies_tx(&zombies)?;
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
                    .iter()
                    .filter(|p| deleted_files.iter().any(|d| d == *p))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    let remaining = paths
                        .into_iter()
                        .filter(|p| !missing.contains(p))
                        .collect::<Vec<_>>();
                    zombies.push(ZombieReport {
                        key: m.key.clone(),
                        missing_paths: missing,
                        archived: !dry_run && remaining.is_empty(),
                        remaining_paths: remaining,
                    });
                }
            }
        }

        if !dry_run {
            self.repair_zombies_tx(&zombies)?;
        }

        Ok(zombies)
    }

    /// Export dense context (rules and recent sessions).
    pub fn context(&self) -> Result<(Vec<RuleEntry>, Vec<SessionEntry>)> {
        let rules = self.dump()?;
        let sessions = self.session_list(3)?;
        Ok((rules, sessions))
    }

    fn ranked_anchor_keys(
        &self,
        files: &[String],
        topic: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let clean_paths: Vec<String> = files
            .iter()
            .filter_map(|file| normalize_query_path(file))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(256)
            .collect();
        if clean_paths.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let queries: Vec<NormalizedQueryFile<'_>> = clean_paths
            .iter()
            .map(|path| NormalizedQueryFile::new(path))
            .collect();
        let clean_topic = topic.map(str::trim).filter(|value| !value.is_empty());
        let exact_route_budget = clean_paths.len().saturating_mul(limit.min(64));
        let candidate_budget = limit
            .saturating_mul(32)
            .max(exact_route_budget)
            .clamp(128, 8192);
        let per_route_budget = limit.saturating_mul(4).clamp(32, 512).min(candidate_budget);
        let sql_limit = i64::try_from(per_route_budget).unwrap_or(i64::MAX);

        // Exact full paths are queried first so a broad basename or directory
        // route can never crowd them out of the bounded candidate set.
        let mut lookup_routes = Vec::new();
        let mut seen_routes = HashSet::new();
        for path in &clean_paths {
            let route = (ROUTE_PATH_SUFFIX, route_hash(path));
            if seen_routes.insert(route) {
                lookup_routes.push((route.0, route.1, true));
            }
        }
        for path in &clean_paths {
            for route in routing_entries_for_path(path) {
                if seen_routes.insert(route) {
                    lookup_routes.push((route.0, route.1, false));
                }
            }
        }

        let mut candidates: HashMap<String, String> =
            HashMap::with_capacity(candidate_budget.min(1024));
        let topic_pattern = clean_topic.map(|prefix| format!("{prefix}%"));
        let mut lookup = self.conn.prepare_cached(
            "SELECT m.key, m.anchor
             FROM memory_routes r
             JOIN memories m ON m.key = r.memory_key
             WHERE r.route_kind = ?1 AND r.route_hash = ?2 AND m.archived_at IS NULL
               AND (?3 IS NULL OR m.key LIKE ?4 OR m.key = ?3)
             ORDER BY m.key
             LIMIT ?5;",
        )?;
        for (route_kind, route_hash, exact) in lookup_routes {
            let route_limit = if exact {
                i64::try_from(limit.min(64)).unwrap_or(i64::MAX)
            } else {
                sql_limit
            };
            let mut rows = lookup.query(params![
                route_kind,
                route_hash,
                clean_topic,
                topic_pattern.as_deref(),
                route_limit
            ])?;
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let anchor: String = row.get(1)?;
                if candidates.len() < candidate_budget || candidates.contains_key(&key) {
                    candidates.entry(key).or_insert(anchor);
                }
            }
        }

        let mut scored: Vec<(u32, String)> = Vec::with_capacity(candidates.len());
        for (key, anchor) in candidates {
            let paths = extract_anchor_paths(&anchor);
            let best = queries
                .iter()
                .flat_map(|query| {
                    paths
                        .iter()
                        .filter_map(move |path| score_single_anchor(query, path))
                })
                .max();
            if let Some(score) = best {
                scored.push((score, key));
            }
        }
        scored.sort_unstable_by(|(score_a, key_a), (score_b, key_b)| {
            score_b.cmp(score_a).then_with(|| key_a.cmp(key_b))
        });
        Ok(scored.into_iter().take(limit).map(|(_, key)| key).collect())
    }

    fn universal_keys(&self, topic: Option<&str>, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let clean_topic = topic.map(str::trim).filter(|value| !value.is_empty());
        let mut keys = Vec::with_capacity(limit.min(32));
        if let Some(prefix) = clean_topic {
            let pattern = format!("{}%", prefix);
            let mut stmt = self.conn.prepare_cached(
                "SELECT key FROM memories
                 WHERE anchor IS NULL AND archived_at IS NULL AND (key LIKE ?1 OR key = ?2)
                 ORDER BY key LIMIT ?3;",
            )?;
            let mut rows = stmt.query(params![pattern, prefix, limit as i64])?;
            while let Some(row) = rows.next()? {
                keys.push(row.get(0)?);
            }
        } else {
            let mut stmt = self.conn.prepare_cached(
                "SELECT key FROM memories
                 WHERE anchor IS NULL AND archived_at IS NULL
                 ORDER BY key LIMIT ?1;",
            )?;
            let mut rows = stmt.query(params![limit as i64])?;
            while let Some(row) = rows.next()? {
                keys.push(row.get(0)?);
            }
        }
        Ok(keys)
    }

    fn materialize_active_rules(&self, keys: &[String]) -> Result<Vec<RuleRecord>> {
        let mut rules = Vec::with_capacity(keys.len());
        let mut fetch = self.conn.prepare_cached(
            "SELECT key, val, anchor, archived_at, archive_reason, kind
             FROM memories WHERE key = ?1 AND archived_at IS NULL LIMIT 1;",
        )?;
        for key in keys {
            let mut rows = fetch.query(params![key])?;
            if let Some(row) = rows.next()? {
                rules.push(RuleRecord {
                    key: row.get(0)?,
                    val: row.get(1)?,
                    anchor: row.get(2)?,
                    archived_at: row.get(3)?,
                    archive_reason: row.get(4)?,
                    kind: row.get(5)?,
                });
            }
        }
        Ok(rules)
    }

    fn topic_keys(&self, topic: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT key FROM memories WHERE archived_at IS NULL
             AND (key LIKE ?1 OR key = ?2) ORDER BY key LIMIT ?3;",
        )?;
        let mut rows = stmt.query(params![format!("{topic}%"), topic, limit as i64])?;
        let mut keys = Vec::new();
        while let Some(row) = rows.next()? {
            keys.push(row.get(0)?);
        }
        Ok(keys)
    }

    /// Combine task search with file routing while retaining bounded graph and
    /// repository-wide slots. The normal context methods remain the fast path
    /// when no task query or explanation was requested.
    pub fn context_for_task(
        &self,
        files: &[String],
        anchor: Option<&str>,
        topic: Option<&str>,
        query: Option<&str>,
        limit: usize,
        explain: bool,
    ) -> Result<TaskContext> {
        let requested_limit = if limit == 0 { 20 } else { limit };
        let clean_query = query.map(str::trim).filter(|value| !value.is_empty());
        if clean_query.is_none() && !explain {
            let (rules, relations, sessions) = if !files.is_empty() {
                self.context_for_files(files, topic, requested_limit)?
            } else {
                self.context_filtered(anchor, topic, requested_limit)?
            };
            return Ok(TaskContext {
                rules,
                relations,
                sessions,
                candidates: Vec::new(),
            });
        }

        let max_limit = requested_limit.min(512);
        let candidate_limit = max_limit.saturating_mul(4).clamp(max_limit, 512);
        let clean_topic = topic.map(str::trim).filter(|value| !value.is_empty());
        let active_files = if files.is_empty() {
            anchor
                .map(|value| vec![value.to_string()])
                .unwrap_or_default()
        } else {
            files.to_vec()
        };

        if clean_query.is_none() {
            let (rules, relations, sessions) = if !files.is_empty() {
                self.context_for_files(files, clean_topic, max_limit)?
            } else {
                self.context_filtered(anchor, clean_topic, max_limit)?
            };
            let (wider, _, _) = if !files.is_empty() {
                self.context_for_files(files, clean_topic, candidate_limit)?
            } else {
                self.context_filtered(anchor, clean_topic, candidate_limit)?
            };
            let direct: HashSet<String> = self
                .ranked_anchor_keys(&active_files, clean_topic, candidate_limit)?
                .into_iter()
                .collect();
            let universal: HashSet<String> = self
                .universal_keys(clean_topic, candidate_limit)?
                .into_iter()
                .collect();
            let mut seen = HashSet::new();
            let candidates = rules
                .iter()
                .chain(&wider)
                .filter(|rule| seen.insert(rule.key.as_str()))
                .map(|rule| {
                    let reason = if direct.contains(&rule.key) {
                        "file anchor"
                    } else if universal.contains(&rule.key) {
                        "repository-wide rule"
                    } else if !active_files.is_empty() {
                        "linked memory"
                    } else if clean_topic.is_some() {
                        "topic prefix"
                    } else {
                        "key order"
                    };
                    ContextCandidate {
                        key: rule.key.clone(),
                        reason: reason.to_string(),
                    }
                })
                .collect();
            return Ok(TaskContext {
                rules,
                relations,
                sessions,
                candidates,
            });
        }

        let query = clean_query.expect("query checked above");
        let task_keys: Vec<String> = self
            .find(query)?
            .into_iter()
            .filter(|rule| {
                !rule.is_archived() && clean_topic.is_none_or(|prefix| rule.key.starts_with(prefix))
            })
            .map(|rule| rule.key)
            .collect();
        if task_keys.is_empty() && active_files.is_empty() {
            return self.context_for_task(files, anchor, topic, None, limit, explain);
        }
        let file_keys = self.ranked_anchor_keys(&active_files, clean_topic, candidate_limit)?;
        let mut direct = Vec::with_capacity(task_keys.len() + file_keys.len());
        let mut direct_seen = HashSet::new();
        for index in 0..task_keys.len().max(file_keys.len()) {
            for key in [task_keys.get(index), file_keys.get(index)]
                .into_iter()
                .flatten()
            {
                if direct_seen.insert(key.as_str()) {
                    direct.push(key.clone());
                }
            }
        }

        let mut related_reasons = HashMap::new();
        let related_candidates: Vec<String> = if direct.is_empty() {
            Vec::new()
        } else {
            let keys: Vec<&str> = direct.iter().map(String::as_str).collect();
            let mut related = BTreeSet::new();
            for relation in self.get_relations_touching(&keys, candidate_limit)? {
                let (other, origin) = if direct_seen.contains(relation.source_key.as_str()) {
                    (&relation.target_key, &relation.source_key)
                } else {
                    (&relation.source_key, &relation.target_key)
                };
                if clean_topic.is_none_or(|prefix| other.starts_with(prefix)) {
                    if explain {
                        related_reasons.entry(other.clone()).or_insert_with(|| {
                            format!("linked from {origin} via {}", relation.rel_type)
                        });
                    }
                    related.insert(other.clone());
                }
            }
            related.into_iter().collect()
        };
        let mut related = Vec::with_capacity(related_candidates.len());
        if !related_candidates.is_empty() {
            let mut is_active = self.conn.prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE key = ?1 AND archived_at IS NULL);",
            )?;
            for key in related_candidates {
                if is_active.query_row(params![&key], |row| row.get::<_, bool>(0))? {
                    related.push(key);
                }
            }
        }
        let universal = match clean_topic {
            Some(prefix) => self.topic_keys(prefix, candidate_limit)?,
            None => self.universal_keys(None, candidate_limit)?,
        };
        let mut selected = Self::select_diverse_context_keys(
            direct.clone(),
            related.clone(),
            universal.clone(),
            max_limit,
        );
        if max_limit >= 2
            && !task_keys.is_empty()
            && let Some(file_key) = file_keys.first()
            && !selected.contains(file_key)
        {
            if selected.len() == max_limit {
                selected.pop();
            }
            selected.push(file_key.clone());
        }
        let rules = self.materialize_active_rules(&selected)?;
        let relations = if rules.is_empty() {
            Vec::new()
        } else {
            let keys: Vec<&str> = rules.iter().map(|rule| rule.key.as_str()).collect();
            self.get_relations_touching(&keys, max_limit.saturating_mul(4))?
        };
        let sessions = self.session_list(3)?;

        let mut candidates = Vec::new();
        if explain {
            let mut positions = HashMap::new();
            for (keys, reason) in [
                (&task_keys, "task query"),
                (&file_keys, "file anchor"),
                (&related, "linked memory"),
                (
                    &universal,
                    if clean_topic.is_some() {
                        "topic prefix"
                    } else {
                        "repository-wide rule"
                    },
                ),
            ] {
                for key in keys {
                    let reason = if reason == "linked memory" {
                        related_reasons
                            .get(key)
                            .map(String::as_str)
                            .unwrap_or(reason)
                    } else {
                        reason
                    };
                    if let Some(&index) = positions.get(key) {
                        let candidate: &mut ContextCandidate = &mut candidates[index];
                        candidate.reason.push_str(" + ");
                        candidate.reason.push_str(reason);
                    } else {
                        positions.insert(key.clone(), candidates.len());
                        candidates.push(ContextCandidate {
                            key: key.clone(),
                            reason: reason.to_string(),
                        });
                    }
                }
            }
        }
        Ok(TaskContext {
            rules,
            relations,
            sessions,
            candidates,
        })
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
                let candidate_limit = max_limit.saturating_mul(4).min(512).max(max_limit);
                let direct = self.ranked_anchor_keys(&[a], maybe_topic, candidate_limit)?;
                let keys: Vec<&str> = direct.iter().map(String::as_str).collect();
                let related = self
                    .get_outgoing_targets(&keys, candidate_limit)?
                    .into_iter()
                    .filter(|key| {
                        maybe_topic.is_none_or(|prefix| key.starts_with(prefix) || key == prefix)
                    })
                    .collect();
                let universal = self.universal_keys(maybe_topic, candidate_limit)?;
                let selected =
                    Self::select_diverse_context_keys(direct, related, universal, max_limit);
                self.materialize_active_rules(&selected)?
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
        let clean_topic = topic.map(str::trim).filter(|s| !s.is_empty());
        let candidate_limit = max_limit.saturating_mul(4).min(512).max(max_limit);
        let direct_keys = self.ranked_anchor_keys(files, clean_topic, candidate_limit)?;
        let mut direct_key_set = HashSet::with_capacity(candidate_limit.min(direct_keys.len()));
        for key in &direct_keys {
            direct_key_set.insert(key.clone());
        }
        let universal_keys = self.universal_keys(clean_topic, candidate_limit)?;

        // 2. Expand one graph hop through indexed source/target lookups.
        let mut related_keys = BTreeSet::new();
        if !direct_keys.is_empty() {
            let keys: Vec<&str> = direct_keys.iter().map(String::as_str).collect();
            let touching = self.get_relations_touching(&keys, candidate_limit)?;
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
        let selected_keys = Self::select_diverse_context_keys(
            direct_keys,
            related_keys.into_iter().collect(),
            universal_keys,
            max_limit,
        );

        // 4. Materialize only selected records.
        let combined_rules = self.materialize_active_rules(&selected_keys)?;

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

impl<'a> NormalizedQueryFile<'a> {
    fn new(clean_path: &'a str) -> Self {
        let (dir, base) = clean_path
            .rsplit_once('/')
            .map_or((None, clean_path), |(dir, base)| (Some(dir), base));
        Self {
            clean_path,
            dir,
            base,
            is_generic: is_generic_filename(base),
        }
    }
}

fn normalize_query_path(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_start_matches('@').trim().replace('\\', "/");
    let without_line = normalized
        .rsplit_once(':')
        .filter(|(_, line)| line.parse::<u32>().is_ok())
        .map_or(normalized.as_str(), |(path, _)| path);
    let clean = without_line.trim_start_matches("./").trim_matches('/');
    (!clean.is_empty()).then(|| clean.to_string())
}

fn is_generic_filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_matches('_');
    const GENERIC_STEMS: &[&str] = &[
        "mod",
        "lib",
        "main",
        "index",
        "init",
        "types",
        "utils",
        "helpers",
        "common",
        "config",
        "constants",
        "error",
        "errors",
        "conftest",
        "package",
        "build",
        "makefile",
        "dockerfile",
        "cmakelists",
    ];
    GENERIC_STEMS
        .iter()
        .any(|candidate| stem.eq_ignore_ascii_case(candidate))
}

#[inline]
fn has_path_boundary_suffix(path: &str, suffix: &str) -> bool {
    path.strip_suffix(suffix)
        .is_some_and(|prefix| prefix.ends_with('/'))
}

#[inline]
fn has_path_boundary_prefix(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn score_single_anchor(query: &NormalizedQueryFile<'_>, anchor_clean: &str) -> Option<u32> {
    // 1. Exact match (e.g. "src/api/mod.rs" == "src/api/mod.rs")
    if query.clean_path == anchor_clean {
        return Some(1000);
    }

    // 2. Relative suffix match (e.g. query "api/mod.rs" matches anchor "src/api/mod.rs"
    //    or query "src/api/mod.rs" matches anchor "api/mod.rs") without heap allocation.
    if has_path_boundary_suffix(query.clean_path, anchor_clean)
        || has_path_boundary_suffix(anchor_clean, query.clean_path)
    {
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

        // Parent/child subdirectory without intermediate Vec or String allocations
        if has_path_boundary_prefix(qd, ad) || has_path_boundary_prefix(ad, qd) {
            let shared = qd
                .split('/')
                .zip(ad.split('/'))
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
    } else if query.base.eq_ignore_ascii_case(anchor_base) {
        // Query was unqualified (e.g. query was just "auth.rs" or "user.rs")
        return Some(100);
    }

    None
}
