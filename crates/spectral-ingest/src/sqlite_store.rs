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

use crate::{
    CompactionTier, Episode, Fingerprint, Memory, MemoryAnnotation, MemoryHit, MemoryStore,
    RelatedMemory, RetrievalEvent, SpectrogramRow, WriteOutcome,
};
use lru::LruCache;
use rusqlite::{params, Connection};
use std::future::Future;
use std::num::NonZeroUsize;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::TimeBucket;

const WING_CACHE_CAPACITY: usize = 32;

/// Parse a timestamp string (SQLite datetime or RFC3339) to epoch seconds.
fn parse_ts(s: &str) -> Option<f64> {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp() as f64);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp() as f64);
    }
    None
}

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
                key, content, description,
                content=memories, content_rowid=rowid
            );

            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content, description)
                VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
                VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
                VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
                INSERT INTO memories_fts(rowid, key, content, description)
                VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
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
            CREATE INDEX IF NOT EXISTS idx_spectrogram_action ON memory_spectrogram(action_type);

            CREATE TABLE IF NOT EXISTS episodes (
                id             TEXT PRIMARY KEY,
                started_at     TEXT NOT NULL,
                ended_at       TEXT NOT NULL,
                memory_count   INTEGER NOT NULL DEFAULT 0,
                wing           TEXT NOT NULL,
                summary_preview TEXT,
                created_at     TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at     TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_started_at ON episodes(started_at);
            CREATE INDEX IF NOT EXISTS idx_episodes_wing ON episodes(wing);

            CREATE TABLE IF NOT EXISTS memory_annotations (
                id          TEXT PRIMARY KEY,
                memory_id   TEXT NOT NULL,
                description TEXT NOT NULL,
                who         TEXT NOT NULL,
                why         TEXT NOT NULL,
                where_      TEXT,
                when_       TEXT NOT NULL,
                how         TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_annotations_memory_id
                ON memory_annotations(memory_id);",
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

        // episode_id column
        let mut has_episode_id = false;
        let mut stmt3 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows3 = stmt3.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows3 {
            if name?.as_str() == "episode_id" {
                has_episode_id = true;
            }
        }
        if !has_episode_id {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN episode_id TEXT DEFAULT NULL;
                 CREATE INDEX IF NOT EXISTS idx_memories_episode_id ON memories(episode_id);",
            )?;
        }

        // retrieval_events table (recall→recognition feedback loop)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS retrieval_events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                query_hash      TEXT NOT NULL,
                timestamp       TEXT NOT NULL,
                memory_ids_json TEXT NOT NULL,
                method          TEXT NOT NULL,
                wing            TEXT,
                question_type   TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_retrieval_events_ts
                ON retrieval_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_retrieval_events_query_hash
                ON retrieval_events(query_hash);",
        )?;

        // declarative_density column
        let mut has_declarative_density = false;
        let mut stmt5 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows5 = stmt5.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows5 {
            if name?.as_str() == "declarative_density" {
                has_declarative_density = true;
            }
        }
        if !has_declarative_density {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN declarative_density REAL DEFAULT NULL",
            )?;
        }

        // compaction_tier column
        let mut has_compaction_tier = false;
        let mut stmt4 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows4 = stmt4.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows4 {
            if name?.as_str() == "compaction_tier" {
                has_compaction_tier = true;
            }
        }
        if !has_compaction_tier {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN compaction_tier TEXT DEFAULT NULL",
            )?;
        }

        // description + description_generated_at columns
        let mut has_description = false;
        let mut has_description_generated_at = false;
        let mut stmt6 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows6 = stmt6.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows6 {
            match name?.as_str() {
                "description" => has_description = true,
                "description_generated_at" => has_description_generated_at = true,
                _ => {}
            }
        }
        if !has_description {
            conn.execute_batch("ALTER TABLE memories ADD COLUMN description TEXT DEFAULT NULL")?;
        }
        if !has_description_generated_at {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN description_generated_at TEXT DEFAULT NULL",
            )?;
        }

        // co_retrieval_pairs table
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS co_retrieval_pairs (
                memory_id_a TEXT NOT NULL,
                memory_id_b TEXT NOT NULL,
                co_count    INTEGER NOT NULL DEFAULT 0,
                last_updated TEXT NOT NULL,
                PRIMARY KEY (memory_id_a, memory_id_b)
            );
            CREATE INDEX IF NOT EXISTS idx_co_retrieval_a
                ON co_retrieval_pairs(memory_id_a);
            CREATE INDEX IF NOT EXISTS idx_co_retrieval_b
                ON co_retrieval_pairs(memory_id_b);",
        )?;

        // session_id column on retrieval_events
        let mut has_session_id = false;
        let mut stmt7 = conn.prepare("PRAGMA table_info(retrieval_events)")?;
        let rows7 = stmt7.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows7 {
            if name?.as_str() == "session_id" {
                has_session_id = true;
            }
        }
        if !has_session_id {
            conn.execute_batch(
                "ALTER TABLE retrieval_events ADD COLUMN session_id TEXT DEFAULT NULL;
                 CREATE INDEX IF NOT EXISTS idx_retrieval_events_session
                     ON retrieval_events(session_id);",
            )?;
        }

        // content_hash column for write dedup
        let mut has_content_hash = false;
        let mut stmt8 = conn.prepare("PRAGMA table_info(memories)")?;
        let rows8 = stmt8.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows8 {
            if name?.as_str() == "content_hash" {
                has_content_hash = true;
            }
        }
        if !has_content_hash {
            conn.execute_batch("ALTER TABLE memories ADD COLUMN content_hash TEXT DEFAULT NULL")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memories_content_hash ON memories(content_hash)",
        )?;

        // FTS5 migration: add description column to FTS virtual table.
        // FTS5 does not support ALTER TABLE, so we detect and drop+recreate.
        let fts_has_description = {
            let fts_sql: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'memories_fts'",
                    [],
                    |row| row.get(0),
                )
                .ok();
            fts_sql
                .as_deref()
                .map_or(false, |sql| sql.contains("description"))
        };
        if !fts_has_description {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS memories_ai;
                 DROP TRIGGER IF EXISTS memories_ad;
                 DROP TRIGGER IF EXISTS memories_au;
                 DROP TABLE IF EXISTS memories_fts;

                 CREATE VIRTUAL TABLE memories_fts USING fts5(
                     key, content, description,
                     content=memories, content_rowid=rowid
                 );

                 CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
                     INSERT INTO memories_fts(rowid, key, content, description)
                     VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
                 END;
                 CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
                     INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
                     VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
                 END;
                 CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
                     INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
                     VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
                     INSERT INTO memories_fts(rowid, key, content, description)
                     VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
                 END;

                 INSERT INTO memories_fts(rowid, key, content, description)
                 SELECT rowid, key, content, COALESCE(description, '') FROM memories;",
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
const MEMORY_COLUMNS: &str = "id, key, content, wing, hall, signal_score, visibility, source, device_id, confidence, created_at, last_reinforced_at, episode_id, compaction_tier, declarative_density, description, description_generated_at, content_hash";

/// Parse a Memory from a row with the standard column order.
/// Columns: id(0), key(1), content(2), wing(3), hall(4), signal_score(5),
/// visibility(6), source(7), device_id(8), confidence(9), created_at(10),
/// last_reinforced_at(11), episode_id(12), compaction_tier(13),
/// declarative_density(14), description(15), description_generated_at(16)
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
        episode_id: row.get(12).ok(),
        compaction_tier: row
            .get::<_, String>(13)
            .ok()
            .and_then(|s| crate::CompactionTier::parse(&s)),
        declarative_density: row.get(14).ok(),
        description: row.get(15).ok(),
        description_generated_at: row.get(16).ok(),
        content_hash: row.get(17).ok(),
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
        episode_id: row.get(12).ok(),
        declarative_density: row.get(14).ok(),
        description: row.get(15).ok(),
    })
}

impl MemoryStore for SqliteStore {
    fn write(
        &self,
        memory: &Memory,
        fingerprints: &[Fingerprint],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WriteOutcome>> + Send + '_>> {
        let memory = memory.clone();
        let fingerprints = fingerprints.to_vec();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let mut conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Compute content hash of incoming content.
            let incoming_hash = blake3::hash(memory.content.as_bytes()).to_hex().to_string();

            // Wrap in a single transaction for atomicity.
            let tx = conn.transaction()?;

            // Probe for existing row.
            let existing: Option<(Option<String>, String)> = tx
                .query_row(
                    "SELECT content_hash, content FROM memories WHERE key = ?1",
                    params![memory.key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            let outcome = match existing {
                None => {
                    // Case 1: No existing row — insert.
                    if memory.created_at.is_some() {
                        tx.execute(
                            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility,
                                                   source, device_id, confidence, created_at, content_hash)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                                memory.created_at,
                                incoming_hash,
                            ],
                        )?;
                    } else {
                        tx.execute(
                            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility,
                                                   source, device_id, confidence, content_hash)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                                incoming_hash,
                            ],
                        )?;
                    }

                    // Set episode_id if provided
                    if let Some(ref ep_id) = memory.episode_id {
                        tx.execute(
                            "UPDATE memories SET episode_id = ?1 WHERE id = ?2",
                            params![ep_id, memory.id],
                        )?;
                    }
                    // Set compaction_tier if provided
                    if let Some(tier) = memory.compaction_tier {
                        tx.execute(
                            "UPDATE memories SET compaction_tier = ?1 WHERE id = ?2",
                            params![tier.as_str(), memory.id],
                        )?;
                    }
                    // Set declarative_density if provided
                    if let Some(dd) = memory.declarative_density {
                        tx.execute(
                            "UPDATE memories SET declarative_density = ?1 WHERE id = ?2",
                            params![dd, memory.id],
                        )?;
                    }

                    // Write fingerprints for new memory.
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

                    WriteOutcome::Inserted
                }
                Some((existing_hash, existing_content)) => {
                    // Resolve effective hash: use stored hash, or compute from existing content if NULL.
                    let effective_existing_hash = existing_hash.unwrap_or_else(|| {
                        blake3::hash(existing_content.as_bytes())
                            .to_hex()
                            .to_string()
                    });

                    if effective_existing_hash == incoming_hash {
                        // Case 2: Same content — true no-op. Preserve all fields.
                        // Backfill content_hash if it was NULL.
                        tx.execute(
                            "UPDATE memories SET content_hash = ?1 WHERE key = ?2 AND content_hash IS NULL",
                            params![incoming_hash, memory.key],
                        )?;
                        // Skip fingerprint rewrites entirely.
                        WriteOutcome::NoOp
                    } else {
                        // Case 3: Content differs — update content only, preserve everything else.
                        tx.execute(
                            "UPDATE memories SET content = ?1, content_hash = ?2, updated_at = datetime('now') WHERE key = ?3",
                            params![memory.content, incoming_hash, memory.key],
                        )?;

                        // Rewrite fingerprints (content changed, so fingerprints may differ).
                        // Get the memory id for the existing row.
                        let mem_id: String = tx.query_row(
                            "SELECT id FROM memories WHERE key = ?1",
                            params![memory.key],
                            |row| row.get(0),
                        )?;
                        // Delete old fingerprints for this memory.
                        tx.execute(
                            "DELETE FROM constellation_fingerprints WHERE anchor_memory_id = ?1 OR target_memory_id = ?1",
                            params![mem_id],
                        )?;
                        // Write new fingerprints.
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

                        WriteOutcome::ContentUpdated
                    }
                }
            };

            tx.commit()?;

            // Invalidate wing cache for the written memory's wing.
            if outcome != WriteOutcome::NoOp {
                if let Some(ref wing) = memory.wing {
                    if let Ok(mut cache) = wing_cache.lock() {
                        cache.pop(wing);
                    }
                }
            }

            Ok(outcome)
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

    fn list_memories_by_signal(
        &self,
        min_signal: f64,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE signal_score >= ?1 \
                 ORDER BY signal_score DESC LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![min_signal, limit as i64], memory_from_row)?;
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
                       m.created_at, m.last_reinforced_at, ms.hits,
                       m.declarative_density
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
                let mut hit = memory_hit_from_row(row, hits as usize)?;
                // Column 13 in this query is declarative_density (not compaction_tier)
                hit.declarative_density = row.get(13).ok();
                Ok(hit)
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
        let query = query_words
            .iter()
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
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
                 ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) LIMIT ?2",
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

    fn list_wing_memories_since(
        &self,
        wing: &str,
        since: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let wing = wing.to_string();
        let since = since.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories \
                 WHERE wing = ?1 AND created_at > ?2 \
                 ORDER BY created_at DESC LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![wing, since, limit as i64], memory_from_row)?;
            let mut memories = Vec::new();
            for row in rows {
                memories.push(row?);
            }
            Ok(memories)
        })
    }

    fn delete_wing_memories_before(
        &self,
        wing: &str,
        before: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let wing = wing.to_string();
        let before = before.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let deleted = conn.execute(
                "DELETE FROM memories WHERE wing = ?1 AND created_at < ?2",
                params![wing, before],
            )?;
            Ok(deleted)
        })
    }

    fn prune_wing_keeping_recent_per_source(
        &self,
        wing: &str,
        keep: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let wing = wing.to_string();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Get distinct sources in this wing
            let mut src_stmt = conn.prepare(
                "SELECT DISTINCT source FROM memories WHERE wing = ?1 AND source IS NOT NULL",
            )?;
            let sources: Vec<String> = src_stmt
                .query_map(params![wing], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();

            let mut total_deleted = 0;
            for source in &sources {
                let deleted = conn.execute(
                    "DELETE FROM memories WHERE wing = ?1 AND source = ?2 AND id NOT IN (\
                         SELECT id FROM memories WHERE wing = ?1 AND source = ?2 \
                         ORDER BY created_at DESC LIMIT ?3\
                     )",
                    params![wing, source, keep as i64],
                )?;
                total_deleted += deleted;
            }
            Ok(total_deleted)
        })
    }

    fn write_episode(
        &self,
        episode: &Episode,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let episode = episode.clone();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "INSERT INTO episodes (id, started_at, ended_at, memory_count, wing, summary_preview, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
                 ON CONFLICT(id) DO UPDATE SET
                    ended_at = excluded.ended_at,
                    memory_count = excluded.memory_count,
                    summary_preview = excluded.summary_preview,
                    updated_at = datetime('now')",
                params![
                    episode.id,
                    episode.started_at,
                    episode.ended_at,
                    episode.memory_count as i64,
                    episode.wing,
                    episode.summary_preview,
                ],
            )?;
            Ok(())
        })
    }

    fn find_recent_episode(
        &self,
        wing: &str,
        since: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Episode>>> + Send + '_>> {
        let wing = wing.to_string();
        let since = since.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT id, started_at, ended_at, memory_count, wing, summary_preview
                 FROM episodes WHERE wing = ?1 AND ended_at > ?2
                 ORDER BY ended_at DESC LIMIT 1",
            )?;
            let episode = stmt
                .query_row(params![wing, since], |row| {
                    Ok(Episode {
                        id: row.get(0)?,
                        started_at: row.get(1)?,
                        ended_at: row.get(2)?,
                        memory_count: row.get::<_, i64>(3)? as usize,
                        wing: row.get(4)?,
                        summary_preview: row.get(5)?,
                    })
                })
                .ok();
            Ok(episode)
        })
    }

    fn list_episodes(
        &self,
        wing: Option<&str>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Episode>>> + Send + '_>> {
        let wing = wing.map(|s| s.to_string());
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            fn episode_from_row(row: &rusqlite::Row) -> rusqlite::Result<Episode> {
                Ok(Episode {
                    id: row.get(0)?,
                    started_at: row.get(1)?,
                    ended_at: row.get(2)?,
                    memory_count: row.get::<_, i64>(3)? as usize,
                    wing: row.get(4)?,
                    summary_preview: row.get(5)?,
                })
            }

            let episodes = if let Some(ref w) = wing {
                let mut stmt = conn.prepare(
                    "SELECT id, started_at, ended_at, memory_count, wing, summary_preview
                     FROM episodes WHERE wing = ?1 ORDER BY ended_at DESC LIMIT ?2",
                )?;
                let v: Vec<Episode> = stmt
                    .query_map(params![w, limit as i64], episode_from_row)?
                    .filter_map(|r| r.ok())
                    .collect();
                v
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, started_at, ended_at, memory_count, wing, summary_preview
                     FROM episodes ORDER BY ended_at DESC LIMIT ?1",
                )?;
                let v: Vec<Episode> = stmt
                    .query_map(params![limit as i64], episode_from_row)?
                    .filter_map(|r| r.ok())
                    .collect();
                v
            };
            Ok(episodes)
        })
    }

    fn list_memories_by_episode(
        &self,
        episode_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let episode_id = episode_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE episode_id = ?1 ORDER BY created_at"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mems = stmt
                .query_map(params![episode_id], memory_from_row)?
                .filter_map(|r| r.ok())
                .collect();
            Ok(mems)
        })
    }

    fn write_annotation(
        &self,
        annotation: &MemoryAnnotation,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let annotation = annotation.clone();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let when_rfc = annotation.when_.to_rfc3339();

            // Idempotent on (memory_id, description, when_): if an identical
            // annotation already exists, skip the insert.
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM memory_annotations
                     WHERE memory_id = ?1 AND description = ?2 AND when_ = ?3",
                    params![annotation.memory_id, annotation.description, when_rfc],
                    |row| row.get(0),
                )
                .ok();

            if existing.is_some() {
                return Ok(());
            }

            let who_json = serde_json::to_string(&annotation.who)?;
            conn.execute(
                "INSERT INTO memory_annotations
                 (id, memory_id, description, who, why, where_, when_, how, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    annotation.id,
                    annotation.memory_id,
                    annotation.description,
                    who_json,
                    annotation.why,
                    annotation.where_,
                    when_rfc,
                    annotation.how,
                    annotation.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    fn list_annotations(
        &self,
        memory_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryAnnotation>>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT id, memory_id, description, who, why, where_, when_, how, created_at
                 FROM memory_annotations WHERE memory_id = ?1 ORDER BY created_at",
            )?;
            let annotations: Vec<MemoryAnnotation> = stmt
                .query_map(params![memory_id], |row| {
                    let who_json: String = row.get(3)?;
                    let who: Vec<crate::EntityRef> =
                        serde_json::from_str(&who_json).unwrap_or_default();
                    let when_str: String = row.get(6)?;
                    let when_ = chrono::DateTime::parse_from_rfc3339(&when_str)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());
                    let created_str: String = row.get(8)?;
                    let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());
                    Ok(MemoryAnnotation {
                        id: row.get(0)?,
                        memory_id: row.get(1)?,
                        description: row.get(2)?,
                        who,
                        why: row.get(4)?,
                        where_: row.get(5)?,
                        when_,
                        how: row.get(7)?,
                        created_at,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(annotations)
        })
    }

    fn set_declarative_density(
        &self,
        memory_id: &str,
        density: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "UPDATE memories SET declarative_density = ?1 WHERE id = ?2",
                params![density, memory_id],
            )?;
            Ok(())
        })
    }

    fn set_compaction_tier(
        &self,
        memory_id: &str,
        tier: CompactionTier,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let tier_str = tier.as_str().to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "UPDATE memories SET compaction_tier = ?1 WHERE id = ?2",
                params![tier_str, memory_id],
            )?;
            Ok(())
        })
    }

    fn backfill_fingerprint_time_buckets(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Fetch all fingerprints with their anchor/target memory timestamps
            let mut stmt = conn.prepare(
                "SELECT f.id, m_anchor.created_at, m_target.created_at
                 FROM constellation_fingerprints f
                 JOIN memories m_anchor ON m_anchor.id = f.anchor_memory_id
                 JOIN memories m_target ON m_target.id = f.target_memory_id
                 WHERE f.time_delta_bucket = 'unknown' OR f.time_delta_bucket IS NULL",
            )?;

            let rows: Vec<(String, Option<String>, Option<String>)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            let mut updated = 0;
            let mut update_stmt = conn.prepare(
                "UPDATE constellation_fingerprints
                 SET time_delta_bucket = ?1,
                     fingerprint_hash = ?2
                 WHERE id = ?3",
            )?;

            for (fp_id, anchor_ts, target_ts) in &rows {
                let bucket = match (anchor_ts.as_deref(), target_ts.as_deref()) {
                    (Some(a), Some(t)) => {
                        let a_secs = parse_ts(a);
                        let t_secs = parse_ts(t);
                        match (a_secs, t_secs) {
                            (Some(a), Some(t)) => TimeBucket::from_delta_secs(a - t),
                            _ => TimeBucket::Older,
                        }
                    }
                    _ => TimeBucket::Older,
                };

                // Also need to recompute the hash with the new bucket.
                // Fetch anchor_hall, target_hall, wing for this fingerprint.
                let fp_meta: (String, String, String) = conn.query_row(
                    "SELECT anchor_hall, target_hall, wing FROM constellation_fingerprints WHERE id = ?1",
                    params![fp_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;

                let new_hash = {
                    use sha2::{Digest, Sha256};
                    let raw = format!(
                        "{}|{}|{}|{}",
                        fp_meta.0,
                        fp_meta.1,
                        fp_meta.2,
                        bucket.as_str()
                    );
                    let digest = Sha256::digest(raw.as_bytes());
                    format!(
                        "{:016x}",
                        u64::from_be_bytes(digest[..8].try_into().unwrap())
                    )
                };

                update_stmt.execute(params![bucket.as_str(), new_hash, fp_id])?;
                updated += 1;
            }

            Ok(updated)
        })
    }

    fn log_retrieval_event(
        &self,
        event: &RetrievalEvent,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let query_hash = event.query_hash.clone();
        let timestamp = event.timestamp.clone();
        let memory_ids_json = event.memory_ids_json.clone();
        let method = event.method.clone();
        let wing = event.wing.clone();
        let question_type = event.question_type.clone();
        let session_id = event.session_id.clone();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "INSERT INTO retrieval_events \
                    (query_hash, timestamp, memory_ids_json, method, wing, question_type, session_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    query_hash,
                    timestamp,
                    memory_ids_json,
                    method,
                    wing,
                    question_type,
                    session_id
                ],
            )?;
            Ok(())
        })
    }

    fn count_retrieval_events(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM retrieval_events", [], |row| {
                    row.get(0)
                })?;
            Ok(count as usize)
        })
    }

    fn count_retrieval_events_by_method(
        &self,
        method: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let method = method.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM retrieval_events WHERE method = ?1",
                params![method],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    fn get_memory(
        &self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Memory>>> + Send + '_>> {
        let id = id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = ?1");
            match conn.query_row(&sql, params![id], memory_from_row) {
                Ok(mem) => Ok(Some(mem)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    fn set_description(
        &self,
        id: &str,
        description: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let id = id.to_string();
        let description = description.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let rows_affected = conn.execute(
                "UPDATE memories SET description = ?1, \
                     description_generated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
                 WHERE id = ?2",
                params![description, id],
            )?;
            if rows_affected == 0 {
                anyhow::bail!("memory not found: {id}");
            }
            Ok(())
        })
    }

    fn list_undescribed(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories \
                 WHERE description IS NULL \
                 ORDER BY created_at DESC LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows: Vec<Memory> = stmt
                .query_map(params![limit as i64], memory_from_row)?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
    }

    fn related_memories(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RelatedMemory>>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT memory_id_b AS related_id, co_count FROM co_retrieval_pairs \
                     WHERE memory_id_a = ?1 \
                 UNION ALL \
                 SELECT memory_id_a AS related_id, co_count FROM co_retrieval_pairs \
                     WHERE memory_id_b = ?1 \
                 ORDER BY co_count DESC LIMIT ?2",
            )?;
            let rows: Vec<RelatedMemory> = stmt
                .query_map(params![memory_id, limit as i64], |row| {
                    Ok(RelatedMemory {
                        memory_id: row.get(0)?,
                        co_count: row.get::<_, i64>(1)? as u64,
                        memory: None,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
    }

    /// Rebuild is wrapped in a transaction so the DELETE + INSERT sequence
    /// is atomic — concurrent `related_memories` queries never see an empty table.
    fn rebuild_co_retrieval_index(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let mut conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Read all retrieval events (before the transaction — read-only)
            let mut stmt = conn.prepare("SELECT memory_ids_json FROM retrieval_events")?;
            let events: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);

            // Aggregate co-retrieval counts
            let mut pair_counts: std::collections::HashMap<(String, String), i64> =
                std::collections::HashMap::new();

            for json in &events {
                let ids: Vec<String> = match serde_json::from_str(json) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // Generate all pairs (i, j) where i < j lexicographically
                for i in 0..ids.len() {
                    for j in (i + 1)..ids.len() {
                        let (a, b) = if ids[i] < ids[j] {
                            (ids[i].clone(), ids[j].clone())
                        } else {
                            (ids[j].clone(), ids[i].clone())
                        };
                        *pair_counts.entry((a, b)).or_insert(0) += 1;
                    }
                }
            }

            // Atomic truncate-and-rewrite
            let tx = conn.transaction()?;

            tx.execute("DELETE FROM co_retrieval_pairs", [])?;

            let now = chrono::Utc::now().to_rfc3339();
            {
                let mut insert_stmt = tx.prepare(
                    "INSERT INTO co_retrieval_pairs \
                         (memory_id_a, memory_id_b, co_count, last_updated) \
                     VALUES (?1, ?2, ?3, ?4)",
                )?;

                for ((a, b), count) in &pair_counts {
                    insert_stmt.execute(params![a, b, count, now])?;
                }
            }

            tx.commit()?;

            Ok(pair_counts.len())
        })
    }

    fn events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RetrievalEvent>>> + Send + '_>> {
        let session_id = session_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT query_hash, timestamp, memory_ids_json, method, wing, question_type, session_id \
                 FROM retrieval_events WHERE session_id = ?1 \
                 ORDER BY timestamp ASC LIMIT ?2",
            )?;
            let rows: Vec<RetrievalEvent> = stmt
                .query_map(params![session_id, limit as i64], |row| {
                    Ok(RetrievalEvent {
                        query_hash: row.get(0)?,
                        timestamp: row.get(1)?,
                        memory_ids_json: row.get(2)?,
                        method: row.get(3)?,
                        wing: row.get(4)?,
                        question_type: row.get(5)?,
                        session_id: row.get(6)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
    }

    fn memories_for_session(
        &self,
        session_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>> {
        let session_id = session_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT memory_ids_json FROM retrieval_events \
                 WHERE session_id = ?1 ORDER BY timestamp ASC",
            )?;
            let jsons: Vec<String> = stmt
                .query_map(params![session_id], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();

            let mut seen = std::collections::HashSet::new();
            let mut result = Vec::new();
            for json in &jsons {
                let ids: Vec<String> = match serde_json::from_str(json) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for id in ids {
                    if seen.insert(id.clone()) {
                        result.push(id);
                    }
                }
            }
            Ok(result)
        })
    }

    fn backfill_content_hashes(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt =
                conn.prepare("SELECT id, content FROM memories WHERE content_hash IS NULL")?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            let count = rows.len();
            for (id, content) in &rows {
                let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
                conn.execute(
                    "UPDATE memories SET content_hash = ?1 WHERE id = ?2",
                    params![hash, id],
                )?;
            }
            Ok(count)
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
            content: "Alice decided to use Clerk".into(),
            wing: Some("alice".into()),
            hall: Some("fact".into()),
            signal_score: 0.85,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        let results = store.list_wing_memories("alice", 0.5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");
    }

    #[tokio::test]
    async fn upsert_overwrites_content_preserves_signal() {
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome1 = store.write(&mem1, &[]).await.unwrap();
        assert_eq!(outcome1, WriteOutcome::Inserted);

        // Reinforce to bump signal_score
        store.reinforce_memory("k", 0.2).await.unwrap();

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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome2 = store.write(&mem2, &[]).await.unwrap();
        assert_eq!(outcome2, WriteOutcome::ContentUpdated);

        let results = store.list_wing_memories("w", 0.0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "v2");
        // signal_score should be preserved (0.6 + 0.2 = 0.8 from reinforce), not overwritten
        assert!(
            (results[0].signal_score - 0.8).abs() < 0.01,
            "signal_score should be preserved at 0.8, got {}",
            results[0].signal_score
        );
    }

    #[tokio::test]
    async fn fts5_works() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "auth_decision".into(),
            content: "Alice decided to use Clerk for auth".into(),
            wing: Some("alice".into()),
            hall: Some("fact".into()),
            signal_score: 0.85,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
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
    async fn fts_matches_description_text() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "session_abc:turn:0:user".into(),
            content: "I saw Dr. Patel for my sinusitis follow-up".into(),
            wing: Some("general".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        // "doctors" should NOT match content (only "Dr." is there)
        let results = store.fts_search(&["doctors".into()], 10).await.unwrap();
        assert!(results.is_empty(), "should not match without description");

        // Add description with category-level vocabulary
        store
            .set_description(
                "m1",
                "User visits doctors including ENT specialist Dr. Patel",
            )
            .await
            .unwrap();

        // "doctors" should now match via the description column
        let results = store.fts_search(&["doctors".into()], 10).await.unwrap();
        assert_eq!(results.len(), 1, "should match via description");
        assert_eq!(results[0].id, "m1");
    }

    #[tokio::test]
    async fn fts_description_null_does_not_break_search() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "I love hiking in the mountains".into(),
            wing: None,
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        // Content match should still work when description is NULL
        let results = store.fts_search(&["hiking".into()], 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
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
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        }
    }

    #[tokio::test]
    async fn wing_cache_serves_repeated_queries() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "apollo"), &[])
            .await
            .unwrap();

        // First call — cache miss, queries SQLite
        let r1 = store.wing_search("apollo", &[], 10).await.unwrap();
        assert_eq!(r1.len(), 1);

        // Second call — should hit cache (same result)
        let r2 = store.wing_search("apollo", &[], 10).await.unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].id, r1[0].id);

        // Verify cache is populated
        let cache = store.wing_cache.lock().unwrap();
        assert!(cache.peek(&"apollo".to_string()).is_some());
    }

    #[tokio::test]
    async fn wing_cache_invalidated_on_write() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "apollo"), &[])
            .await
            .unwrap();

        // Populate cache
        let r1 = store.wing_search("apollo", &[], 10).await.unwrap();
        assert_eq!(r1.len(), 1);

        // Write to same wing — should invalidate cache
        store
            .write(&make_mem("m2", "k2", "apollo"), &[])
            .await
            .unwrap();

        // Cache entry should be gone
        {
            let cache = store.wing_cache.lock().unwrap();
            assert!(cache.peek(&"apollo".to_string()).is_none());
        }

        // Next query should see the new memory
        let r2 = store.wing_search("apollo", &[], 10).await.unwrap();
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

    // ── Episode storage tests ───────────────────────────────────────

    #[tokio::test]
    async fn write_episode_persists_episode() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ep = Episode {
            id: "ep-1".into(),
            started_at: "2023-06-15 10:00:00".into(),
            ended_at: "2023-06-15 10:30:00".into(),
            memory_count: 3,
            wing: "general".into(),
            summary_preview: Some("Discussed project architecture".into()),
        };
        store.write_episode(&ep).await.unwrap();

        let episodes = store.list_episodes(None, 100).await.unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, "ep-1");
        assert_eq!(episodes[0].memory_count, 3);
        assert_eq!(episodes[0].wing, "general");
        assert_eq!(
            episodes[0].summary_preview.as_deref(),
            Some("Discussed project architecture")
        );
    }

    #[tokio::test]
    async fn list_memories_by_episode_returns_constituents() {
        let store = SqliteStore::open_in_memory().unwrap();

        let ep = Episode {
            id: "ep-mem-test".into(),
            started_at: "2023-06-15 10:00:00".into(),
            ended_at: "2023-06-15 10:30:00".into(),
            memory_count: 3,
            wing: "general".into(),
            summary_preview: None,
        };
        store.write_episode(&ep).await.unwrap();

        for i in 0..3 {
            let mem = Memory {
                id: format!("em{i}"),
                key: format!("ep-key-{i}"),
                content: format!("Episode memory content {i}"),
                wing: Some("general".into()),
                hall: Some("fact".into()),
                signal_score: 0.7,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: Some("ep-mem-test".into()),
                compaction_tier: None,
                declarative_density: None,
                description: None,
                description_generated_at: None,
                content_hash: None,
            };
            store.write(&mem, &[]).await.unwrap();
        }

        let mems = store.list_memories_by_episode("ep-mem-test").await.unwrap();
        assert_eq!(mems.len(), 3);
        for m in &mems {
            assert_eq!(m.episode_id.as_deref(), Some("ep-mem-test"));
        }
    }

    #[tokio::test]
    async fn find_recent_episode_finds_episode_in_window() {
        let store = SqliteStore::open_in_memory().unwrap();

        // Episode ended 10 minutes ago
        let now = chrono::Utc::now();
        let ended = (now - chrono::Duration::minutes(10))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let started = (now - chrono::Duration::minutes(40))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let since = (now - chrono::Duration::minutes(30))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let ep = Episode {
            id: "ep-recent".into(),
            started_at: started,
            ended_at: ended,
            memory_count: 5,
            wing: "general".into(),
            summary_preview: None,
        };
        store.write_episode(&ep).await.unwrap();

        let found = store.find_recent_episode("general", &since).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "ep-recent");
    }

    #[tokio::test]
    async fn find_recent_episode_excludes_episode_outside_window() {
        let store = SqliteStore::open_in_memory().unwrap();

        // Episode ended 60 minutes ago
        let now = chrono::Utc::now();
        let ended = (now - chrono::Duration::minutes(60))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let started = (now - chrono::Duration::minutes(90))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let since = (now - chrono::Duration::minutes(30))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let ep = Episode {
            id: "ep-old".into(),
            started_at: started,
            ended_at: ended,
            memory_count: 5,
            wing: "general".into(),
            summary_preview: None,
        };
        store.write_episode(&ep).await.unwrap();

        let found = store.find_recent_episode("general", &since).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn episode_auto_detected_within_time_gap() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();

        // Ingest two memories quickly (same wing, no explicit episode_id)
        crate::ingest::ingest_with(
            "m1",
            "k1",
            "First memory about project design",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        )
        .await
        .unwrap();

        crate::ingest::ingest_with(
            "m2",
            "k2",
            "Second memory about project design details",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        )
        .await
        .unwrap();

        // Both should share the same episode
        let mems: Vec<Memory> = {
            let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories ORDER BY id");
            let conn = store.conn();
            let mut stmt = conn.prepare(&sql).unwrap();
            stmt.query_map([], memory_from_row)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(mems.len(), 2);
        assert!(mems[0].episode_id.is_some());
        assert_eq!(mems[0].episode_id, mems[1].episode_id);
    }

    #[tokio::test]
    async fn episode_consumer_provided_id_used() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Memory with explicit episode",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts {
                episode_id: Some("my-session".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Drop the MutexGuard before calling async store methods
        let ep_id: String = {
            let conn = store.conn();
            conn.query_row("SELECT episode_id FROM memories WHERE id = 'm1'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(ep_id, "my-session");

        let episodes = store.list_episodes(None, 100).await.unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, "my-session");
    }

    #[tokio::test]
    async fn episode_consumer_provided_id_joins_existing() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();
        let opts = || IngestOpts {
            episode_id: Some("shared-ep".into()),
            ..Default::default()
        };

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "First in shared episode",
            "core",
            0.0,
            "private",
            &config,
            &store,
            opts(),
        )
        .await
        .unwrap();

        crate::ingest::ingest_with(
            "m2",
            "k2",
            "Second in shared episode",
            "core",
            0.0,
            "private",
            &config,
            &store,
            opts(),
        )
        .await
        .unwrap();

        let mems = store.list_memories_by_episode("shared-ep").await.unwrap();
        assert_eq!(mems.len(), 2);

        let episodes = store.list_episodes(None, 100).await.unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].memory_count, 2);
    }

    #[tokio::test]
    async fn memory_hit_includes_episode_id_after_recall() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Memory with episode for recall test",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts {
                episode_id: Some("ep-recall".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let hits = store
            .fts_search(&["episode".into(), "recall".into()], 10)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].episode_id.as_deref(), Some("ep-recall"));
    }

    #[tokio::test]
    async fn auto_detected_episode_uses_correct_wing() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        // Use wing rules that classify "work" content into "work" wing
        let config = IngestConfig {
            wing_rules: vec![(
                regex::Regex::new("work|project|deploy").unwrap(),
                "work".into(),
            )],
            ..Default::default()
        };

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Work project deploy task",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        )
        .await
        .unwrap();

        let episodes = store.list_episodes(Some("work"), 100).await.unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].wing, "work");
    }

    #[tokio::test]
    async fn find_recent_episode_only_searches_same_wing() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();

        // Create an episode in "work" wing
        let ep = Episode {
            id: "ep-work".into(),
            started_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            ended_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            memory_count: 1,
            wing: "work".into(),
            summary_preview: None,
        };
        store.write_episode(&ep).await.unwrap();

        // Ingest into "personal" wing — should NOT join "work" episode
        let config = IngestConfig {
            wing_rules: vec![(
                regex::Regex::new("personal|hobby").unwrap(),
                "personal".into(),
            )],
            ..Default::default()
        };

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Personal hobby activity",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        )
        .await
        .unwrap();

        // Check that the personal memory got a different episode
        let mem: Memory = {
            let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
            let conn = store.conn();
            conn.query_row(&sql, [], memory_from_row).unwrap()
        };
        assert_ne!(mem.episode_id.as_deref(), Some("ep-work"));
    }

    // ── Annotation + compaction_tier tests ───────────────────────────

    #[tokio::test]
    async fn write_and_list_annotation() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test content"), &[])
            .await
            .unwrap();

        let ann = MemoryAnnotation {
            id: "ann-1".into(),
            memory_id: "m1".into(),
            description: "Team standup discussion".into(),
            who: vec![
                crate::EntityRef {
                    canonical_id: "person:jesse-sharratt".into(),
                    display_name: "Jesse Sharratt".into(),
                },
                crate::EntityRef {
                    canonical_id: "project:permagent".into(),
                    display_name: "Permagent".into(),
                },
            ],
            why: "Reviewing sprint progress".into(),
            where_: Some("office".into()),
            when_: chrono::Utc::now(),
            how: "Verbal discussion in standup".into(),
            created_at: chrono::Utc::now(),
        };
        store.write_annotation(&ann).await.unwrap();

        let annotations = store.list_annotations("m1").await.unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].id, "ann-1");
        assert_eq!(annotations[0].description, "Team standup discussion");
        assert_eq!(annotations[0].who.len(), 2);
        assert_eq!(annotations[0].who[0].canonical_id, "person:jesse-sharratt");
        assert_eq!(annotations[0].who[1].display_name, "Permagent");
    }

    #[tokio::test]
    async fn annotation_idempotent_on_same_content() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        let when_ = chrono::Utc::now();
        let ann = MemoryAnnotation {
            id: "ann-idem-1".into(),
            memory_id: "m1".into(),
            description: "Same description".into(),
            who: vec![],
            why: "same why".into(),
            where_: None,
            when_,
            how: "manual".into(),
            created_at: chrono::Utc::now(),
        };
        store.write_annotation(&ann).await.unwrap();

        // Second call with identical (memory_id, description, when_) but different id
        let ann2 = MemoryAnnotation {
            id: "ann-idem-2".into(), // different id
            memory_id: "m1".into(),
            description: "Same description".into(), // same
            who: vec![],
            why: "same why".into(),
            where_: None,
            when_, // same
            how: "manual".into(),
            created_at: chrono::Utc::now(),
        };
        store.write_annotation(&ann2).await.unwrap();

        // Should still be exactly one row — second was a no-op
        let annotations = store.list_annotations("m1").await.unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].id, "ann-idem-1");
    }

    #[tokio::test]
    async fn list_annotations_empty_for_no_annotations() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();
        let annotations = store.list_annotations("m1").await.unwrap();
        assert!(annotations.is_empty());
    }

    #[tokio::test]
    async fn set_compaction_tier_persists() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        store
            .set_compaction_tier("m1", CompactionTier::HourlyRollup)
            .await
            .unwrap();

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
        let conn = store.conn();
        let mem: Memory = conn.query_row(&sql, [], memory_from_row).unwrap();
        assert_eq!(mem.compaction_tier, Some(CompactionTier::HourlyRollup));
    }

    #[tokio::test]
    async fn compaction_tier_defaults_to_none() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
        let conn = store.conn();
        let mem: Memory = conn.query_row(&sql, [], memory_from_row).unwrap();
        assert!(mem.compaction_tier.is_none());
    }

    #[tokio::test]
    async fn compaction_tier_invalid_string_produces_none() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        // Simulate an external write of an invalid tier string
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE memories SET compaction_tier = 'bogus_tier' WHERE id = 'm1'",
                [],
            )
            .unwrap();
        }

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
        let conn = store.conn();
        let mem: Memory = conn.query_row(&sql, [], memory_from_row).unwrap();
        assert!(
            mem.compaction_tier.is_none(),
            "invalid tier string should parse as None"
        );
    }

    #[tokio::test]
    async fn entity_ref_serde_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        let ann = MemoryAnnotation {
            id: "ann-serde".into(),
            memory_id: "m1".into(),
            description: "Test serde".into(),
            who: vec![
                crate::EntityRef {
                    canonical_id: "did:chitin:jesse-sharratt".into(),
                    display_name: "Jesse".into(),
                },
                crate::EntityRef {
                    canonical_id: "project:spectral".into(),
                    display_name: "Spectral".into(),
                },
            ],
            why: "testing".into(),
            where_: None,
            when_: chrono::Utc::now(),
            how: "automated".into(),
            created_at: chrono::Utc::now(),
        };
        store.write_annotation(&ann).await.unwrap();

        let loaded = store.list_annotations("m1").await.unwrap();
        assert_eq!(loaded[0].who.len(), 2);
        assert_eq!(loaded[0].who[0].canonical_id, "did:chitin:jesse-sharratt");
        assert_eq!(loaded[0].who[0].display_name, "Jesse");
        assert_eq!(loaded[0].who[1].canonical_id, "project:spectral");
    }

    #[tokio::test]
    async fn entity_ref_with_special_characters_in_canonical_id() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "test"), &[])
            .await
            .unwrap();

        let ann = MemoryAnnotation {
            id: "ann-special".into(),
            memory_id: "m1".into(),
            description: "Special chars".into(),
            who: vec![crate::EntityRef {
                canonical_id: "did:chitin:org:make-tuned-unit:agent:spectral-v2".into(),
                display_name: "Spectral v2 Agent".into(),
            }],
            why: "testing".into(),
            where_: None,
            when_: chrono::Utc::now(),
            how: "automated".into(),
            created_at: chrono::Utc::now(),
        };
        store.write_annotation(&ann).await.unwrap();

        let loaded = store.list_annotations("m1").await.unwrap();
        assert_eq!(
            loaded[0].who[0].canonical_id,
            "did:chitin:org:make-tuned-unit:agent:spectral-v2"
        );
    }

    #[tokio::test]
    async fn compaction_tier_round_trips_through_ingest_with() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Raw ambient event from activity monitor",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts {
                compaction_tier: Some(CompactionTier::Raw),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
        let mem: Memory = {
            let conn = store.conn();
            conn.query_row(&sql, [], memory_from_row).unwrap()
        };
        assert_eq!(mem.compaction_tier, Some(CompactionTier::Raw));
    }

    #[tokio::test]
    async fn ingest_opts_compaction_tier_defaults_to_none() {
        use crate::ingest::{IngestConfig, IngestOpts};

        let store = SqliteStore::open_in_memory().unwrap();
        let config = IngestConfig::default();

        crate::ingest::ingest_with(
            "m1",
            "k1",
            "Default ingest without compaction tier",
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        )
        .await
        .unwrap();

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = 'm1'");
        let mem: Memory = {
            let conn = store.conn();
            conn.query_row(&sql, [], memory_from_row).unwrap()
        };
        assert!(mem.compaction_tier.is_none());
    }

    // ── Retrieval event tests ──────────────────────────────────────

    #[tokio::test]
    async fn log_retrieval_event_inserts_row() {
        let store = SqliteStore::open_in_memory().unwrap();
        let event = RetrievalEvent {
            query_hash: "abc123".into(),
            timestamp: "2023-05-30T23:40:00Z".into(),
            memory_ids_json: "[\"m1\",\"m2\"]".into(),
            method: "cascade".into(),
            wing: Some("permagent".into()),
            question_type: Some("Counting".into()),
            session_id: None,
        };
        store.log_retrieval_event(&event).await.unwrap();

        // Verify the row exists
        let conn = store.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM retrieval_events WHERE query_hash = 'abc123'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify all fields
        let (method, wing, qtype): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT method, wing, question_type FROM retrieval_events WHERE query_hash = 'abc123'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(method, "cascade");
        assert_eq!(wing.as_deref(), Some("permagent"));
        assert_eq!(qtype.as_deref(), Some("Counting"));
    }

    #[tokio::test]
    async fn log_retrieval_event_duplicate_queries_create_separate_rows() {
        let store = SqliteStore::open_in_memory().unwrap();
        let event = RetrievalEvent {
            query_hash: "same_hash".into(),
            timestamp: "2023-05-30T10:00:00Z".into(),
            memory_ids_json: "[\"m1\"]".into(),
            method: "topk_fts".into(),
            wing: None,
            question_type: None,
            session_id: None,
        };
        store.log_retrieval_event(&event).await.unwrap();
        store.log_retrieval_event(&event).await.unwrap();

        let conn = store.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM retrieval_events WHERE query_hash = 'same_hash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 2,
            "each retrieval should create a separate event row"
        );
    }

    #[tokio::test]
    async fn log_retrieval_event_with_null_optionals() {
        let store = SqliteStore::open_in_memory().unwrap();
        let event = RetrievalEvent {
            query_hash: "hash_no_wing".into(),
            timestamp: "2023-06-01T00:00:00Z".into(),
            memory_ids_json: "[]".into(),
            method: "probe".into(),
            wing: None,
            question_type: None,
            session_id: None,
        };
        store.log_retrieval_event(&event).await.unwrap();

        let conn = store.conn();
        let (wing, qtype): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT wing, question_type FROM retrieval_events WHERE query_hash = 'hash_no_wing'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(wing.is_none());
        assert!(qtype.is_none());
    }

    // ── Description tests ───────────────────────────────────────────

    #[tokio::test]
    async fn get_memory_returns_some_for_existing() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = make_mem("m1", "k1", "test_wing");
        store.write(&mem, &[]).await.unwrap();

        let fetched = store.get_memory("m1").await.unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.id, "m1");
        assert_eq!(fetched.key, "k1");
        assert!(fetched.description.is_none());
        assert!(fetched.description_generated_at.is_none());
    }

    #[tokio::test]
    async fn get_memory_returns_none_for_missing() {
        let store = SqliteStore::open_in_memory().unwrap();
        let fetched = store.get_memory("nonexistent").await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn set_description_writes_field_and_timestamp() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = make_mem("m1", "k1", "test_wing");
        store.write(&mem, &[]).await.unwrap();

        store
            .set_description("m1", "A detailed description")
            .await
            .unwrap();

        let fetched = store.get_memory("m1").await.unwrap().unwrap();
        assert_eq!(
            fetched.description.as_deref(),
            Some("A detailed description")
        );
        let ts = fetched.description_generated_at.as_ref().unwrap();
        // Verify ISO-8601 format (ends with Z)
        assert!(ts.ends_with('Z'), "timestamp should be ISO-8601: {ts}");
        assert!(
            ts.contains('T'),
            "timestamp should contain T separator: {ts}"
        );
    }

    #[tokio::test]
    async fn set_description_returns_err_for_missing_memory() {
        let store = SqliteStore::open_in_memory().unwrap();
        let result = store.set_description("nonexistent", "desc").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_description_overwrites_existing() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = make_mem("m1", "k1", "test_wing");
        store.write(&mem, &[]).await.unwrap();

        store.set_description("m1", "first").await.unwrap();
        let first = store.get_memory("m1").await.unwrap().unwrap();
        let _ts1 = first.description_generated_at.clone().unwrap();

        store.set_description("m1", "second").await.unwrap();
        let second = store.get_memory("m1").await.unwrap().unwrap();
        assert_eq!(second.description.as_deref(), Some("second"));
        // Timestamp should be updated (or at least present)
        assert!(second.description_generated_at.is_some());
    }

    #[tokio::test]
    async fn list_undescribed_returns_only_null_descriptions() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.write(&make_mem("m1", "k1", "w"), &[]).await.unwrap();
        store.write(&make_mem("m2", "k2", "w"), &[]).await.unwrap();
        store.write(&make_mem("m3", "k3", "w"), &[]).await.unwrap();

        // Describe one
        store.set_description("m1", "described").await.unwrap();

        let undescribed = store.list_undescribed(100).await.unwrap();
        assert_eq!(undescribed.len(), 2);
        let ids: Vec<&str> = undescribed.iter().map(|m| m.id.as_str()).collect();
        assert!(!ids.contains(&"m1"));
        assert!(ids.contains(&"m2"));
        assert!(ids.contains(&"m3"));
    }

    #[tokio::test]
    async fn list_undescribed_respects_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..5 {
            store
                .write(&make_mem(&format!("m{i}"), &format!("k{i}"), "w"), &[])
                .await
                .unwrap();
        }

        let undescribed = store.list_undescribed(2).await.unwrap();
        assert_eq!(undescribed.len(), 2);
    }

    // ── Co-retrieval index tests ────────────────────────────────────

    async fn insert_retrieval_event(store: &SqliteStore, memory_ids: &[&str]) {
        let ids_json = serde_json::to_string(&memory_ids).unwrap();
        let event = RetrievalEvent {
            query_hash: "test_hash".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
            memory_ids_json: ids_json,
            method: "cascade".into(),
            wing: None,
            question_type: None,
            session_id: None,
        };
        store.log_retrieval_event(&event).await.unwrap();
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_from_empty() {
        let store = SqliteStore::open_in_memory().unwrap();
        let count = store.rebuild_co_retrieval_index().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_single_event_no_pairs() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1"]).await;
        let count = store.rebuild_co_retrieval_index().await.unwrap();
        assert_eq!(count, 0, "single memory in event produces no pairs");
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_pairs_from_one_event() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1", "m2", "m3"]).await;
        let count = store.rebuild_co_retrieval_index().await.unwrap();
        // 3 memories → C(3,2) = 3 pairs
        assert_eq!(count, 3);

        let related = store.related_memories("m1", 10).await.unwrap();
        assert_eq!(related.len(), 2);
        assert!(related.iter().all(|r| r.co_count == 1));
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_aggregates_across_events() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1", "m2"]).await;
        insert_retrieval_event(&store, &["m1", "m2", "m3"]).await;
        let count = store.rebuild_co_retrieval_index().await.unwrap();
        // Event 1: (m1,m2). Event 2: (m1,m2),(m1,m3),(m2,m3). Unique pairs: 3.
        assert_eq!(count, 3);

        let related = store.related_memories("m1", 10).await.unwrap();
        // m2 co-occurred with m1 in both events → co_count=2
        let m2_entry = related.iter().find(|r| r.memory_id == "m2").unwrap();
        assert_eq!(m2_entry.co_count, 2);
        // m3 co-occurred with m1 in one event → co_count=1
        let m3_entry = related.iter().find(|r| r.memory_id == "m3").unwrap();
        assert_eq!(m3_entry.co_count, 1);
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_normalizes_pair_order() {
        let store = SqliteStore::open_in_memory().unwrap();
        // Event has m2 before m1 — should still normalize to (m1, m2)
        insert_retrieval_event(&store, &["m2", "m1"]).await;
        let count = store.rebuild_co_retrieval_index().await.unwrap();
        assert_eq!(count, 1);

        // Query from either side should work
        let from_m1 = store.related_memories("m1", 10).await.unwrap();
        assert_eq!(from_m1.len(), 1);
        assert_eq!(from_m1[0].memory_id, "m2");

        let from_m2 = store.related_memories("m2", 10).await.unwrap();
        assert_eq!(from_m2.len(), 1);
        assert_eq!(from_m2[0].memory_id, "m1");
    }

    #[tokio::test]
    async fn rebuild_co_retrieval_index_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1", "m2"]).await;

        let count1 = store.rebuild_co_retrieval_index().await.unwrap();
        let count2 = store.rebuild_co_retrieval_index().await.unwrap();
        assert_eq!(count1, count2, "rebuild should be idempotent");

        let related = store.related_memories("m1", 10).await.unwrap();
        assert_eq!(related[0].co_count, 1, "counts should not accumulate");
    }

    #[tokio::test]
    async fn related_memories_orders_by_co_count_desc() {
        let store = SqliteStore::open_in_memory().unwrap();
        // m1+m2 co-occur 3 times, m1+m3 once
        for _ in 0..3 {
            insert_retrieval_event(&store, &["m1", "m2"]).await;
        }
        insert_retrieval_event(&store, &["m1", "m3"]).await;
        store.rebuild_co_retrieval_index().await.unwrap();

        let related = store.related_memories("m1", 10).await.unwrap();
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].memory_id, "m2");
        assert_eq!(related[0].co_count, 3);
        assert_eq!(related[1].memory_id, "m3");
        assert_eq!(related[1].co_count, 1);
    }

    #[tokio::test]
    async fn related_memories_respects_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1", "m2", "m3", "m4", "m5", "m6"]).await;
        store.rebuild_co_retrieval_index().await.unwrap();

        let related = store.related_memories("m1", 2).await.unwrap();
        assert_eq!(related.len(), 2);
    }

    #[tokio::test]
    async fn related_memories_returns_empty_for_unknown_id() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_retrieval_event(&store, &["m1", "m2"]).await;
        store.rebuild_co_retrieval_index().await.unwrap();

        let related = store.related_memories("unknown", 10).await.unwrap();
        assert!(related.is_empty());
    }

    // ── Session retrieval tests ─────────────────────────────────────

    fn make_session_event(session: Option<&str>, ts: &str, ids: &[&str]) -> RetrievalEvent {
        RetrievalEvent {
            query_hash: "h".into(),
            timestamp: ts.into(),
            memory_ids_json: serde_json::to_string(&ids).unwrap(),
            method: "cascade".into(),
            wing: None,
            question_type: None,
            session_id: session.map(|s| s.into()),
        }
    }

    #[tokio::test]
    async fn log_event_with_session_id() {
        let store = SqliteStore::open_in_memory().unwrap();
        let event = make_session_event(Some("sess-1"), "2024-01-01T00:00:00Z", &["m1"]);
        store.log_retrieval_event(&event).await.unwrap();

        let conn = store.conn();
        let sid: Option<String> = conn
            .query_row(
                "SELECT session_id FROM retrieval_events LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sid.as_deref(), Some("sess-1"));
    }

    #[tokio::test]
    async fn log_event_without_session_id_stores_null() {
        let store = SqliteStore::open_in_memory().unwrap();
        let event = make_session_event(None, "2024-01-01T00:00:00Z", &["m1"]);
        store.log_retrieval_event(&event).await.unwrap();

        let conn = store.conn();
        let sid: Option<String> = conn
            .query_row(
                "SELECT session_id FROM retrieval_events LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sid.is_none());
    }

    #[tokio::test]
    async fn events_for_session_returns_only_matching() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..3 {
            let e = make_session_event(Some("A"), &format!("2024-01-0{i}T00:00:00Z"), &["m1"]);
            store.log_retrieval_event(&e).await.unwrap();
        }
        for i in 0..2 {
            let e = make_session_event(Some("B"), &format!("2024-02-0{i}T00:00:00Z"), &["m2"]);
            store.log_retrieval_event(&e).await.unwrap();
        }

        let events = store.events_for_session("A", 100).await.unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| e.session_id.as_deref() == Some("A")));
    }

    #[tokio::test]
    async fn events_for_session_orders_by_timestamp_asc() {
        let store = SqliteStore::open_in_memory().unwrap();
        // Insert out of order
        store
            .log_retrieval_event(&make_session_event(
                Some("S"),
                "2024-01-03T00:00:00Z",
                &["m1"],
            ))
            .await
            .unwrap();
        store
            .log_retrieval_event(&make_session_event(
                Some("S"),
                "2024-01-01T00:00:00Z",
                &["m2"],
            ))
            .await
            .unwrap();
        store
            .log_retrieval_event(&make_session_event(
                Some("S"),
                "2024-01-02T00:00:00Z",
                &["m3"],
            ))
            .await
            .unwrap();

        let events = store.events_for_session("S", 100).await.unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[0].timestamp < events[1].timestamp);
        assert!(events[1].timestamp < events[2].timestamp);
    }

    #[tokio::test]
    async fn events_for_session_respects_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..5 {
            let e = make_session_event(Some("S"), &format!("2024-01-0{i}T00:00:00Z"), &["m1"]);
            store.log_retrieval_event(&e).await.unwrap();
        }

        let events = store.events_for_session("S", 2).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn events_for_session_returns_empty_for_unknown() {
        let store = SqliteStore::open_in_memory().unwrap();
        let e = make_session_event(Some("X"), "2024-01-01T00:00:00Z", &["m1"]);
        store.log_retrieval_event(&e).await.unwrap();

        let events = store.events_for_session("unknown", 100).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn memories_for_session_dedupes_across_events() {
        let store = SqliteStore::open_in_memory().unwrap();
        let e1 = make_session_event(Some("S"), "2024-01-01T00:00:00Z", &["m1", "m2"]);
        let e2 = make_session_event(Some("S"), "2024-01-02T00:00:00Z", &["m2", "m3"]);
        store.log_retrieval_event(&e1).await.unwrap();
        store.log_retrieval_event(&e2).await.unwrap();

        let mems = store.memories_for_session("S").await.unwrap();
        assert_eq!(mems.len(), 3, "m2 should appear only once");
        // m1, m2 from first event; m3 from second (m2 deduped)
        assert_eq!(mems, vec!["m1", "m2", "m3"]);
    }

    #[tokio::test]
    async fn memories_for_session_orders_by_first_seen() {
        let store = SqliteStore::open_in_memory().unwrap();
        let e1 = make_session_event(Some("S"), "2024-01-01T00:00:00Z", &["m1"]);
        let e2 = make_session_event(Some("S"), "2024-01-02T00:00:00Z", &["m2"]);
        store.log_retrieval_event(&e1).await.unwrap();
        store.log_retrieval_event(&e2).await.unwrap();

        let mems = store.memories_for_session("S").await.unwrap();
        assert_eq!(mems, vec!["m1", "m2"], "m1 should come before m2");
    }

    // ── WriteOutcome / content-hash dedup tests ─────────────────────

    #[tokio::test]
    async fn write_inserts_new_memory() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "hello world".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome = store.write(&mem, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::Inserted);

        // Row exists with correct content_hash
        let conn = store.conn();
        let hash: String = conn
            .query_row(
                "SELECT content_hash FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let expected = blake3::hash(b"hello world").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn write_noop_on_identical_content_preserves_signal_score() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "stable content".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        // Reinforce to bump signal_score to 0.8
        store.reinforce_memory("k1", 0.2).await.unwrap();
        let after_reinforce: f64 = {
            let conn = store.conn();
            conn.query_row(
                "SELECT signal_score FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!((after_reinforce - 0.8).abs() < 0.01);

        // Re-write identical content (stale signal_score 0.6 in struct)
        let outcome = store.write(&mem, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::NoOp);

        // signal_score should still be 0.8
        let final_score: f64 = {
            let conn = store.conn();
            conn.query_row(
                "SELECT signal_score FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(
            (final_score - 0.8).abs() < 0.01,
            "signal_score should be preserved at 0.8, got {final_score}"
        );
    }

    #[tokio::test]
    async fn write_noop_does_not_touch_updated_at() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "stable".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        let original_updated_at: Option<String> = {
            let conn = store.conn();
            conn.query_row(
                "SELECT updated_at FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };

        // Sleep 1 second then re-write identical content
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let outcome = store.write(&mem, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::NoOp);

        let after_updated_at: Option<String> = {
            let conn = store.conn();
            conn.query_row(
                "SELECT updated_at FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(original_updated_at, after_updated_at);
    }

    #[tokio::test]
    async fn write_content_change_updates_in_place() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem1 = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "v1".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem1, &[]).await.unwrap();

        let original_updated_at: Option<String> = {
            let conn = store.conn();
            conn.query_row(
                "SELECT updated_at FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let mem2 = Memory {
            id: "m2".into(),
            key: "k1".into(),
            content: "v2".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.9,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome = store.write(&mem2, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::ContentUpdated);

        let conn = store.conn();
        let (content, hash, updated_at): (String, String, Option<String>) = conn
            .query_row(
                "SELECT content, content_hash, updated_at FROM memories WHERE key = 'k1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(content, "v2");
        let expected_hash = blake3::hash(b"v2").to_hex().to_string();
        assert_eq!(hash, expected_hash);
        assert_ne!(updated_at, original_updated_at, "updated_at should advance");
    }

    #[tokio::test]
    async fn write_content_change_preserves_signal_score_and_other_fields() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "original".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.5,
            visibility: "private".into(),
            source: Some("test_source".into()),
            device_id: None,
            confidence: 0.9,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        store.write(&mem, &[]).await.unwrap();

        // Reinforce to bump signal_score
        store.reinforce_memory("k1", 0.3).await.unwrap();

        // Write with new content but different signal_score/wing/hall in struct
        let mem2 = Memory {
            id: "m2".into(),
            key: "k1".into(),
            content: "updated content".into(),
            wing: Some("different_wing".into()),
            hall: Some("different_hall".into()),
            signal_score: 0.1, // caller passes stale/wrong score
            visibility: "public".into(),
            source: Some("new_source".into()),
            device_id: None,
            confidence: 0.5,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome = store.write(&mem2, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::ContentUpdated);

        let conn = store.conn();
        let (content, signal_score, wing, hall, visibility, source): (
            String,
            f64,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT content, signal_score, wing, hall, visibility, source FROM memories WHERE key = 'k1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        assert_eq!(content, "updated content");
        // signal_score preserved: 0.5 + 0.3 = 0.8
        assert!(
            (signal_score - 0.8).abs() < 0.01,
            "signal_score should be preserved at 0.8, got {signal_score}"
        );
        // All other fields should be preserved from original insert
        assert_eq!(wing.as_deref(), Some("w"));
        assert_eq!(hall.as_deref(), Some("fact"));
        assert_eq!(visibility, "private");
        assert_eq!(source.as_deref(), Some("test_source"));
    }

    #[tokio::test]
    async fn write_null_hash_row_routes_correctly_noop() {
        let store = SqliteStore::open_in_memory().unwrap();
        // Insert a row with NULL content_hash (simulating pre-backfill state)
        {
            let conn = store.conn();
            conn.execute(
                "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
                 VALUES ('m1', 'k1', 'legacy content', 'w', 'fact', 0.7, 'private')",
                [],
            )
            .unwrap();
        }

        // Write identical content
        let mem = Memory {
            id: "m_new".into(),
            key: "k1".into(),
            content: "legacy content".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.5,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome = store.write(&mem, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::NoOp);

        // content_hash should now be populated (backfilled)
        let hash: Option<String> = {
            let conn = store.conn();
            conn.query_row(
                "SELECT content_hash FROM memories WHERE key = 'k1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(hash.is_some(), "content_hash should be backfilled on NoOp");
        let expected = blake3::hash(b"legacy content").to_hex().to_string();
        assert_eq!(hash.unwrap(), expected);
    }

    #[tokio::test]
    async fn write_null_hash_row_with_different_content_updates() {
        let store = SqliteStore::open_in_memory().unwrap();
        // Insert a row with NULL content_hash
        {
            let conn = store.conn();
            conn.execute(
                "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
                 VALUES ('m1', 'k1', 'old content', 'w', 'fact', 0.7, 'private')",
                [],
            )
            .unwrap();
        }

        // Write different content
        let mem = Memory {
            id: "m_new".into(),
            key: "k1".into(),
            content: "new content".into(),
            wing: Some("w".into()),
            hall: Some("fact".into()),
            signal_score: 0.5,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
        };
        let outcome = store.write(&mem, &[]).await.unwrap();
        assert_eq!(outcome, WriteOutcome::ContentUpdated);

        let conn = store.conn();
        let (content, hash): (String, String) = conn
            .query_row(
                "SELECT content, content_hash FROM memories WHERE key = 'k1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(content, "new content");
        let expected = blake3::hash(b"new content").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn backfill_content_hashes_populates_null_rows() {
        let store = SqliteStore::open_in_memory().unwrap();
        // Insert rows with NULL hashes via direct SQL
        {
            let conn = store.conn();
            for i in 1..=3 {
                conn.execute(
                    "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
                     VALUES (?1, ?2, ?3, 'w', 'fact', 0.5, 'private')",
                    params![format!("m{i}"), format!("k{i}"), format!("content {i}")],
                )
                .unwrap();
            }
        }

        let count = store.backfill_content_hashes().await.unwrap();
        assert_eq!(count, 3);

        // Verify all rows now have hashes
        let conn = store.conn();
        for i in 1..=3 {
            let hash: Option<String> = conn
                .query_row(
                    "SELECT content_hash FROM memories WHERE key = ?1",
                    params![format!("k{i}")],
                    |row| row.get(0),
                )
                .unwrap();
            let expected = blake3::hash(format!("content {i}").as_bytes())
                .to_hex()
                .to_string();
            assert_eq!(hash, Some(expected));
        }
    }

    #[tokio::test]
    async fn backfill_is_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        {
            let conn = store.conn();
            conn.execute(
                "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
                 VALUES ('m1', 'k1', 'hello', 'w', 'fact', 0.5, 'private')",
                [],
            )
            .unwrap();
        }

        let count1 = store.backfill_content_hashes().await.unwrap();
        assert_eq!(count1, 1);

        let count2 = store.backfill_content_hashes().await.unwrap();
        assert_eq!(count2, 0, "second backfill should find no NULL rows");
    }
}
