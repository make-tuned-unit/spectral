//! Concrete `MemoryStore` implementation using rusqlite with FTS5.
//!
//! # Memory mapping (mmap)
//!
//! By default, `SqliteStore` memory-maps the database file via SQLite's
//! `mmap_size` PRAGMA. This eliminates page cache eviction stalls that
//! otherwise produce 5–100 ms p99 latency outliers on multi-MB databases.
//!
//! The default mmap size adapts to the database file size at open time:
//!
//! - **Minimum:** 50 MB (covers small/empty brains)
//! - **Adaptive:** `file_size × 1.2` (20% headroom for growth)
//! - **Maximum:** 1 GB (cap for very large brains)
//!
//! Trade-offs:
//!
//! - **Memory pressure:** mmap'd pages count against process memory in
//!   utilities like `top`. On a 16 GB machine, mapping 1 GB is fine. On
//!   embedded systems with <512 MB RAM, consider disabling.
//! - **macOS:** mmap performance is excellent. **Linux:** also excellent.
//!   **Windows:** less consistent; consider testing if Windows is a target.
//! - **Brain growth past max:** if the database exceeds 1 GB, the portion
//!   beyond falls back to page cache behavior. Override `mmap_size`
//!   explicitly for very large brains.
//!
//! Override via [`SqliteStoreConfig::mmap_size`]:
//! - `None` (default): adaptive (50 MB – 1 GB)
//! - `Some(0)`: disable mmap entirely
//! - `Some(n)`: use exactly *n* bytes

use crate::{Fingerprint, Memory, MemoryHit, MemoryStore, SpectrogramRow};
use lru::LruCache;
use rusqlite::{params, Connection};
use std::future::Future;
use std::num::NonZeroUsize;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const WING_CACHE_CAPACITY: usize = 32;

/// Configuration for [`SqliteStore`].
#[derive(Debug, Clone, Default)]
pub struct SqliteStoreConfig {
    /// Maximum memory-map size for SQLite, in bytes.
    ///
    /// - `None` (default) — compute adaptively based on file size (50 MB – 1 GB).
    /// - `Some(0)` — disable mmap entirely (page cache only).
    /// - `Some(n)` — use exactly *n* bytes.
    pub mmap_size: Option<u64>,
}


/// SQLite-backed memory store with FTS5 search.
///
/// Includes an LRU cache for wing-scoped memory queries. The cache is
/// invalidated on writes that affect the cached wing.
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
    /// LRU cache: wing name -> memories for that wing.
    /// Invalidated by `write()` when the written memory's wing matches a cached entry.
    wing_cache: Arc<Mutex<LruCache<String, Vec<MemoryHit>>>>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish_non_exhaustive()
    }
}

impl SqliteStore {
    /// Compute an adaptive mmap size based on database file size.
    fn compute_mmap_size(db_path: &Path) -> u64 {
        const MIN_MMAP: u64 = 52_428_800; // 50 MB
        const MAX_MMAP: u64 = 1_073_741_824; // 1 GB
        const HEADROOM: f64 = 1.2; // 20% above current size

        match std::fs::metadata(db_path) {
            Ok(m) => {
                let target = (m.len() as f64 * HEADROOM) as u64;
                target.clamp(MIN_MMAP, MAX_MMAP)
            }
            Err(_) => MIN_MMAP, // fallback for new databases
        }
    }

    /// Open or create a memory database at the given path.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_with_config(path, &SqliteStoreConfig::default())
    }

    /// Open or create a memory database with explicit configuration.
    pub fn open_with_config(path: &Path, config: &SqliteStoreConfig) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;

        let mmap_size = match config.mmap_size {
            Some(explicit) => explicit,
            None => Self::compute_mmap_size(path),
        };

        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA temp_store   = MEMORY;
             PRAGMA mmap_size    = {mmap_size};"
        ))?;
        Self::init_schema(&conn)?;
        Self::migrate_provenance_columns(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wing_cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(WING_CACHE_CAPACITY).unwrap(),
            ))),
        })
    }

    /// Create an in-memory database (useful for tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Self::migrate_provenance_columns(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wing_cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(WING_CACHE_CAPACITY).unwrap(),
            ))),
        })
    }

    fn init_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id            TEXT PRIMARY KEY,
                key           TEXT NOT NULL UNIQUE,
                content       TEXT NOT NULL,
                category      TEXT NOT NULL DEFAULT 'core',
                wing          TEXT DEFAULT NULL,
                hall          TEXT DEFAULT NULL,
                signal_score  REAL DEFAULT 0.5,
                visibility    TEXT NOT NULL DEFAULT 'private',
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
                source        TEXT DEFAULT NULL,
                device_id     BLOB DEFAULT NULL,
                confidence    REAL NOT NULL DEFAULT 1.0
            );
            CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
            CREATE INDEX IF NOT EXISTS idx_memories_wing ON memories(wing);
            CREATE INDEX IF NOT EXISTS idx_memories_signal ON memories(signal_score);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                key, content, content=memories, content_rowid=rowid
            );

            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content)
                VALUES (new.rowid, new.key, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content)
                VALUES ('delete', old.rowid, old.key, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content)
                VALUES ('delete', old.rowid, old.key, old.content);
                INSERT INTO memories_fts(rowid, key, content)
                VALUES (new.rowid, new.key, new.content);
            END;

            CREATE TABLE IF NOT EXISTS constellation_fingerprints (
                id                TEXT PRIMARY KEY,
                fingerprint_hash  TEXT NOT NULL,
                anchor_memory_id  TEXT NOT NULL,
                target_memory_id  TEXT NOT NULL,
                wing              TEXT,
                anchor_hall       TEXT,
                target_hall       TEXT,
                time_delta_bucket TEXT,
                created_at        TEXT,
                FOREIGN KEY (anchor_memory_id) REFERENCES memories(id),
                FOREIGN KEY (target_memory_id) REFERENCES memories(id)
            );
            CREATE INDEX IF NOT EXISTS idx_fp_hash ON constellation_fingerprints(fingerprint_hash);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_hash
                ON constellation_fingerprints(wing, fingerprint_hash);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_anchor_hall
                ON constellation_fingerprints(wing, anchor_hall);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_target_hall
                ON constellation_fingerprints(wing, target_hall);

            CREATE TABLE IF NOT EXISTS memory_spectrogram (
                memory_id         TEXT PRIMARY KEY,
                entity_density    REAL,
                action_type       TEXT,
                decision_polarity REAL,
                causal_depth      REAL,
                emotional_valence REAL,
                temporal_specificity REAL,
                novelty           REAL,
                peak_dimensions   TEXT,
                created_at        TEXT DEFAULT (datetime('now')),
                FOREIGN KEY (memory_id) REFERENCES memories(id)
            );
            CREATE INDEX IF NOT EXISTS idx_spectrogram_action ON memory_spectrogram(action_type);",
        )?;
        Ok(())
    }

    /// Idempotent migration: adds source/device_id/confidence columns to
    /// existing databases that lack them.
    fn migrate_provenance_columns(conn: &Connection) -> anyhow::Result<()> {
        let mut has_source = false;
        let mut has_device_id = false;
        let mut has_confidence = false;

        let mut stmt = conn.prepare("PRAGMA table_info(memories)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows {
            match name?.as_str() {
                "source" => has_source = true,
                "device_id" => has_device_id = true,
                "confidence" => has_confidence = true,
                _ => {}
            }
        }

        if !has_source {
            conn.execute_batch("ALTER TABLE memories ADD COLUMN source TEXT DEFAULT NULL")?;
        }
        if !has_device_id {
            conn.execute_batch("ALTER TABLE memories ADD COLUMN device_id BLOB DEFAULT NULL")?;
        }
        if !has_confidence {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0",
            )?;
        }

        // Check for last_reinforced_at column (added for Memify feedback loop)
        let mut has_last_reinforced = false;
        let mut stmt2 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows2 = stmt2.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows2 {
            if name?.as_str() == "last_reinforced_at" {
                has_last_reinforced = true;
            }
        }
        if !has_last_reinforced {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN last_reinforced_at TEXT DEFAULT NULL",
            )?;
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

/// Standard column list for memory queries.
const MEMORY_COLUMNS: &str = "id, key, content, wing, hall, signal_score, visibility, source, device_id, confidence, created_at, last_reinforced_at";

/// Parse a Memory from a row with the standard column order.
/// Columns: id(0), key(1), content(2), wing(3), hall(4), signal_score(5),
/// visibility(6), source(7), device_id(8), confidence(9), created_at(10),
/// last_reinforced_at(11)
fn memory_from_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let device_blob: Option<Vec<u8>> = row.get(8)?;
    let device_id = device_blob.and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    Ok(Memory {
        id: row.get(0)?,
        key: row.get(1)?,
        content: row.get(2)?,
        wing: row.get(3)?,
        hall: row.get(4)?,
        signal_score: row.get(5)?,
        visibility: row.get::<_, String>(6).unwrap_or_else(|_| "private".into()),
        source: row.get(7)?,
        device_id,
        confidence: row.get::<_, f64>(9).unwrap_or(1.0),
        created_at: row.get(10).ok(),
        last_reinforced_at: row.get(11).ok(),
    })
}

fn memory_hit_from_row(row: &rusqlite::Row, hits: usize) -> rusqlite::Result<MemoryHit> {
    let device_blob: Option<Vec<u8>> = row.get(8)?;
    let device_id = device_blob.and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    Ok(MemoryHit {
        id: row.get(0)?,
        key: row.get(1)?,
        content: row.get(2)?,
        wing: row.get(3)?,
        hall: row.get(4)?,
        signal_score: row.get(5)?,
        visibility: row.get::<_, String>(6).unwrap_or_else(|_| "private".into()),
        hits,
        source: row.get(7)?,
        device_id,
        confidence: row.get::<_, f64>(9).unwrap_or(1.0),
        created_at: row.get(10).ok(),
        last_reinforced_at: row.get(11).ok(),
    })
}

impl MemoryStore for SqliteStore {
    fn write(
        &self,
        memory: &Memory,
        fingerprints: &[Fingerprint],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let memory = memory.clone();
        let fingerprints = fingerprints.to_vec();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let mut conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Wrap memory + all fingerprints in a single transaction for atomicity and performance.
            let tx = conn.transaction()?;

            tx.execute(
                "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility,
                                       source, device_id, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(key) DO UPDATE SET
                    content = excluded.content,
                    wing = excluded.wing,
                    hall = excluded.hall,
                    signal_score = excluded.signal_score,
                    visibility = excluded.visibility,
                    source = excluded.source,
                    device_id = excluded.device_id,
                    confidence = excluded.confidence,
                    updated_at = datetime('now')",
                params![
                    memory.id,
                    memory.key,
                    memory.content,
                    memory.wing,
                    memory.hall,
                    memory.signal_score,
                    memory.visibility,
                    memory.source,
                    memory.device_id.as_ref().map(|b| b.as_slice()),
                    memory.confidence,
                ],
            )?;

            for fp in &fingerprints {
                tx.execute(
                    "INSERT OR IGNORE INTO constellation_fingerprints
                     (id, fingerprint_hash, anchor_memory_id, target_memory_id,
                      wing, anchor_hall, target_hall, time_delta_bucket, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                    params![
                        fp.id,
                        fp.hash,
                        fp.anchor_memory_id,
                        fp.target_memory_id,
                        fp.wing,
                        fp.anchor_hall,
                        fp.target_hall,
                        fp.time_delta_bucket,
                    ],
                )?;
            }

            tx.commit()?;

            // Invalidate wing cache for the written memory's wing.
            if let Some(ref wing) = memory.wing {
                if let Ok(mut cache) = wing_cache.lock() {
                    cache.pop(wing);
                }
            }

            Ok(())
        })
    }

    fn list_wing_memories(
        &self,
        wing: &str,
        min_signal: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let wing = wing.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE wing = ?1 AND signal_score >= ?2
                 ORDER BY signal_score DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![wing, min_signal], memory_from_row)?;
            let mut memories = Vec::new();
            for row in rows {
                memories.push(row?);
            }
            Ok(memories)
        })
    }

    fn fingerprint_search(
        &self,
        wing: &str,
        hall: &str,
        hashes: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        let wing = wing.to_string();
        let hall = hall.to_string();
        let hashes = hashes.to_vec();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            if hashes.is_empty() {
                return Ok(Vec::new());
            }

            // Unified CTE: hash match + hall match in one query, scored server-side.
            let hash_placeholders: String = hashes
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 3))
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!(
                "WITH matched_pairs AS (
                    SELECT DISTINCT anchor_memory_id, target_memory_id
                    FROM constellation_fingerprints
                    WHERE wing = ?1 AND fingerprint_hash IN ({hash_placeholders})
                    UNION
                    SELECT DISTINCT anchor_memory_id, target_memory_id
                    FROM constellation_fingerprints
                    WHERE wing = ?1 AND (anchor_hall = ?2 OR target_hall = ?2)
                ),
                memory_scores AS (
                    SELECT memory_id, COUNT(*) AS hits FROM (
                        SELECT anchor_memory_id AS memory_id FROM matched_pairs
                        UNION ALL
                        SELECT target_memory_id AS memory_id FROM matched_pairs
                    )
                    GROUP BY memory_id
                    ORDER BY hits DESC
                    LIMIT ?{limit_param}
                )
                SELECT m.id, m.key, m.content, m.wing, m.hall, m.signal_score,
                       m.visibility, m.source, m.device_id, m.confidence,
                       m.created_at, m.last_reinforced_at, ms.hits
                FROM memory_scores ms
                JOIN memories m ON m.id = ms.memory_id
                ORDER BY ms.hits DESC",
                hash_placeholders = hash_placeholders,
                limit_param = hashes.len() + 3,
            );

            let mut stmt = conn.prepare_cached(&sql)?;

            // Bind parameters: ?1 = wing, ?2 = hall, ?3..N = hashes, ?N+1 = limit
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(wing));
            param_values.push(Box::new(hall));
            for h in &hashes {
                param_values.push(Box::new(h.clone()));
            }
            param_values.push(Box::new(max_results as i64));

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                let hits: i64 = row.get(12)?;
                memory_hit_from_row(row, hits as usize)
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }

            Ok(results)
        })
    }

    fn wing_search(
        &self,
        wing: &str,
        _query_terms: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        let wing = wing.to_string();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            // Check cache first
            if let Ok(mut cache) = wing_cache.lock() {
                if let Some(cached) = cache.get(&wing) {
                    let results: Vec<MemoryHit> =
                        cached.iter().take(max_results).cloned().collect();
                    return Ok(results);
                }
            }

            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE wing = ?1
                 ORDER BY signal_score DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![wing], |row| memory_hit_from_row(row, 0))?;
            let mut all_results = Vec::new();
            for row in rows {
                all_results.push(row?);
            }

            // Cache the full result set (not truncated) so different max_results can reuse
            if let Ok(mut cache) = wing_cache.lock() {
                cache.put(wing, all_results.clone());
            }

            all_results.truncate(max_results);
            Ok(all_results)
        })
    }

    fn fts_search(
        &self,
        query_words: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        let query = query_words.join(" OR ");
        let conn = self.conn.clone();

        Box::pin(async move {
            if query.is_empty() {
                return Ok(Vec::new());
            }
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT m.{cols}
                 FROM memories_fts fts
                 JOIN memories m ON m.rowid = fts.rowid
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
                cols = MEMORY_COLUMNS.replace(", ", ", m."),
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![query, max_results as i64], |row| {
                memory_hit_from_row(row, 0)
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    fn fetch_by_ids(
        &self,
        ids: &[String],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let ids = ids.to_vec();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut results = Vec::new();
            for id in &ids {
                let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = ?1");
                if let Ok(mem) = conn.query_row(&sql, params![id], memory_from_row) {
                    results.push(mem);
                }
            }
            Ok(results)
        })
    }

    fn reinforce_memory(
        &self,
        key: &str,
        strength: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<String>>> + Send + '_>> {
        let key = key.to_string();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Read current wing before update (for cache invalidation)
            let wing: Option<String> = conn
                .query_row(
                    "SELECT wing FROM memories WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .ok()
                .flatten();

            let updated = conn.execute(
                "UPDATE memories SET
                    signal_score = MIN(signal_score + ?1, 1.0),
                    last_reinforced_at = datetime('now'),
                    updated_at = datetime('now')
                 WHERE key = ?2",
                params![strength, key],
            )?;

            if updated == 0 {
                return Ok(None);
            }

            // Invalidate wing cache for the reinforced memory's wing
            if let Some(ref w) = wing {
                if let Ok(mut cache) = wing_cache.lock() {
                    cache.pop(w);
                }
            }

            Ok(wing)
        })
    }

    fn write_spectrogram(
        &self,
        memory_id: &str,
        entity_density: f64,
        action_type: &str,
        decision_polarity: f64,
        causal_depth: f64,
        emotional_valence: f64,
        temporal_specificity: f64,
        novelty: f64,
        peak_dimensions: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let action_type = action_type.to_string();
        let peak_dimensions = peak_dimensions.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "INSERT INTO memory_spectrogram
                     (memory_id, entity_density, action_type, decision_polarity,
                      causal_depth, emotional_valence, temporal_specificity, novelty,
                      peak_dimensions, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
                 ON CONFLICT(memory_id) DO UPDATE SET
                    entity_density = excluded.entity_density,
                    action_type = excluded.action_type,
                    decision_polarity = excluded.decision_polarity,
                    causal_depth = excluded.causal_depth,
                    emotional_valence = excluded.emotional_valence,
                    temporal_specificity = excluded.temporal_specificity,
                    novelty = excluded.novelty,
                    peak_dimensions = excluded.peak_dimensions",
                params![
                    memory_id,
                    entity_density,
                    action_type,
                    decision_polarity,
                    causal_depth,
                    emotional_valence,
                    temporal_specificity,
                    novelty,
                    peak_dimensions,
                ],
            )?;
            Ok(())
        })
    }

    fn load_spectrogram(
        &self,
        memory_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<SpectrogramRow>>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let result = conn.query_row(
                "SELECT s.memory_id, m.wing, s.entity_density, s.action_type,
                        s.decision_polarity, s.causal_depth, s.emotional_valence,
                        s.temporal_specificity, s.novelty, s.peak_dimensions
                 FROM memory_spectrogram s
                 JOIN memories m ON m.id = s.memory_id
                 WHERE s.memory_id = ?1",
                params![memory_id],
                |row| {
                    Ok(SpectrogramRow {
                        memory_id: row.get(0)?,
                        wing: row.get(1)?,
                        entity_density: row.get(2)?,
                        action_type: row.get(3)?,
                        decision_polarity: row.get(4)?,
                        causal_depth: row.get(5)?,
                        emotional_valence: row.get(6)?,
                        temporal_specificity: row.get(7)?,
                        novelty: row.get(8)?,
                        peak_dimensions: row.get(9)?,
                    })
                },
            );
            match result {
                Ok(row) => Ok(Some(row)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    fn load_spectrograms(
        &self,
        wing_filter: Option<&str>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<SpectrogramRow>>> + Send + '_>> {
        let wing_filter = wing_filter.map(String::from);
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut results = Vec::new();

            if let Some(ref wing) = wing_filter {
                let mut stmt = conn.prepare(
                    "SELECT s.memory_id, m.wing, s.entity_density, s.action_type,
                            s.decision_polarity, s.causal_depth, s.emotional_valence,
                            s.temporal_specificity, s.novelty, s.peak_dimensions
                     FROM memory_spectrogram s
                     JOIN memories m ON m.id = s.memory_id
                     WHERE m.wing = ?1
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![wing, limit as i64], |row| {
                    Ok(SpectrogramRow {
                        memory_id: row.get(0)?,
                        wing: row.get(1)?,
                        entity_density: row.get(2)?,
                        action_type: row.get(3)?,
                        decision_polarity: row.get(4)?,
                        causal_depth: row.get(5)?,
                        emotional_valence: row.get(6)?,
                        temporal_specificity: row.get(7)?,
                        novelty: row.get(8)?,
                        peak_dimensions: row.get(9)?,
                    })
                })?;
                for row in rows {
                    results.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT s.memory_id, m.wing, s.entity_density, s.action_type,
                            s.decision_polarity, s.causal_depth, s.emotional_valence,
                            s.temporal_specificity, s.novelty, s.peak_dimensions
                     FROM memory_spectrogram s
                     JOIN memories m ON m.id = s.memory_id
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], |row| {
                    Ok(SpectrogramRow {
                        memory_id: row.get(0)?,
                        wing: row.get(1)?,
                        entity_density: row.get(2)?,
                        action_type: row.get(3)?,
                        decision_polarity: row.get(4)?,
                        causal_depth: row.get(5)?,
                        emotional_valence: row.get(6)?,
                        temporal_specificity: row.get(7)?,
                        novelty: row.get(8)?,
                        peak_dimensions: row.get(9)?,
                    })
                })?;
                for row in rows {
                    results.push(row?);
                }
            }

            Ok(results)
        })
    }

    fn memories_without_spectrogram(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>> {
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT m.id FROM memories m
                 LEFT JOIN memory_spectrogram s ON m.id = s.memory_id
                 WHERE s.memory_id IS NULL
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_and_list() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "test_key".into(),
            content: "Jesse decided to use Clerk".into(),
            wing: Some("jesse".into()),
            hall: Some("fact".into()),
            signal_score: 0.85,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&mem, &[]).await.unwrap();

        let results = store.list_wing_memories("jesse", 0.5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");
    }

    #[tokio::test]
    async fn upsert_overwrites() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem1 = Memory {
            id: "m1".into(),
            key: "k".into(),
            content: "v1".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&mem1, &[]).await.unwrap();

        let mem2 = Memory {
            id: "m2".into(),
            key: "k".into(),
            content: "v2".into(),
            wing: Some("w".into()),
            hall: Some("discovery".into()),
            signal_score: 0.8,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&mem2, &[]).await.unwrap();

        let results = store.list_wing_memories("w", 0.0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "v2");
    }

    #[tokio::test]
    async fn fts5_works() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "auth_decision".into(),
            content: "Jesse decided to use Clerk for auth".into(),
            wing: Some("jesse".into()),
            hall: Some("fact".into()),
            signal_score: 0.85,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&mem, &[]).await.unwrap();

        let conn = store.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'Clerk'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn fingerprints_stored() {
        let store = SqliteStore::open_in_memory().unwrap();
        let m0 = Memory {
            id: "m0".into(),
            key: "k0".into(),
            content: "anchor".into(),
            wing: Some("w".into()),
            hall: Some("event".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&m0, &[]).await.unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "test".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        let fp = Fingerprint {
            id: "fp1".into(),
            hash: "abc123".into(),
            anchor_memory_id: "m0".into(),
            target_memory_id: "m1".into(),
            wing: "w".into(),
            anchor_hall: "event".into(),
            target_hall: "fact".into(),
            time_delta_bucket: "same_day".into(),
        };
        store.write(&mem, &[fp]).await.unwrap();

        let conn = store.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM constellation_fingerprints WHERE wing = 'w'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn idempotent_fingerprint_insert() {
        let store = SqliteStore::open_in_memory().unwrap();
        let m0 = Memory {
            id: "m0".into(),
            key: "k0".into(),
            content: "anchor".into(),
            wing: Some("w".into()),
            hall: Some("event".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        store.write(&m0, &[]).await.unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "test".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };
        let fp = Fingerprint {
            id: "fp1".into(),
            hash: "abc123".into(),
            anchor_memory_id: "m0".into(),
            target_memory_id: "m1".into(),
            wing: "w".into(),
            anchor_hall: "event".into(),
            target_hall: "fact".into(),
            time_delta_bucket: "same_day".into(),
        };
        store.write(&mem, std::slice::from_ref(&fp)).await.unwrap();
        store.write(&mem, std::slice::from_ref(&fp)).await.unwrap();

        let conn = store.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM constellation_fingerprints",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── Wing cache tests ─────────────────────────────────────────────

    fn make_mem(id: &str, key: &str, wing: &str) -> Memory {
        Memory {
            id: id.into(),
            key: key.into(),
            content: format!("content for {key}"),
            wing: Some(wing.into()),
            hall: Some("fact".into()),
            signal_score: 0.8,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        }
    }

    #[tokio::test]
    async fn wing_cache_serves_repeated_queries() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "polybot"), &[])
            .await
            .unwrap();

        // First call — cache miss, queries SQLite
        let r1 = store.wing_search("polybot", &[], 10).await.unwrap();
        assert_eq!(r1.len(), 1);

        // Second call — should hit cache (same result)
        let r2 = store.wing_search("polybot", &[], 10).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].id, r1[0].id);

        // Verify cache is populated
        let cache = store.wing_cache.lock().unwrap();
        assert!(cache.peek(&"polybot".to_string()).is_some());
    }

    #[tokio::test]
    async fn wing_cache_invalidated_on_write() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "polybot"), &[])
            .await
            .unwrap();

        // Populate cache
        let r1 = store.wing_search("polybot", &[], 10).await.unwrap();
        assert_eq!(r1.len(), 1);

        // Write to same wing — should invalidate cache
        store
            .write(&make_mem("m2", "k2", "polybot"), &[])
            .await
            .unwrap();

        // Cache entry should be gone
        {
            let cache = store.wing_cache.lock().unwrap();
            assert!(cache.peek(&"polybot".to_string()).is_none());
        }

        // Next query should see the new memory
        let r2 = store.wing_search("polybot", &[], 10).await.unwrap();
        assert_eq!(r2.len(), 2);
    }

    #[tokio::test]
    async fn wing_cache_size_bounded() {
        let store = SqliteStore::open_in_memory().unwrap();

        // Write memories for 33 different wings
        for i in 0..33 {
            let wing = format!("wing-{i}");
            store
                .write(&make_mem(&format!("m{i}"), &format!("k{i}"), &wing), &[])
                .await
                .unwrap();
            // Populate cache for this wing
            store.wing_search(&wing, &[], 10).await.unwrap();
        }

        // Cache should have at most WING_CACHE_CAPACITY entries
        let cache = store.wing_cache.lock().unwrap();
        assert!(cache.len() <= WING_CACHE_CAPACITY);
        // The oldest entry (wing-0) should have been evicted
        assert!(cache.peek(&"wing-0".to_string()).is_none());
        // The newest entry should still be present
        assert!(cache.peek(&"wing-32".to_string()).is_some());
    }

    #[tokio::test]
    async fn wing_cache_thread_safe() {
        use std::sync::Arc;

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());

        // Write memories for 4 wings
        for i in 0..4 {
            let wing = format!("wing-{i}");
            store
                .write(&make_mem(&format!("m{i}"), &format!("k{i}"), &wing), &[])
                .await
                .unwrap();
        }

        // Spawn 4 threads each querying a different wing
        let mut handles = Vec::new();
        for i in 0..4 {
            let store = Arc::clone(&store);
            let wing = format!("wing-{i}");
            handles.push(std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                for _ in 0..100 {
                    let result = rt.block_on(store.wing_search(&wing, &[], 10)).unwrap();
                    assert_eq!(result.len(), 1);
                }
            }));
        }

        for h in handles {
            h.join().expect("thread should not panic");
        }
    }

    // ── Compound index test ──────────────────────────────────────────

    #[tokio::test]
    async fn compound_hall_indexes_exist() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn();

        let mut stmt = conn
            .prepare("PRAGMA index_list(constellation_fingerprints)")
            .unwrap();
        let indexes: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(
            indexes.contains(&"idx_fp_wing_anchor_hall".to_string()),
            "missing idx_fp_wing_anchor_hall; found: {indexes:?}"
        );
        assert!(
            indexes.contains(&"idx_fp_wing_target_hall".to_string()),
            "missing idx_fp_wing_target_hall; found: {indexes:?}"
        );
    }

    // ── Transaction atomicity test ───────────────────────────────────

    #[tokio::test]
    async fn remember_writes_atomically() {
        let store = SqliteStore::open_in_memory().unwrap();

        let m0 = make_mem("m0", "k0", "w");
        store.write(&m0, &[]).await.unwrap();

        let mem = make_mem("m1", "k1", "w");
        let fps: Vec<Fingerprint> = (0..5)
            .map(|i| Fingerprint {
                id: format!("fp{i}"),
                hash: format!("hash{i}"),
                anchor_memory_id: "m0".into(),
                target_memory_id: "m1".into(),
                wing: "w".into(),
                anchor_hall: "event".into(),
                target_hall: "fact".into(),
                time_delta_bucket: "same_day".into(),
            })
            .collect();
        store.write(&mem, &fps).await.unwrap();

        // Verify all fingerprints + memory committed together
        let conn = store.conn();
        let mem_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        let fp_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM constellation_fingerprints",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 2); // m0 + m1
        assert_eq!(fp_count, 5); // all 5 fingerprints
    }
}
