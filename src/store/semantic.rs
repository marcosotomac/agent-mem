use super::{RuleRecord, SemanticBuildReport, SemanticStatus, Store, now_epoch};
use crate::error::{Error, Result};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const MODEL_ID: &str = "multilingual-e5-small";
const DIMENSIONS: usize = 384;
const MODEL_MAX_TOKENS: usize = 256;
const BUILD_BATCH_SIZE: usize = 128;
const QUERY_CACHE_CAPACITY: usize = 256;
const ANN_CANDIDATES: usize = 24;
const MAX_RESULTS: usize = 5;
const MAX_COSINE_DISTANCE: f32 = 1.0;

static MODEL: Mutex<Option<TextEmbedding>> = Mutex::new(None);

pub(crate) struct SemanticRuntime {
    generation: Option<String>,
    index: Option<Index>,
    query_cache: VecDeque<(String, Vec<f32>)>,
}

impl Default for SemanticRuntime {
    fn default() -> Self {
        Self {
            generation: None,
            index: None,
            query_cache: VecDeque::with_capacity(QUERY_CACHE_CAPACITY),
        }
    }
}

#[derive(Clone)]
struct SemanticDocument {
    id: u64,
    key: String,
    text: String,
    fingerprint_hi: i64,
    fingerprint_lo: i64,
}

struct SemanticState {
    generation: String,
    model_id: String,
    dimensions: usize,
    records_count: usize,
    built_at: i64,
    dirty: bool,
}

fn semantic_error(error: impl std::fmt::Display) -> Error {
    Error::Semantic(error.to_string())
}

fn configured_threads() -> usize {
    env::var("AGENT_MEM_SEMANTIC_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .min(4)
        })
}

fn model_cache_dir() -> PathBuf {
    if let Some(path) = env::var_os("AGENT_MEM_MODEL_DIR") {
        return PathBuf::from(path);
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("agent-mem")
        .join("models")
}

fn with_model<T>(operation: impl FnOnce(&mut TextEmbedding) -> Result<T>) -> Result<T> {
    let mut guard = MODEL
        .lock()
        .map_err(|_| Error::Semantic("embedding model lock was poisoned".into()))?;
    if guard.is_none() {
        let cache_dir = model_cache_dir();
        fs::create_dir_all(&cache_dir)?;
        let is_tty = std::io::stderr().is_terminal() && env::var("AGENT_MEM_QUIET").is_err();
        let model_dir = cache_dir.join("models--intfloat--multilingual-e5-small");
        let needs_download = !model_dir.exists();

        if is_tty && needs_download {
            eprintln!(
                "  \x1b[38;5;150m↓\x1b[0m  \x1b[38;5;245mdownloading local embedding model\x1b[0m \x1b[1;37m{MODEL_ID}\x1b[0m \x1b[38;5;240m(first-time setup, cached globally)\x1b[0m"
            );
        }

        let options = TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
            .with_cache_dir(cache_dir)
            .with_max_length(MODEL_MAX_TOKENS)
            .with_intra_threads(configured_threads())
            .with_show_download_progress(is_tty);
        let embedding = TextEmbedding::try_new(options).map_err(semantic_error)?;

        if is_tty && needs_download {
            eprintln!("  \x1b[38;5;150m✓\x1b[0m  \x1b[38;5;245mmodel ready\x1b[0m");
        }

        *guard = Some(embedding);
    }
    operation(guard.as_mut().expect("embedding model initialized"))
}

fn bounded_chars(input: &str, limit: usize) -> String {
    input.chars().take(limit).collect()
}

fn embedding_text(key: &str, val: &str, anchor: Option<&str>, kind: &str) -> String {
    let normalized_key = key.replace(['/', '-', '_', ':'], " ");
    let mut text = String::with_capacity(normalized_key.len() + val.len().min(4096) + 32);
    text.push_str("passage: ");
    text.push_str(kind);
    text.push(' ');
    text.push_str(&normalized_key);
    text.push(' ');
    text.push_str(&bounded_chars(val, 4096));
    if let Some(anchor) = anchor {
        text.push(' ');
        text.push_str(&bounded_chars(anchor, 512));
    }
    text
}

fn update_hash(hash: &mut u64, bytes: &[u8]) {
    *hash ^= bytes.len() as u64;
    *hash = hash.wrapping_mul(0x100000001b3);
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn fingerprint(key: &str, val: &str, anchor: Option<&str>, kind: &str) -> (i64, i64) {
    let mut hi = 0xcbf29ce484222325u64;
    let mut lo = 0x9e3779b97f4a7c15u64;
    for field in [key, val, anchor.unwrap_or_default(), kind] {
        update_hash(&mut hi, field.as_bytes());
        for byte in field.as_bytes().iter().rev() {
            lo ^= u64::from(*byte);
            lo = lo.wrapping_mul(0x517cc1b727220a95);
        }
        lo ^= field.len() as u64;
    }
    (hi as i64, lo as i64)
}

fn update_corpus_fingerprint(corpus: &mut (u64, u64), fingerprint: (i64, i64)) {
    update_hash(&mut corpus.0, &fingerprint.0.to_le_bytes());
    update_hash(&mut corpus.1, &fingerprint.1.to_le_bytes());
}

fn normalize_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn index_file(parent: &Path, generation: &str) -> PathBuf {
    parent.join(format!("semantic-{generation}.usearch"))
}

fn path_for_usearch(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::Semantic("semantic index path is not valid UTF-8".into()))
}

fn new_index(capacity: usize) -> Result<Index> {
    let options = IndexOptions {
        dimensions: DIMENSIONS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: 16,
        expansion_add: 64,
        expansion_search: 48,
        multi: false,
    };
    let index = Index::new(&options).map_err(semantic_error)?;
    if capacity > 0 {
        index
            .reserve_capacity_and_threads(capacity, configured_threads())
            .map_err(semantic_error)?;
    }
    Ok(index)
}

fn embed_batch(model: &mut TextEmbedding, index: &Index, batch: &[SemanticDocument]) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let texts: Vec<&str> = batch
        .iter()
        .map(|document| document.text.as_str())
        .collect();
    let embeddings = model
        .embed(texts, Some(BUILD_BATCH_SIZE))
        .map_err(semantic_error)?;
    if embeddings.len() != batch.len() {
        return Err(Error::Semantic(format!(
            "embedding model returned {} vectors for {} documents",
            embeddings.len(),
            batch.len()
        )));
    }
    for (document, vector) in batch.iter().zip(embeddings) {
        if vector.len() != DIMENSIONS {
            return Err(Error::Semantic(format!(
                "embedding dimension mismatch: expected {DIMENSIONS}, got {}",
                vector.len()
            )));
        }
        index.add(document.id, &vector).map_err(semantic_error)?;
    }
    Ok(())
}

impl Store {
    fn semantic_parent(&self) -> Result<&Path> {
        self.db_path
            .as_deref()
            .and_then(Path::parent)
            .ok_or_else(|| Error::Semantic("semantic indexes require a file-backed store".into()))
    }

    fn semantic_state(&self) -> Result<Option<SemanticState>> {
        self.conn
            .query_row(
                "SELECT generation, model_id, dimensions, records_count, built_at, dirty
                 FROM semantic_state WHERE singleton = 1;",
                [],
                |row| {
                    Ok(SemanticState {
                        generation: row.get(0)?,
                        model_id: row.get(1)?,
                        dimensions: row.get(2)?,
                        records_count: row.get(3)?,
                        built_at: row.get(4)?,
                        dirty: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn semantic_rebuild(&mut self) -> Result<SemanticBuildReport> {
        let started = Instant::now();
        let parent = self.semantic_parent()?.to_path_buf();
        fs::create_dir_all(&parent)?;
        let records_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived_at IS NULL;",
            [],
            |row| row.get(0),
        )?;
        let index = new_index(records_count)?;
        let mut mappings = Vec::with_capacity(records_count);
        let mut corpus = (0x243f6a8885a308d3, 0x13198a2e03707344);

        if records_count > 0 {
            if std::io::stderr().is_terminal() && env::var("AGENT_MEM_QUIET").is_err() {
                eprintln!(
                    "  \x1b[38;5;150m⟳\x1b[0m  \x1b[38;5;245mindexing semantic embeddings for\x1b[0m \x1b[1;37m{} memories\x1b[0m...",
                    records_count
                );
            }
            with_model(|model| {
                let mut stmt = self.conn.prepare(
                    "SELECT key, val, anchor, kind FROM memories
                     WHERE archived_at IS NULL ORDER BY key;",
                )?;
                let mut rows = stmt.query([])?;
                let mut batch = Vec::with_capacity(BUILD_BATCH_SIZE);
                let mut id = 1u64;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    let val: String = row.get(1)?;
                    let anchor: Option<String> = row.get(2)?;
                    let kind: String = row.get(3)?;
                    let (fingerprint_hi, fingerprint_lo) =
                        fingerprint(&key, &val, anchor.as_deref(), &kind);
                    update_corpus_fingerprint(&mut corpus, (fingerprint_hi, fingerprint_lo));
                    let document = SemanticDocument {
                        id,
                        text: embedding_text(&key, &val, anchor.as_deref(), &kind),
                        key,
                        fingerprint_hi,
                        fingerprint_lo,
                    };
                    mappings.push((
                        document.id,
                        document.key.clone(),
                        document.fingerprint_hi,
                        document.fingerprint_lo,
                    ));
                    batch.push(document);
                    id += 1;
                    if batch.len() == BUILD_BATCH_SIZE {
                        embed_batch(model, &index, &batch)?;
                        batch.clear();
                    }
                }
                embed_batch(model, &index, &batch)
            })?;
        }

        let generation = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            std::process::id()
        );
        let final_path = index_file(&parent, &generation);
        let temp_path = parent.join(format!(".semantic-{generation}.tmp"));
        index
            .save(path_for_usearch(&temp_path)?)
            .map_err(semantic_error)?;
        fs::File::open(&temp_path)?.sync_all()?;
        fs::rename(&temp_path, &final_path)?;

        let old_generation = self.semantic_state()?.map(|state| state.generation);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let verified = {
            let mut verified_corpus = (0x243f6a8885a308d3, 0x13198a2e03707344);
            let mut verified_count = 0usize;
            let mut stmt = tx.prepare(
                "SELECT key, val, anchor, kind FROM memories
                 WHERE archived_at IS NULL ORDER BY key;",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let val: String = row.get(1)?;
                let anchor: Option<String> = row.get(2)?;
                let kind: String = row.get(3)?;
                update_corpus_fingerprint(
                    &mut verified_corpus,
                    fingerprint(&key, &val, anchor.as_deref(), &kind),
                );
                verified_count += 1;
            }
            verified_corpus == corpus && verified_count == mappings.len()
        };
        if !verified {
            drop(tx);
            let _ = fs::remove_file(&final_path);
            return Err(Error::Semantic(
                "memories changed while the semantic index was being built; retry the build".into(),
            ));
        }

        tx.execute("DELETE FROM semantic_records;", [])?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO semantic_records
                 (semantic_id, memory_key, fingerprint_hi, fingerprint_lo)
                 VALUES (?1, ?2, ?3, ?4);",
            )?;
            for (id, key, fingerprint_hi, fingerprint_lo) in &mappings {
                insert.execute(params![*id as i64, key, fingerprint_hi, fingerprint_lo])?;
            }
        }
        tx.execute(
            "INSERT INTO semantic_state
             (singleton, generation, model_id, dimensions, records_count, built_at, dirty)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, 0)
             ON CONFLICT(singleton) DO UPDATE SET
                generation = excluded.generation,
                model_id = excluded.model_id,
                dimensions = excluded.dimensions,
                records_count = excluded.records_count,
                built_at = excluded.built_at,
                dirty = 0;",
            params![
                generation,
                MODEL_ID,
                DIMENSIONS as i64,
                mappings.len() as i64,
                now_epoch()
            ],
        )?;
        if let Err(error) = tx.commit() {
            let _ = fs::remove_file(&final_path);
            return Err(error.into());
        }

        if let Some(old_generation) = old_generation
            && old_generation != generation
        {
            let _ = fs::remove_file(index_file(&parent, &old_generation));
        }
        *self.semantic_runtime.borrow_mut() = SemanticRuntime::default();
        let index_bytes = fs::metadata(&final_path)?.len();
        Ok(SemanticBuildReport {
            records_count: mappings.len(),
            elapsed_ms: started.elapsed().as_millis(),
            index_path: final_path,
            index_bytes,
            model_id: MODEL_ID.to_string(),
        })
    }

    pub fn semantic_status(&self) -> Result<SemanticStatus> {
        let Some(state) = self.semantic_state()? else {
            return Ok(SemanticStatus {
                enabled: false,
                dirty: false,
                model_id: None,
                records_count: 0,
                built_at: None,
                index_path: None,
                index_bytes: 0,
            });
        };
        let path = index_file(self.semantic_parent()?, &state.generation);
        let index_bytes = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(SemanticStatus {
            enabled: path.is_file() && state.model_id == MODEL_ID && state.dimensions == DIMENSIONS,
            dirty: state.dirty,
            model_id: Some(state.model_id),
            records_count: state.records_count,
            built_at: Some(state.built_at),
            index_path: Some(path),
            index_bytes,
        })
    }

    pub fn semantic_clear(&mut self) -> Result<usize> {
        let parent = self.semantic_parent()?.to_path_buf();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM semantic_records;", [])?;
        tx.execute("DELETE FROM semantic_state;", [])?;
        tx.commit()?;
        *self.semantic_runtime.borrow_mut() = SemanticRuntime::default();

        let mut removed = 0usize;
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if ((name.starts_with("semantic-") && name.ends_with(".usearch"))
                    || (name.starts_with(".semantic-") && name.ends_with(".tmp")))
                    && fs::remove_file(entry.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn query_embedding(&self, query: &str) -> Result<Vec<f32>> {
        let cache_key = normalize_query(query);
        {
            let mut runtime = self.semantic_runtime.borrow_mut();
            if let Some(position) = runtime
                .query_cache
                .iter()
                .position(|(cached, _)| cached == &cache_key)
            {
                let entry = runtime
                    .query_cache
                    .remove(position)
                    .expect("cached query position exists");
                let vector = entry.1.clone();
                runtime.query_cache.push_front(entry);
                return Ok(vector);
            }
        }

        let query = format!("query: {}", bounded_chars(query, 1024));
        let vector = with_model(|model| {
            let mut embeddings = model
                .embed([query.as_str()], Some(1))
                .map_err(semantic_error)?;
            let vector = embeddings.pop().ok_or_else(|| {
                Error::Semantic("embedding model returned no query vector".into())
            })?;
            if vector.len() != DIMENSIONS {
                return Err(Error::Semantic(format!(
                    "embedding dimension mismatch: expected {DIMENSIONS}, got {}",
                    vector.len()
                )));
            }
            Ok(vector)
        })?;

        let mut runtime = self.semantic_runtime.borrow_mut();
        runtime.query_cache.push_front((cache_key, vector.clone()));
        runtime.query_cache.truncate(QUERY_CACHE_CAPACITY);
        Ok(vector)
    }

    pub(crate) fn find_semantic(&self, query: &str) -> Result<Vec<RuleRecord>> {
        let Some(state) = self.semantic_state()? else {
            return Ok(Vec::new());
        };
        if state.model_id != MODEL_ID || state.dimensions != DIMENSIONS || state.records_count == 0
        {
            return Ok(Vec::new());
        }
        let path = index_file(self.semantic_parent()?, &state.generation);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let query_vector = self.query_embedding(query)?;

        let matches = {
            let mut runtime = self.semantic_runtime.borrow_mut();
            if runtime.generation.as_deref() != Some(state.generation.as_str()) {
                let index =
                    Index::restore_view(path_for_usearch(&path)?).map_err(semantic_error)?;
                runtime.index = Some(index);
                runtime.generation = Some(state.generation.clone());
            }
            runtime
                .index
                .as_ref()
                .expect("semantic index initialized")
                .search(&query_vector, ANN_CANDIDATES.min(state.records_count))
                .map_err(semantic_error)?
        };

        let mut results = Vec::with_capacity(MAX_RESULTS);
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.key, m.val, m.anchor, m.archived_at, m.archive_reason, m.kind,
                    sr.fingerprint_hi, sr.fingerprint_lo
             FROM semantic_records sr
             JOIN memories m ON m.key = sr.memory_key
             WHERE sr.semantic_id = ?1 AND m.archived_at IS NULL
             LIMIT 1;",
        )?;
        for (id, distance) in matches.keys.iter().zip(matches.distances.iter()) {
            if *distance > MAX_COSINE_DISTANCE {
                continue;
            }
            let record = stmt
                .query_row(params![*id as i64], |row| {
                    let rule = RuleRecord {
                        key: row.get(0)?,
                        val: row.get(1)?,
                        anchor: row.get(2)?,
                        archived_at: row.get(3)?,
                        archive_reason: row.get(4)?,
                        kind: row.get(5)?,
                    };
                    let fingerprint_hi: i64 = row.get(6)?;
                    let fingerprint_lo: i64 = row.get(7)?;
                    Ok((rule, fingerprint_hi, fingerprint_lo))
                })
                .optional()?;
            if let Some((rule, fingerprint_hi, fingerprint_lo)) = record
                && fingerprint(&rule.key, &rule.val, rule.anchor.as_deref(), &rule.kind)
                    == (fingerprint_hi, fingerprint_lo)
            {
                results.push(rule);
                if results.len() == MAX_RESULTS {
                    break;
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_include_all_semantic_fields() {
        let base = fingerprint("key", "value", Some("src/lib.rs"), "decision");
        assert_ne!(
            base,
            fingerprint("other", "value", Some("src/lib.rs"), "decision")
        );
        assert_ne!(
            base,
            fingerprint("key", "other", Some("src/lib.rs"), "decision")
        );
        assert_ne!(
            base,
            fingerprint("key", "value", Some("src/main.rs"), "decision")
        );
        assert_ne!(
            base,
            fingerprint("key", "value", Some("src/lib.rs"), "rule")
        );
    }

    #[test]
    fn query_normalization_is_cache_stable() {
        assert_eq!(normalize_query("  COBRAR   Dos Veces "), "cobrar dos veces");
    }
}
