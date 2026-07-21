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
    CompactionTier, ConsolidateOpts, ConsolidationEdge, ConsolidationResult, EntityField, Episode,
    FieldSource, Fingerprint, ForgetReceipt, InvalidSourcePolicy, Memory, MemoryAnnotation,
    MemoryHit, MemoryStore, RelatedMemory, RetrievalEvent, SkipReason, SpectrogramRow,
    WriteOutcome,
};
use lru::LruCache;
use rusqlite::{params, Connection, OptionalExtension};
use std::future::Future;
use std::num::NonZeroUsize;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::TimeBucket;

const WING_CACHE_CAPACITY: usize = 32;

/// Default FTS5 tokenizer for the memories index. Porter stemming bridges
/// plural/inflected queries to singular content deterministically and at
/// zero runtime cost — the recall-path complement to Spectral's no-LLM,
/// no-embedding retrieval commitment. Override via
/// `SqliteStoreConfig::fts_tokenizer` or the `SPECTRAL_FTS_TOKENIZER` env
/// var (set to `"unicode61"` or an empty string to disable stemming).
pub const DEFAULT_FTS_TOKENIZER: &str = "porter unicode61";

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
    /// FTS5 tokenizer spec for the memories index (e.g. `"porter unicode61"`).
    ///
    /// - `None` (default) — fall back to the `SPECTRAL_FTS_TOKENIZER` env var,
    ///   then to [`DEFAULT_FTS_TOKENIZER`] (`"porter unicode61"`). Porter
    ///   stemming bridges plural/inflected queries to singular content
    ///   ("doctors" → "doctor") at zero runtime cost; validated Tier-0/Tier-1
    ///   (see docs/internal/ORACLE_TIER0.md, docs/internal/TIER1_RESULTS.md).
    /// - `Some("unicode61")` — explicit no-stemming tokenizer (SQLite default
    ///   behavior). An empty string also disables the tokenize clause.
    /// - An existing database built with a different tokenizer is rebuilt
    ///   once on open (drop + recreate + repopulate of the FTS index; the
    ///   memories table itself is untouched). Not applied in read-only mode.
    pub fts_tokenizer: Option<String>,
    /// Open the database read-only. Default false.
    ///
    /// In read-only mode the connection is opened with
    /// `SQLITE_OPEN_READ_ONLY`, and **no** schema creation, migration, FTS
    /// rebuild, or backfill runs — opening never mutates the database. Any
    /// write attempted through the store fails at the driver level. This is
    /// the mode for federated read-time fan-out over a brain you don't own.
    /// Fails if the database file does not exist.
    pub read_only: bool,
    /// Enable stemmed + unstemmed **RRF fusion** for FTS recall. Default false.
    ///
    /// Porter stemming (the default tokenizer) is a recall device that trades
    /// away precision: it bridges `doctors`→`doctor` but also conflates
    /// `university`→`univers`←`universe`, so a short colliding distractor can
    /// outrank the answer at a tight `k`. With fusion on, the store maintains a
    /// second, content-only, **unstemmed** (`unicode61`) FTS index and, at
    /// query time, fuses the porter-ranked and unstemmed-ranked lists by
    /// Reciprocal Rank Fusion (Cormack 2009, k=60). Different *representations*
    /// (the Beitzel precondition) — the fused list recovers both porter's
    /// over-stemming precision losses and the unstemmed channel's inflection
    /// misses; measured recall@1 0.57→1.00 on the fusion micro-bench. Rank-based,
    /// so no score normalization across the two BM25 scales.
    ///
    /// Costs a second FTS index (content only, kept in sync by triggers, purged
    /// on delete like the primary), so it is opt-in per the measure-before-
    /// defaulting discipline. Falls back to `SPECTRAL_FTS_FUSION`. On a
    /// read-only open the fused path is used only if the raw index already
    /// exists in the file (it is never created read-only).
    pub fts_fusion: bool,
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
    /// Whether `fts_search` should RRF-fuse the porter index with the unstemmed
    /// `memories_fts_raw` index. True only when fusion was requested AND the raw
    /// index is present in the file.
    fusion: bool,
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
        if config.read_only {
            return Self::open_read_only(path, config);
        }
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
        let fts_tokenizer = Self::resolve_fts_tokenizer(config);
        Self::init_schema(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_provenance_columns(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_fts_tokenizer(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_fk_cascade(&conn)?;
        // Enable FK enforcement AFTER all migrations complete (migrate_fk_cascade
        // turns FK OFF for table rebuilds). This is per-connection in SQLite and
        // must be set on every connection before any DML runs.
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        // Stemmed + unstemmed RRF fusion: build/refresh the content-only
        // unstemmed index when requested (idempotent). Write-mode only.
        let fusion = if Self::fusion_enabled(config) {
            Self::ensure_fusion_index(&conn)?;
            true
        } else {
            false
        };
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wing_cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(WING_CACHE_CAPACITY).unwrap(),
            ))),
            fusion,
        })
    }

    /// Open an existing memory database strictly read-only: the connection
    /// carries `SQLITE_OPEN_READ_ONLY`, and no schema creation, migration,
    /// FTS rebuild, or backfill runs. Opening never mutates the file; any
    /// write attempted later fails at the driver level. Fails if the file
    /// does not exist.
    fn open_read_only(path: &Path, config: &SqliteStoreConfig) -> anyhow::Result<Self> {
        use rusqlite::OpenFlags;
        if !path.exists() {
            anyhow::bail!(
                "read-only open requires an existing database: {} not found",
                path.display()
            );
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mmap_size = match config.mmap_size {
            Some(explicit) => explicit,
            None => Self::compute_mmap_size(path),
        };
        // Per-connection, non-persistent pragmas only.
        conn.execute_batch(&format!(
            "PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size  = {mmap_size};
             PRAGMA query_only = ON;"
        ))?;
        // Read-only: never create the raw index; fuse only if it already exists
        // in the file (a replica synced from a fusion-enabled brain).
        let fusion = Self::fusion_enabled(config) && Self::fusion_index_present(&conn);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wing_cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(WING_CACHE_CAPACITY).unwrap(),
            ))),
            fusion,
        })
    }

    /// Create an in-memory database (useful for tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let fts_tokenizer = Self::resolve_fts_tokenizer(&SqliteStoreConfig::default());
        Self::init_schema(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_provenance_columns(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_fts_tokenizer(&conn, fts_tokenizer.as_deref())?;
        Self::migrate_fk_cascade(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wing_cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(WING_CACHE_CAPACITY).unwrap(),
            ))),
            fusion: false,
        })
    }

    /// Whether stemmed+unstemmed fusion is requested: explicit config flag, or
    /// the `SPECTRAL_FTS_FUSION` env var (`1`/`true`). Default false.
    fn fusion_enabled(config: &SqliteStoreConfig) -> bool {
        config.fts_fusion
            || std::env::var("SPECTRAL_FTS_FUSION")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
    }

    /// Whether the unstemmed fusion index exists in the database file.
    fn fusion_index_present(conn: &Connection) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='memories_fts_raw'",
            [],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// Idempotently create the content-only, **unstemmed** (`unicode61`) FTS
    /// index used as the second fusion channel, its sync triggers, and populate
    /// it from `memories`. The primary porter index is untouched. Cheap to
    /// re-run: returns early once the table exists (triggers keep it in sync,
    /// and the AFTER DELETE trigger purges it on `forget`, so no changes are
    /// needed in the delete path).
    fn ensure_fusion_index(conn: &Connection) -> anyhow::Result<()> {
        if Self::fusion_index_present(conn) {
            return Ok(());
        }
        // One transaction: the CREATE and its repopulating INSERT commit together,
        // so a crash between them can't leave `memories_fts_raw` present-but-empty
        // (which `fusion_index_present` would then treat as built, skipping the
        // repopulate forever).
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE VIRTUAL TABLE memories_fts_raw USING fts5(
                 content,
                 content=memories, content_rowid=rowid, tokenize = 'unicode61'
             );
             CREATE TRIGGER IF NOT EXISTS memories_ai_raw AFTER INSERT ON memories BEGIN
                 INSERT INTO memories_fts_raw(rowid, content) VALUES (new.rowid, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS memories_ad_raw AFTER DELETE ON memories BEGIN
                 INSERT INTO memories_fts_raw(memories_fts_raw, rowid, content)
                 VALUES ('delete', old.rowid, old.content);
             END;
             CREATE TRIGGER IF NOT EXISTS memories_au_raw AFTER UPDATE ON memories BEGIN
                 INSERT INTO memories_fts_raw(memories_fts_raw, rowid, content)
                 VALUES ('delete', old.rowid, old.content);
                 INSERT INTO memories_fts_raw(rowid, content) VALUES (new.rowid, new.content);
             END;
             INSERT INTO memories_fts_raw(rowid, content) SELECT rowid, content FROM memories;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Resolve the FTS tokenizer: explicit config, then env var, then
    /// [`DEFAULT_FTS_TOKENIZER`] (porter stemming). The spec is sanitized to
    /// bare words so it can be embedded in a `tokenize = '…'` clause. An
    /// explicit empty value (config or env) resolves to `None` — no tokenize
    /// clause, i.e. SQLite's unstemmed unicode61 default.
    fn resolve_fts_tokenizer(config: &SqliteStoreConfig) -> Option<String> {
        let raw = config
            .fts_tokenizer
            .clone()
            .or_else(|| std::env::var("SPECTRAL_FTS_TOKENIZER").ok())
            .unwrap_or_else(|| DEFAULT_FTS_TOKENIZER.to_string());
        let safe: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_')
            .collect();
        let safe = safe.trim().to_string();
        if safe.is_empty() {
            None
        } else {
            Some(safe)
        }
    }

    /// Render the optional `tokenize` argument for the FTS5 CREATE statement.
    fn fts_tokenize_clause(fts_tokenizer: Option<&str>) -> String {
        match fts_tokenizer {
            Some(t) => format!(", tokenize = '{t}'"),
            None => String::new(),
        }
    }

    /// SQL batch that drops and recreates the memories FTS index (table,
    /// triggers) with the given tokenizer, then repopulates it from the
    /// memories table. Used by both the description-column migration and the
    /// tokenizer-change migration. The memories table itself is untouched.
    fn fts_rebuild_batch(fts_tokenizer: Option<&str>) -> String {
        let fts_tok = Self::fts_tokenize_clause(fts_tokenizer);
        // Wrapped in one transaction so the DROP and the repopulating INSERT commit
        // together. `execute_batch` otherwise autocommits each statement, leaving a
        // crash-between-them window where `memories_fts` exists but is empty — and a
        // later matching-tokenizer open skips the rebuild, so FTS recall silently
        // returns nothing for the whole corpus. SQLite DDL is transactional, so a
        // crash mid-rebuild rolls back to the intact prior index. (BEGIN here is
        // safe: every caller invokes this on a fresh connection in autocommit mode.)
        format!(
            "BEGIN IMMEDIATE;
             DROP TRIGGER IF EXISTS memories_ai;
             DROP TRIGGER IF EXISTS memories_ad;
             DROP TRIGGER IF EXISTS memories_au;
             DROP TABLE IF EXISTS memories_fts;

             CREATE VIRTUAL TABLE memories_fts USING fts5(
                 key, content, description,
                 content=memories, content_rowid=rowid{fts_tok}
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
             SELECT rowid, key, content, COALESCE(description, '') FROM memories;
             COMMIT;"
        )
    }

    /// Extract the tokenizer spec from an FTS5 CREATE statement as recorded
    /// in `sqlite_master`, e.g. `tokenize = 'porter unicode61'` →
    /// `Some("porter unicode61")`. Returns `None` when no tokenize clause is
    /// present (SQLite default tokenizer).
    fn parse_fts_tokenize_spec(sql: &str) -> Option<String> {
        let idx = sql.find("tokenize")?;
        let rest = &sql[idx..];
        let open = rest.find('\'')?;
        let quoted = &rest[open + 1..];
        let close = quoted.find('\'')?;
        Some(quoted[..close].trim().to_string())
    }

    /// Idempotent migration: rebuild the FTS index when the tokenizer the
    /// database was built with differs from the resolved tokenizer. FTS5 bakes
    /// the tokenizer into the index at creation time, so a tokenizer change
    /// (e.g. the porter-stemming default introduced after older databases were
    /// created) requires a one-time drop + recreate + repopulate.
    fn migrate_fts_tokenizer(conn: &Connection, fts_tokenizer: Option<&str>) -> anyhow::Result<()> {
        let fts_sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .ok();
        let Some(sql) = fts_sql else {
            return Ok(()); // no FTS table yet; init_schema creates it correctly
        };
        let current = Self::parse_fts_tokenize_spec(&sql);
        let desired = fts_tokenizer.map(|t| t.trim().to_string());
        if current == desired {
            return Ok(());
        }
        tracing::info!(
            from = current.as_deref().unwrap_or("<default>"),
            to = desired.as_deref().unwrap_or("<default>"),
            "rebuilding memories_fts for tokenizer change"
        );
        conn.execute_batch(&Self::fts_rebuild_batch(fts_tokenizer))?;
        Ok(())
    }

    fn init_schema(conn: &Connection, fts_tokenizer: Option<&str>) -> anyhow::Result<()> {
        let fts_tok = Self::fts_tokenize_clause(fts_tokenizer);
        conn.execute_batch(&format!(
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
                content=memories, content_rowid=rowid{fts_tok}
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
                FOREIGN KEY (anchor_memory_id) REFERENCES memories(id) ON DELETE CASCADE,
                FOREIGN KEY (target_memory_id) REFERENCES memories(id) ON DELETE CASCADE
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
                FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
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
                ON memory_annotations(memory_id);

            CREATE TABLE IF NOT EXISTS consolidation_edges (
                source_key      TEXT NOT NULL,
                target_key      TEXT NOT NULL,
                consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (source_key, target_key),
                FOREIGN KEY (source_key) REFERENCES memories(key) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_consolidation_target
                ON consolidation_edges(target_key);

            CREATE TABLE IF NOT EXISTS entity_fields (
                entity_id   TEXT NOT NULL,
                field_name  TEXT NOT NULL,
                value       TEXT NOT NULL,
                source      TEXT NOT NULL,
                source_url  TEXT,
                updated_at  TEXT NOT NULL,
                PRIMARY KEY (entity_id, field_name)
            );
            -- The composite PK already indexes the entity_id prefix (fast
            -- per-entity load). This secondary index supports cross-entity
            -- queries by field_name.
            CREATE INDEX IF NOT EXISTS idx_entity_fields_name
                ON entity_fields(field_name);"
        ))?;
        Ok(())
    }

    /// Idempotent migration: adds source/device_id/confidence columns to
    /// existing databases that lack them.
    fn migrate_provenance_columns(
        conn: &Connection,
        fts_tokenizer: Option<&str>,
    ) -> anyhow::Result<()> {
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

        // Signed-provenance columns (PR #1): authenticated authoring brain and
        // Ed25519 signature over the memory's signed payload. NULL on legacy /
        // unsigned rows.
        let (mut has_source_brain_id, mut has_signature) = (false, false);
        let mut stmt_sig = conn.prepare("PRAGMA table_info(memories)")?;
        let rows_sig = stmt_sig.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows_sig {
            match name?.as_str() {
                "source_brain_id" => has_source_brain_id = true,
                "signature" => has_signature = true,
                _ => {}
            }
        }
        if !has_source_brain_id {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN source_brain_id BLOB DEFAULT NULL",
            )?;
        }
        if !has_signature {
            conn.execute_batch("ALTER TABLE memories ADD COLUMN signature BLOB DEFAULT NULL")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memories_source_brain_id ON memories(source_brain_id)",
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
                .is_some_and(|sql| sql.contains("description"))
        };
        if !fts_has_description {
            conn.execute_batch(&Self::fts_rebuild_batch(fts_tokenizer))?;
        }

        // consolidation_edges table (for existing databases that don't have it)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS consolidation_edges (
                source_key      TEXT NOT NULL,
                target_key      TEXT NOT NULL,
                consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (source_key, target_key),
                FOREIGN KEY (source_key) REFERENCES memories(key) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_consolidation_target
                ON consolidation_edges(target_key);",
        )?;

        Ok(())
    }

    /// Migrate constellation_fingerprints and memory_spectrogram FK definitions
    /// from NO ACTION to ON DELETE CASCADE, matching the memory_annotations convention.
    ///
    /// SQLite cannot ALTER a FK in place, so this uses the 12-step table-rebuild:
    /// 1. Detect whether migration is needed (check FK SQL for CASCADE)
    /// 2. Clean orphaned child rows (idempotent — safe to run on already-cleaned DBs)
    /// 3. Rebuild each table with CASCADE FK definitions
    /// 4. Run PRAGMA foreign_key_check to verify integrity
    fn migrate_fk_cascade(conn: &Connection) -> anyhow::Result<()> {
        // Check if constellation_fingerprints already has CASCADE.
        // On a fresh DB created with init_schema (which now includes CASCADE),
        // the DDL will already contain "ON DELETE CASCADE" and we skip.
        let needs_fp_migration = {
            let sql: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='constellation_fingerprints'",
                    [],
                    |row| row.get(0),
                )
                .ok();
            match sql {
                Some(ddl) => !ddl.to_lowercase().contains("on delete cascade"),
                None => false, // table doesn't exist yet — init_schema will create it with CASCADE
            }
        };

        let needs_spec_migration = {
            let sql: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='memory_spectrogram'",
                    [],
                    |row| row.get(0),
                )
                .ok();
            match sql {
                Some(ddl) => !ddl.to_lowercase().contains("on delete cascade"),
                None => false,
            }
        };

        if !needs_fp_migration && !needs_spec_migration {
            return Ok(());
        }

        // Disable FK enforcement during the rebuild (required for table-rename).
        // This is safe: we clean orphans first, then rebuild, then verify.
        conn.execute_batch("PRAGMA foreign_keys = OFF")?;

        conn.execute_batch("BEGIN")?;

        // Run the table-rebuild inside a closure so any error triggers ROLLBACK.
        let rebuild_result: anyhow::Result<()> = (|| {
            if needs_fp_migration {
                // Step 1: Clean orphans (idempotent — no-op on already-clean DB)
                conn.execute_batch(
                    "DELETE FROM constellation_fingerprints
                     WHERE anchor_memory_id NOT IN (SELECT id FROM memories);
                     DELETE FROM constellation_fingerprints
                     WHERE target_memory_id NOT IN (SELECT id FROM memories);",
                )?;

                // Step 2: Create new table with CASCADE FKs
                conn.execute_batch(
                    "CREATE TABLE constellation_fingerprints_new (
                        id                TEXT PRIMARY KEY,
                        fingerprint_hash  TEXT NOT NULL,
                        anchor_memory_id  TEXT NOT NULL,
                        target_memory_id  TEXT NOT NULL,
                        wing              TEXT,
                        anchor_hall       TEXT,
                        target_hall       TEXT,
                        time_delta_bucket TEXT,
                        created_at        TEXT,
                        FOREIGN KEY (anchor_memory_id) REFERENCES memories(id) ON DELETE CASCADE,
                        FOREIGN KEY (target_memory_id) REFERENCES memories(id) ON DELETE CASCADE
                    )",
                )?;

                // Step 3: Copy surviving rows
                conn.execute_batch(
                    "INSERT INTO constellation_fingerprints_new
                     SELECT * FROM constellation_fingerprints",
                )?;

                // Step 4: Drop old, rename new
                conn.execute_batch(
                    "DROP TABLE constellation_fingerprints;
                     ALTER TABLE constellation_fingerprints_new RENAME TO constellation_fingerprints",
                )?;

                // Step 5: Recreate indexes
                conn.execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_fp_hash ON constellation_fingerprints(fingerprint_hash);
                     CREATE INDEX IF NOT EXISTS idx_fp_wing_hash
                         ON constellation_fingerprints(wing, fingerprint_hash);
                     CREATE INDEX IF NOT EXISTS idx_fp_wing_anchor_hall
                         ON constellation_fingerprints(wing, anchor_hall);
                     CREATE INDEX IF NOT EXISTS idx_fp_wing_target_hall
                         ON constellation_fingerprints(wing, target_hall)",
                )?;
            }

            if needs_spec_migration {
                // Step 1: Clean orphans
                conn.execute_batch(
                    "DELETE FROM memory_spectrogram
                     WHERE memory_id NOT IN (SELECT id FROM memories)",
                )?;

                // Step 2: Create new table with CASCADE FK
                conn.execute_batch(
                    "CREATE TABLE memory_spectrogram_new (
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
                        FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
                    )",
                )?;

                // Step 3: Copy surviving rows
                conn.execute_batch(
                    "INSERT INTO memory_spectrogram_new
                     SELECT * FROM memory_spectrogram",
                )?;

                // Step 4: Drop old, rename new
                conn.execute_batch(
                    "DROP TABLE memory_spectrogram;
                     ALTER TABLE memory_spectrogram_new RENAME TO memory_spectrogram",
                )?;

                // Step 5: Recreate indexes
                conn.execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_spectrogram_action ON memory_spectrogram(action_type)",
                )?;
            }

            // Step 6: Verify integrity — foreign_key_check should return no rows
            let fk_violations: i64 =
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })?;
            if fk_violations > 0 {
                return Err(anyhow::anyhow!(
                    "FK cascade migration: {fk_violations} FK violations remain after orphan cleanup"
                ));
            }

            Ok(())
        })();

        match rebuild_result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                // Explicit rollback on ANY error during the rebuild.
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }

        Ok(())
    }

    /// Audit all FK relationships in the schema for orphaned child rows.
    /// Returns a list of (relationship_description, orphan_count) pairs.
    /// This is used as the gate check for enabling PRAGMA foreign_keys=ON (PR 2).
    pub fn audit_fk_orphans(conn: &Connection) -> anyhow::Result<Vec<(String, i64)>> {
        let mut results = Vec::new();

        // constellation_fingerprints.anchor_memory_id → memories.id
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM constellation_fingerprints
             WHERE anchor_memory_id NOT IN (SELECT id FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push((
            "constellation_fingerprints.anchor_memory_id → memories.id".into(),
            count,
        ));

        // constellation_fingerprints.target_memory_id → memories.id
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM constellation_fingerprints
             WHERE target_memory_id NOT IN (SELECT id FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push((
            "constellation_fingerprints.target_memory_id → memories.id".into(),
            count,
        ));

        // memory_spectrogram.memory_id → memories.id
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_spectrogram
             WHERE memory_id NOT IN (SELECT id FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push(("memory_spectrogram.memory_id → memories.id".into(), count));

        // memory_annotations.memory_id → memories.id (already CASCADE, should be 0)
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_annotations
             WHERE memory_id NOT IN (SELECT id FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push(("memory_annotations.memory_id → memories.id".into(), count));

        // consolidation_edges.source_key → memories.key (already CASCADE, should be 0)
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM consolidation_edges
             WHERE source_key NOT IN (SELECT key FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push((
            "consolidation_edges.source_key → memories.key".into(),
            count,
        ));

        // co_retrieval_pairs: no FK constraint, but check logical orphans
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM co_retrieval_pairs
             WHERE memory_id_a NOT IN (SELECT id FROM memories)
                OR memory_id_b NOT IN (SELECT id FROM memories)",
            [],
            |row| row.get(0),
        )?;
        results.push((
            "co_retrieval_pairs.memory_id_a/b → memories.id (no FK)".into(),
            count,
        ));

        Ok(results)
    }

    /// Crate-internal raw connection access (tests + the `federation_sync`
    /// storage-layer module). Not exposed outside the crate.
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    /// Drop the entire wing read-cache. Call after ANY mutation that changes,
    /// removes, or adds a memory outside the `MemoryStore` write/delete methods —
    /// e.g. the federation import/tombstone paths mutate `memories` directly and
    /// would otherwise keep serving a stale (or deleted) row from `wing_search`.
    pub(crate) fn invalidate_wing_cache(&self) {
        wing_cache_clear(&self.wing_cache);
    }
}

/// Drop a single wing's cached entry (a mutation confined to one known wing).
fn wing_cache_pop(cache: &Arc<Mutex<LruCache<String, Vec<MemoryHit>>>>, wing: &str) {
    if let Ok(mut c) = cache.lock() {
        c.pop(wing);
    }
}

/// Drop every cached wing (a mutation whose affected wing(s) aren't cheaply known,
/// or that spans many). Correctness over cache-hit rate — these paths are rare
/// relative to reads, so the next `wing_search` simply repopulates.
fn wing_cache_clear(cache: &Arc<Mutex<LruCache<String, Vec<MemoryHit>>>>) {
    if let Ok(mut c) = cache.lock() {
        c.clear();
    }
}

/// Standard column list for memory queries.
const MEMORY_COLUMNS: &str = "id, key, content, wing, hall, signal_score, visibility, source, device_id, confidence, created_at, last_reinforced_at, episode_id, compaction_tier, declarative_density, description, description_generated_at, content_hash, source_brain_id, signature";

/// Read an optional 32-byte id blob at `idx`, tolerating a missing column
/// (queries with a narrower projection than [`MEMORY_COLUMNS`]).
fn opt_id32(row: &rusqlite::Row, idx: usize) -> Option<[u8; 32]> {
    let blob: Option<Vec<u8>> = row.get::<_, Option<Vec<u8>>>(idx).ok().flatten();
    blob.and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
}

/// Read an optional signature blob at `idx`, tolerating a missing column.
fn opt_blob(row: &rusqlite::Row, idx: usize) -> Option<Vec<u8>> {
    row.get::<_, Option<Vec<u8>>>(idx).ok().flatten()
}

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
        source_brain_id: opt_id32(row, 18),
        signature: opt_blob(row, 19),
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
        source_brain_id: opt_id32(row, 18),
        signature: opt_blob(row, 19),
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

            // Probe for existing row. Also read the STORED wing: a content update
            // keeps the row's existing wing, but the incoming `memory.wing` may
            // differ (classification is content-driven), so cache invalidation
            // must target the stored wing, not just the incoming one.
            let existing: Option<(Option<String>, String, Option<String>)> = tx
                .query_row(
                    "SELECT content_hash, content, wing FROM memories WHERE key = ?1",
                    params![memory.key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();
            let stored_wing: Option<String> = existing.as_ref().and_then(|(_, _, w)| w.clone());

            let outcome = match existing {
                None => {
                    // Case 1: No existing row — insert.
                    if memory.created_at.is_some() {
                        tx.execute(
                            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility,
                                                   source, device_id, confidence, created_at, content_hash,
                                                   source_brain_id, signature)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                                memory.source_brain_id.as_ref().map(|b| b.as_slice()),
                                memory.signature.as_deref(),
                            ],
                        )?;
                    } else {
                        tx.execute(
                            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility,
                                                   source, device_id, confidence, content_hash,
                                                   source_brain_id, signature)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                                memory.source_brain_id.as_ref().map(|b| b.as_slice()),
                                memory.signature.as_deref(),
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
                Some((existing_hash, existing_content, _)) => {
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

            // Invalidate the wing cache for BOTH the incoming wing and the stored
            // wing. On a content update the row keeps its stored wing while the
            // incoming classification may differ; invalidating only the incoming
            // wing would leave the stored wing serving pre-update content.
            if outcome != WriteOutcome::NoOp {
                if let Some(ref wing) = memory.wing {
                    wing_cache_pop(&wing_cache, wing);
                }
                if let Some(ref wing) = stored_wing {
                    wing_cache_pop(&wing_cache, wing);
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
                    -- Exclude consolidated sources BEFORE the LIMIT. Filtering after
                    -- (in the outer query) shrank the result below max_results when
                    -- top-hit memories were consolidated, hiding lower-ranked but
                    -- valid matches. Mirrors fts_search.
                    WHERE memory_id NOT IN (
                        SELECT id FROM memories
                        WHERE key IN (SELECT source_key FROM consolidation_edges)
                    )
                    GROUP BY memory_id
                    ORDER BY hits DESC
                    LIMIT ?{limit_param}
                )
                SELECT m.{cols}, ms.hits
                FROM memory_scores ms
                JOIN memories m ON m.id = ms.memory_id
                ORDER BY ms.hits DESC",
                hash_placeholders = hash_placeholders,
                cols = MEMORY_COLUMNS.replace(", ", ", m."),
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

            // Full MEMORY_COLUMNS projection (20 cols) + ms.hits at index 20, so the
            // shared row parser fills episode_id/description/source_brain_id/
            // signature (previously nulled by a 14-column projection).
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                let hits: i64 = row.get(20)?;
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
        query_terms: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        let wing = wing.to_string();
        let query_terms = query_terms.to_vec();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            // Full wing set, signal-ordered — from cache or DB. The cache holds
            // the untruncated, un-boosted set so different queries can reuse it.
            let mut all_results: Option<Vec<MemoryHit>> = None;
            if let Ok(mut cache) = wing_cache.lock() {
                if let Some(cached) = cache.get(&wing) {
                    all_results = Some(cached.clone());
                }
            }
            let mut all_results = match all_results {
                Some(r) => r,
                None => {
                    let fetched = {
                        let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
                        let sql = format!(
                            "SELECT {MEMORY_COLUMNS} FROM memories WHERE wing = ?1
                             ORDER BY signal_score DESC"
                        );
                        let mut stmt = conn.prepare(&sql)?;
                        let rows =
                            stmt.query_map(params![wing], |row| memory_hit_from_row(row, 0))?;
                        let mut fetched = Vec::new();
                        for row in rows {
                            fetched.push(row?);
                        }
                        fetched
                    };
                    if let Ok(mut cache) = wing_cache.lock() {
                        cache.put(wing, fetched.clone());
                    }
                    fetched
                }
            };

            // Query-term boost: stable re-rank by how many distinct query
            // terms each memory mentions. No memory is dropped (recall is
            // unchanged); within equal match counts the signal_score order is
            // preserved (stable sort). This makes Tier-2 wing retrieval
            // query-aware instead of a signal-ordered wing dump.
            let terms: Vec<String> = query_terms
                .iter()
                .filter(|t| t.len() > 1)
                .map(|t| t.to_lowercase())
                .collect();
            if !terms.is_empty() {
                all_results.sort_by_cached_key(|hit| {
                    let haystack =
                        format!("{} {}", hit.key.to_lowercase(), hit.content.to_lowercase());
                    let matches = terms
                        .iter()
                        .filter(|t| haystack.contains(t.as_str()))
                        .count();
                    std::cmp::Reverse(matches)
                });
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
        // FTS5 MATCH is superlinear in term count and runs under the connection
        // lock, so an unbounded term list stalls the whole store. Cap here — the
        // single chokepoint every recall path funnels through — so it holds even
        // for callers (e.g. the TACT extractor) that don't pre-cap their words.
        const MAX_FTS_TERMS: usize = 64;
        let query_words = &query_words[..query_words.len().min(MAX_FTS_TERMS)];
        let query = query_words
            .iter()
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        // Unstemmed-channel query with a CONSERVATIVE regular-plural singular
        // added per term (`engineers` → `"engineers" OR "engineer"`). This makes
        // the exact channel S-stemmer-like: it bridges regular plural↔singular
        // (which pure-unstemmed misses) WITHOUT porter's aggressive over-stemming
        // that floods on shared stems (`engineers`/`engineering` → `engin`). Only
        // a single trailing `s` is stripped, and only for len>3 words not ending
        // in `ss`/`us`/`is` — so `university`, `policy`, `class`, `status` are
        // untouched and the collision fixes are preserved.
        let raw_query = query_words
            .iter()
            .filter(|w| !w.is_empty())
            .flat_map(|w| {
                let cleaned = w.replace('"', "");
                let mut forms = vec![format!("\"{cleaned}\"")];
                let lower = cleaned.to_lowercase();
                if lower.len() > 3
                    && lower.ends_with('s')
                    && !lower.ends_with("ss")
                    && !lower.ends_with("us")
                    && !lower.ends_with("is")
                {
                    forms.push(format!("\"{}\"", &cleaned[..cleaned.len() - 1]));
                }
                forms
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let conn = self.conn.clone();
        let fusion = self.fusion;

        Box::pin(async move {
            if query.is_empty() {
                return Ok(Vec::new());
            }
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            if !fusion {
                let sql = format!(
                    "SELECT m.{cols}
                     FROM memories_fts fts
                     JOIN memories m ON m.rowid = fts.rowid
                     WHERE memories_fts MATCH ?1
                       AND m.key NOT IN (SELECT source_key FROM consolidation_edges)
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
                return Ok(results);
            }

            // ── Stemmed + unstemmed RRF fusion ──
            // Pull a deeper list from each channel so fusion can promote a
            // channel-specific winner that sits just past `max_results` in the
            // other channel, then fuse by rank and take the top `max_results`.
            let depth = (max_results.saturating_mul(2)).max(50) as i64;
            let ranked_ids =
                |table: &str, weights: &str, mq: &str| -> anyhow::Result<Vec<String>> {
                    let sql = format!(
                        "SELECT m.id
                     FROM {table} f
                     JOIN memories m ON m.rowid = f.rowid
                     WHERE {table} MATCH ?1
                       AND m.key NOT IN (SELECT source_key FROM consolidation_edges)
                     ORDER BY bm25({table}{weights}) LIMIT ?2"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt.query_map(params![mq, depth], |r| r.get::<_, String>(0))?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.push(r?);
                    }
                    Ok(out)
                };
            let porter = ranked_ids("memories_fts", ", 1.0, 1.0, 0.5", &query)?;
            let raw = ranked_ids("memories_fts_raw", "", &raw_query)?;

            // RRF (Cormack 2009), k=60. Rank-based → no normalization across the
            // two BM25 scales. Deterministic tie-break by id.
            const RRF_K: f64 = 60.0;
            let mut score: std::collections::HashMap<String, f64> =
                std::collections::HashMap::new();
            for list in [&porter, &raw] {
                for (rank, id) in list.iter().enumerate() {
                    *score.entry(id.clone()).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
                }
            }
            let mut fused: Vec<(String, f64)> = score.into_iter().collect();
            fused.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0))
            });
            fused.truncate(max_results);
            if fused.is_empty() {
                return Ok(Vec::new());
            }

            // Fetch the fused ids and re-emit in fused order.
            let placeholders = fused.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT m.{cols} FROM memories m WHERE m.id IN ({placeholders})",
                cols = MEMORY_COLUMNS.replace(", ", ", m."),
            );
            let mut stmt = conn.prepare(&sql)?;
            let params_vec: Vec<&dyn rusqlite::ToSql> = fused
                .iter()
                .map(|(id, _)| id as &dyn rusqlite::ToSql)
                .collect();
            let rows = stmt.query_map(params_vec.as_slice(), |row| memory_hit_from_row(row, 0))?;
            let mut by_id: std::collections::HashMap<String, MemoryHit> =
                std::collections::HashMap::new();
            for r in rows {
                let hit = r?;
                by_id.insert(hit.id.clone(), hit);
            }
            let results = fused
                .into_iter()
                .filter_map(|(id, _)| by_id.remove(&id))
                .collect();
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
            if ids.is_empty() {
                return Ok(results);
            }
            // One set-based query per chunk, not a per-id N+1. Chunk under SQLite's
            // ~999 bound-variable ceiling (`ids` is caller-supplied and unbounded).
            // Callers re-match by id, so result order is irrelevant; ids not found
            // simply return no row (the old per-id `query_row` swallowed the
            // not-found error).
            for chunk in ids.chunks(900) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                let sql =
                    format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id IN ({placeholders})");
                let mut stmt = conn.prepare(&sql)?;
                let params: Vec<&dyn rusqlite::ToSql> =
                    chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
                let rows = stmt.query_map(params.as_slice(), memory_from_row)?;
                for r in rows {
                    results.push(r?);
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

    fn reinforce_batch<'a>(
        &'a self,
        keys: &'a [String],
        strength: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + 'a>> {
        let keys: Vec<String> = keys.to_vec();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            if keys.is_empty() {
                return Ok(0);
            }
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // SQLite caps bound parameters per statement (999 on older builds).
            // Chunk so an arbitrarily large key set can never exceed it; the
            // common recall write-back (<= k keys) is a single chunk, unchanged.
            const CHUNK: usize = 900;
            let mut total_updated = 0usize;
            let mut all_wings: Vec<String> = Vec::new();

            for chunk in keys.chunks(CHUNK) {
                let placeholders = vec!["?"; chunk.len()].join(",");

                // Collect affected wings (for cache invalidation) — replaces the
                // per-key SELECT the single-key path issues.
                let sql = format!(
                    "SELECT DISTINCT wing FROM memories
                     WHERE key IN ({placeholders}) AND wing IS NOT NULL"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    row.get::<_, String>(0)
                })?;
                all_wings.extend(rows.filter_map(Result::ok));

                // One UPDATE per chunk. Same MIN(+strength, 1.0) semantics.
                let sql = format!(
                    "UPDATE memories SET
                        signal_score = MIN(signal_score + ?, 1.0),
                        last_reinforced_at = datetime('now'),
                        updated_at = datetime('now')
                     WHERE key IN ({placeholders})"
                );
                let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
                params.push(&strength);
                for k in chunk {
                    params.push(k);
                }
                total_updated += conn.execute(&sql, params.as_slice())?;
            }

            if let Ok(mut cache) = wing_cache.lock() {
                for w in &all_wings {
                    cache.pop(w);
                }
            }

            Ok(total_updated)
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
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let deleted = conn.execute(
                "DELETE FROM memories WHERE wing = ?1 AND created_at < ?2",
                params![wing, before],
            )?;
            drop(conn);
            wing_cache_pop(&wing_cache, &wing);
            Ok(deleted)
        })
    }

    fn delete_memory_by_key(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ForgetReceipt>> + Send + '_>> {
        let key = key.to_string();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();
        Box::pin(async move {
            use rusqlite::OptionalExtension;
            let mut conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            // Resolve the memory id (FK-CASCADE substrates key on id).
            let id: Option<String> = conn
                .query_row(
                    "SELECT id FROM memories WHERE key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(id) = id else {
                return Ok(ForgetReceipt::default());
            };

            let tx = conn.transaction()?;
            let mut receipt = ForgetReceipt {
                existed: true,
                ..Default::default()
            };

            // Non-FK substrates: scrub explicitly BEFORE the row delete so we
            // can count them and so nothing dangles.
            receipt.fingerprints = tx.execute(
                "DELETE FROM constellation_fingerprints \
                 WHERE anchor_memory_id = ?1 OR target_memory_id = ?1",
                params![id],
            )?;
            receipt.spectrograms = tx.execute(
                "DELETE FROM memory_spectrogram WHERE memory_id = ?1",
                params![id],
            )?;
            receipt.annotations = tx.execute(
                "DELETE FROM memory_annotations WHERE memory_id = ?1",
                params![id],
            )?;
            // Consolidation edges reference memories(key). FK cascades the
            // source side; delete target-side edges (pointers INTO the
            // deleted memory) explicitly so no edge dangles.
            receipt.consolidation_edges = tx.execute(
                "DELETE FROM consolidation_edges WHERE source_key = ?1 OR target_key = ?1",
                params![key],
            )?;
            receipt.co_retrieval_pairs = tx.execute(
                "DELETE FROM co_retrieval_pairs WHERE memory_id_a = ?1 OR memory_id_b = ?1",
                params![id],
            )?;
            // retrieval_events store memory ids in a JSON array; scrub any
            // event that referenced this memory (right-to-be-forgotten covers
            // the access log too). The quoted-id match avoids substring
            // collisions across the 16-hex-char ids. `write()` accepts arbitrary
            // caller ids, so escape LIKE metacharacters (`%`/`_`/`\`) and declare
            // ESCAPE — otherwise an id containing `%` would over-match and scrub
            // unrelated events.
            let escaped_id = id
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let id_needle = format!("%\"{escaped_id}\"%");
            receipt.retrieval_events = tx.execute(
                "DELETE FROM retrieval_events WHERE memory_ids_json LIKE ?1 ESCAPE '\\'",
                params![id_needle],
            )?;

            // The row delete cascades any FK substrates created before the
            // explicit scrubs above (idempotent — already emptied) and fires
            // the FTS AFTER DELETE trigger.
            receipt.memory_rows =
                tx.execute("DELETE FROM memories WHERE key = ?1", params![key])?;
            tx.commit()?;
            drop(conn);

            // Right-to-be-forgotten: the wing read-cache must not keep serving the
            // deleted memory. Clear it wholesale — the row's wing isn't loaded here
            // and a forget must be certain, not best-effort.
            wing_cache_clear(&wing_cache);

            // FTS is a shadow of the memories row; the trigger removed it.
            receipt.fts_rows = receipt.memory_rows;
            Ok(receipt)
        })
    }

    fn set_signature(
        &self,
        memory_id: &str,
        source_brain_id: &[u8; 32],
        signature: &[u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let sbid = source_brain_id.to_vec();
        let sig = signature.to_vec();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let n = conn.execute(
                "UPDATE memories SET source_brain_id = ?1, signature = ?2 WHERE id = ?3",
                params![sbid, sig, memory_id],
            )?;
            drop(conn);
            // source_brain_id/signature are served fields on a cached MemoryHit;
            // the wing isn't loaded here, so clear the cache.
            if n > 0 {
                wing_cache_clear(&wing_cache);
            }
            Ok(n)
        })
    }

    fn prune_wing_keeping_recent_per_source(
        &self,
        wing: &str,
        keep: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let wing = wing.to_string();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Get distinct sources in this wing
            let sources: Vec<String> = {
                let mut src_stmt = conn.prepare(
                    "SELECT DISTINCT source FROM memories WHERE wing = ?1 AND source IS NOT NULL",
                )?;
                let sources: Vec<String> = src_stmt
                    .query_map(params![wing], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                sources
            };

            let mut total_deleted = 0;
            // One transaction + one prepared DELETE across all sources, rather than
            // an autocommit (+ statement recompile) per source.
            let tx = conn.unchecked_transaction()?;
            {
                let mut del_stmt = tx.prepare(
                    "DELETE FROM memories WHERE wing = ?1 AND source = ?2 AND id NOT IN (\
                         SELECT id FROM memories WHERE wing = ?1 AND source = ?2 \
                         ORDER BY created_at DESC LIMIT ?3\
                     )",
                )?;
                for source in &sources {
                    total_deleted += del_stmt.execute(params![wing, source, keep as i64])?;
                }
            }
            tx.commit()?;
            drop(conn);
            if total_deleted > 0 {
                wing_cache_pop(&wing_cache, &wing);
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
        let wing_cache = self.wing_cache.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "UPDATE memories SET declarative_density = ?1 WHERE id = ?2",
                params![density, memory_id],
            )?;
            drop(conn);
            // declarative_density is a served field on a cached MemoryHit.
            wing_cache_clear(&wing_cache);
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
        let wing_cache = self.wing_cache.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            conn.execute(
                "UPDATE memories SET compaction_tier = ?1 WHERE id = ?2",
                params![tier_str, memory_id],
            )?;
            drop(conn);
            // compaction_tier is a served field on a cached MemoryHit.
            wing_cache_clear(&wing_cache);
            Ok(())
        })
    }

    fn backfill_fingerprint_time_buckets(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            // Fetch each fingerprint with its anchor/target timestamps AND the
            // hall/wing metadata needed to recompute the hash — in one query, not
            // a per-row follow-up SELECT (was an N+1: one query_row per fingerprint
            // for the same three columns already on this table).
            // (fp_id, anchor_ts, target_ts, anchor_hall, target_hall, wing)
            type FpBackfillRow = (
                String,
                Option<String>,
                Option<String>,
                String,
                String,
                String,
            );
            let rows: Vec<FpBackfillRow> = {
                let mut stmt = conn.prepare(
                    "SELECT f.id, m_anchor.created_at, m_target.created_at,
                            f.anchor_hall, f.target_hall, f.wing
                     FROM constellation_fingerprints f
                     JOIN memories m_anchor ON m_anchor.id = f.anchor_memory_id
                     JOIN memories m_target ON m_target.id = f.target_memory_id
                     WHERE f.time_delta_bucket = 'unknown' OR f.time_delta_bucket IS NULL",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            };

            if !rows.is_empty() {
                tracing::debug!(
                    count = rows.len(),
                    "found fingerprints with unknown/null time_delta_bucket, backfilling"
                );
            }

            let mut updated = 0;
            // One transaction for the whole backfill — this runs at brain open over
            // however many legacy fingerprints exist; per-row autocommit would be
            // one fsync each.
            let tx = conn.unchecked_transaction()?;
            {
                let mut update_stmt = tx.prepare(
                    "UPDATE constellation_fingerprints
                     SET time_delta_bucket = ?1,
                         fingerprint_hash = ?2
                     WHERE id = ?3",
                )?;

                for (fp_id, anchor_ts, target_ts, anchor_hall, target_hall, wing) in &rows {
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

                    // Recompute the hash with the new bucket, using metadata already
                    // fetched above.
                    let new_hash = {
                        use sha2::{Digest, Sha256};
                        let raw = format!(
                            "{}|{}|{}|{}",
                            anchor_hall,
                            target_hall,
                            wing,
                            bucket.as_str()
                        );
                        let digest = Sha256::digest(raw.as_bytes());
                        format!(
                            "{:016x}",
                            u64::from_be_bytes(digest[..8].try_into().unwrap())
                        )
                    };

                    update_stmt.execute(params![bucket.as_str(), new_hash, fp_id])?;
                    tracing::trace!(fp_id = %fp_id, bucket = bucket.as_str(), "backfilled fingerprint time bucket");
                    updated += 1;
                }
            }
            tx.commit()?;

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
        let wing_cache = self.wing_cache.clone();
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
            drop(conn);
            // description is a served field on a cached MemoryHit.
            wing_cache_clear(&wing_cache);
            Ok(())
        })
    }

    fn set_entity_field(
        &self,
        entity_id: &str,
        field_name: &str,
        value: &str,
        source: FieldSource,
        source_url: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let entity_id = entity_id.to_string();
        let field_name = field_name.to_string();
        let value = value.to_string();
        let source = source.as_str().to_string();
        let source_url = source_url.map(|s| s.to_string());
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            // Provenance guard: an enriched write must not clobber a manual
            // field. Read the stored source under the held lock so the
            // check-then-write is atomic against other writers.
            if source == "enriched" {
                let existing: Option<String> = conn
                    .query_row(
                        "SELECT source FROM entity_fields \
                         WHERE entity_id = ?1 AND field_name = ?2",
                        params![entity_id, field_name],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing.as_deref() == Some("manual") {
                    return Ok(false);
                }
            }
            conn.execute(
                "INSERT INTO entity_fields \
                     (entity_id, field_name, value, source, source_url, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
                 ON CONFLICT(entity_id, field_name) DO UPDATE SET \
                     value = excluded.value, \
                     source = excluded.source, \
                     source_url = excluded.source_url, \
                     updated_at = excluded.updated_at",
                params![entity_id, field_name, value, source, source_url],
            )?;
            Ok(true)
        })
    }

    fn get_entity_fields(
        &self,
        entity_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<EntityField>>> + Send + '_>> {
        let entity_id = entity_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT field_name, value, source, source_url, updated_at \
                 FROM entity_fields WHERE entity_id = ?1 ORDER BY field_name",
            )?;
            let rows: Vec<EntityField> = stmt
                .query_map(params![entity_id], |row| {
                    let source: String = row.get(2)?;
                    Ok(EntityField {
                        field_name: row.get(0)?,
                        value: row.get(1)?,
                        source: FieldSource::from_db(&source),
                        source_url: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
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
                        lift: 0.0,
                        memory: None,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
    }

    fn recommend_by_lift(
        &self,
        memory_id: &str,
        limit: usize,
        min_co_count: u64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RelatedMemory>>> + Send + '_>> {
        let memory_id = memory_id.to_string();
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            // lift(seed, B) = co(seed,B) * total / (occ(seed) * occ(B)), where
            // occ(X) = sum of co_count over pairs containing X, and total = sum
            // of all co_count. High occ(B) (a popular memory) drives lift DOWN,
            // suppressing the popularity bias that raw co_count ranking has.
            let mut stmt = conn.prepare(
                "WITH occ AS (
                    SELECT id, SUM(cc) AS occ FROM (
                        SELECT memory_id_a AS id, co_count AS cc FROM co_retrieval_pairs
                        UNION ALL
                        SELECT memory_id_b AS id, co_count AS cc FROM co_retrieval_pairs
                    ) GROUP BY id
                 ),
                 tot AS (SELECT SUM(co_count) AS total FROM co_retrieval_pairs),
                 neighbors AS (
                    SELECT memory_id_b AS rid, co_count FROM co_retrieval_pairs WHERE memory_id_a = ?1
                    UNION ALL
                    SELECT memory_id_a AS rid, co_count FROM co_retrieval_pairs WHERE memory_id_b = ?1
                 )
                 SELECT n.rid, n.co_count,
                        (n.co_count * (SELECT total FROM tot) * 1.0) /
                        (NULLIF((SELECT occ FROM occ WHERE id = ?1), 0) *
                         NULLIF((SELECT occ FROM occ WHERE id = n.rid), 0)) AS lift
                 FROM neighbors n
                 WHERE n.co_count >= ?2
                 ORDER BY lift DESC, n.co_count DESC
                 LIMIT ?3",
            )?;
            let rows: Vec<RelatedMemory> = stmt
                .query_map(
                    params![memory_id, min_co_count as i64, limit as i64],
                    |row| {
                        Ok(RelatedMemory {
                            memory_id: row.get(0)?,
                            co_count: row.get::<_, i64>(1)? as u64,
                            lift: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                            memory: None,
                        })
                    },
                )?
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
            let rows: Vec<(String, String)> = {
                let mut stmt =
                    conn.prepare("SELECT id, content FROM memories WHERE content_hash IS NULL")?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            };
            let count = rows.len();
            // One transaction + one prepared UPDATE: this stamps every legacy
            // NULL-hash row at brain open, so per-row autocommit (was re-preparing
            // the UPDATE too) would be one fsync each over the whole corpus.
            let tx = conn.unchecked_transaction()?;
            {
                let mut update_stmt =
                    tx.prepare("UPDATE memories SET content_hash = ?1 WHERE id = ?2")?;
                for (id, content) in &rows {
                    let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
                    update_stmt.execute(params![hash, id])?;
                }
            }
            tx.commit()?;
            Ok(count)
        })
    }

    fn consolidate_into(
        &self,
        source_keys: &[String],
        target_key: &str,
        opts: &ConsolidateOpts,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ConsolidationResult>> + Send + '_>> {
        let source_keys = source_keys.to_vec();
        let target_key = target_key.to_string();
        let opts = opts.clone();
        let conn = self.conn.clone();
        let wing_cache = self.wing_cache.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            // One transaction for the whole consolidation: an `AbortAll` bail after
            // an earlier source's edge was already written must roll back, or that
            // source is left consolidated (excluded from search) while the caller
            // is told the operation failed and the target never gained its signal.
            let tx = conn.unchecked_transaction()?;

            // Verify target exists
            let target_exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE key = ?1)",
                params![target_key],
                |row| row.get(0),
            )?;
            if !target_exists {
                if opts.on_invalid_source == InvalidSourcePolicy::AbortAll {
                    anyhow::bail!("target key '{}' does not exist", target_key);
                }
                // All sources get SkipReason — but actually target not found is fatal
                anyhow::bail!("target key '{}' does not exist", target_key);
            }

            let mut consolidated = Vec::new();
            let mut skipped = Vec::new();

            for source in &source_keys {
                // Check source == target
                if source == &target_key {
                    skipped.push((source.clone(), SkipReason::SourceEqualsTarget));
                    continue;
                }

                // Check source exists
                let source_exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM memories WHERE key = ?1)",
                    params![source],
                    |row| row.get(0),
                )?;
                if !source_exists {
                    if opts.on_invalid_source == InvalidSourcePolicy::AbortAll {
                        anyhow::bail!("source key '{}' does not exist", source);
                    }
                    skipped.push((source.clone(), SkipReason::SourceNotFound));
                    continue;
                }

                // Check if already consolidated elsewhere
                let existing_target: Option<String> = tx
                    .query_row(
                        "SELECT target_key FROM consolidation_edges WHERE source_key = ?1",
                        params![source],
                        |row| row.get(0),
                    )
                    .ok();

                if let Some(ref existing) = existing_target {
                    if existing == &target_key {
                        // Idempotent: same edge already exists
                        consolidated.push(source.clone());
                        continue;
                    } else {
                        skipped.push((
                            source.clone(),
                            SkipReason::AlreadyConsolidatedElsewhere(existing.clone()),
                        ));
                        continue;
                    }
                }

                // Insert edge
                tx.execute(
                    "INSERT OR IGNORE INTO consolidation_edges (source_key, target_key)
                     VALUES (?1, ?2)",
                    params![source, target_key],
                )?;
                consolidated.push(source.clone());

                // Chain flattening: if source was previously a target, add edges from its
                // inbound sources to the new target. Original edges preserved for history.
                tx.execute(
                    "INSERT OR IGNORE INTO consolidation_edges (source_key, target_key)
                     SELECT source_key, ?1
                     FROM consolidation_edges
                     WHERE target_key = ?2 AND source_key != ?1",
                    params![target_key, source],
                )?;
            }

            // Update target signal_score: sum source scores, capped at 1.0
            if !consolidated.is_empty() {
                let sum: f64 = {
                    let keys_csv: String = consolidated
                        .iter()
                        .map(|k| format!("'{}'", k.replace('\'', "''")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql =
                        format!("SELECT COALESCE(SUM(signal_score), 0.0) FROM memories WHERE key IN ({keys_csv})");
                    tx.query_row(&sql, [], |row| row.get(0))?
                };

                let current_score: f64 = tx.query_row(
                    "SELECT signal_score FROM memories WHERE key = ?1",
                    params![target_key],
                    |row| row.get(0),
                )?;
                let new_score = (current_score + sum).min(1.0);
                tx.execute(
                    "UPDATE memories SET signal_score = ?1 WHERE key = ?2",
                    params![new_score, target_key],
                )?;

                tx.commit()?;
                drop(conn);
                // Edges exclude sources from search and the target's signal changed;
                // both affect wing_search output.
                wing_cache_clear(&wing_cache);
                Ok(ConsolidationResult {
                    consolidated,
                    skipped,
                    target_score_after: new_score,
                })
            } else {
                let current_score: f64 = tx.query_row(
                    "SELECT signal_score FROM memories WHERE key = ?1",
                    params![target_key],
                    |row| row.get(0),
                )?;
                tx.commit()?;
                Ok(ConsolidationResult {
                    consolidated,
                    skipped,
                    target_score_after: current_score,
                })
            }
        })
    }

    fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ConsolidationEdge>>> + Send + '_>> {
        let target_key = target_key.map(|s| s.to_string());
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut edges = Vec::new();
            if let Some(ref target) = target_key {
                let mut stmt = conn.prepare(
                    "SELECT source_key, target_key, consolidated_at
                     FROM consolidation_edges WHERE target_key = ?1
                     ORDER BY consolidated_at DESC",
                )?;
                let rows = stmt.query_map(params![target], |row| {
                    Ok(ConsolidationEdge {
                        source_key: row.get(0)?,
                        target_key: row.get(1)?,
                        consolidated_at: row.get(2)?,
                    })
                })?;
                for row in rows {
                    edges.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT source_key, target_key, consolidated_at
                     FROM consolidation_edges
                     ORDER BY consolidated_at DESC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(ConsolidationEdge {
                        source_key: row.get(0)?,
                        target_key: row.get(1)?,
                        consolidated_at: row.get(2)?,
                    })
                })?;
                for row in rows {
                    edges.push(row?);
                }
            };
            Ok(edges)
        })
    }

    fn list_unconsolidated(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT m.key FROM memories m
                 WHERE NOT EXISTS (
                     SELECT 1 FROM consolidation_edges ce WHERE ce.source_key = m.key
                 )
                 ORDER BY m.created_at DESC LIMIT ?1",
            )?;
            let keys: Vec<String> = stmt
                .query_map(params![limit as i64], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(keys)
        })
    }

    fn consolidated_source_keys(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::collections::HashSet<String>>> + Send + '_>>
    {
        let conn = self.conn.clone();
        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare("SELECT source_key FROM consolidation_edges")?;
            let keys: std::collections::HashSet<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(keys)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delete_memory_by_key_purges_substrates() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "victim-id".into(),
            key: "victim".into(),
            content: "the doctor prescribed rest for the knee injury".into(),
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
            source_brain_id: None,
            signature: None,
        };
        // Give it a self-referential fingerprint so we can watch it purge.
        let fp = Fingerprint {
            id: "fp1".into(),
            hash: "h1".into(),
            anchor_memory_id: "victim-id".into(),
            target_memory_id: "victim-id".into(),
            wing: "general".into(),
            anchor_hall: "fact".into(),
            target_hall: "fact".into(),
            time_delta_bucket: "same".into(),
        };
        store.write(&mem, std::slice::from_ref(&fp)).await.unwrap();

        // Present in FTS and fingerprints before delete.
        let hits = store.fts_search(&["doctor".into()], 10).await.unwrap();
        assert_eq!(hits.len(), 1);

        let receipt = store.delete_memory_by_key("victim").await.unwrap();
        assert!(receipt.existed);
        assert_eq!(receipt.memory_rows, 1);
        assert_eq!(receipt.fts_rows, 1);
        assert_eq!(receipt.fingerprints, 1, "self-referential fp counted once");

        // Gone from FTS and the memories table.
        let after = store.fts_search(&["doctor".into()], 10).await.unwrap();
        assert!(after.is_empty(), "FTS shadow must be purged");
        // Scope the connection guard: holding it across a later store call
        // (which re-locks the same mutex) would deadlock.
        {
            let conn = store.conn();
            let remaining: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM constellation_fingerprints WHERE anchor_memory_id = 'victim-id'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(remaining, 0, "fingerprints must be purged");
        }

        // Deleting a missing key reports not-existed.
        let none = store.delete_memory_by_key("ghost").await.unwrap();
        assert!(!none.existed);
        assert_eq!(none.memory_rows, 0);
    }

    #[tokio::test]
    async fn wing_search_boosts_query_term_matches() {
        let store = SqliteStore::open_in_memory().unwrap();
        let base = Memory {
            id: String::new(),
            key: String::new(),
            content: String::new(),
            wing: Some("apollo".into()),
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
            source_brain_id: None,
            signature: None,
        };
        let mems = [
            ("m1", "k1", "high signal but off-topic content", 0.9),
            ("m2", "k2", "medium signal about deployment plans", 0.5),
            ("m3", "k3", "low signal note on kubernetes deployment", 0.1),
        ];
        for (id, key, content, signal) in mems {
            let mut m = base.clone();
            m.id = id.into();
            m.key = key.into();
            m.content = content.into();
            m.signal_score = signal;
            store.write(&m, &[]).await.unwrap();
        }

        // No terms: signal order preserved (wing dump).
        let plain = store.wing_search("apollo", &[], 3).await.unwrap();
        assert_eq!(plain[0].id, "m1", "no-term search stays signal-ordered");

        // Terms: matches outrank higher-signal non-matches; two-term match
        // outranks one-term match regardless of signal.
        let terms = vec!["kubernetes".to_string(), "deployment".to_string()];
        let boosted = store.wing_search("apollo", &terms, 3).await.unwrap();
        assert_eq!(
            boosted.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            vec!["m3", "m2", "m1"],
            "term overlap should outrank signal, signal breaks ties"
        );

        // Second call hits the LRU cache — the boost must still apply.
        let cached = store
            .wing_search("apollo", &["off-topic".to_string()], 3)
            .await
            .unwrap();
        assert_eq!(
            cached[0].id, "m1",
            "cache-path search must re-rank by the new query's terms"
        );
    }

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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
    async fn fts_rebuild_is_atomic_so_a_failed_rebuild_preserves_the_index() {
        // The FTS rebuild (DROP + CREATE + repopulate) must be one transaction:
        // if it fails partway, the original populated index must survive rather
        // than be left dropped-or-empty (which would silently break all recall).
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "k1".into(),
            content: "the quarterly budget review is scheduled".into(),
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
            source_brain_id: None,
            signature: None,
        };
        store.write(&mem, &[]).await.unwrap();
        assert_eq!(store.fts_search(&["budget".into()], 10).await.unwrap().len(), 1);

        // Run the real rebuild batch but with a syntax error appended so it fails
        // AFTER the DROP/CREATE — the transaction must roll the whole thing back.
        let batch = SqliteStore::fts_rebuild_batch(None);
        // Splice a guaranteed failure before the COMMIT.
        let poisoned = batch.replace("COMMIT;", "INSERT INTO no_such_table VALUES (1); COMMIT;");
        let err = store.conn().execute_batch(&poisoned);
        assert!(err.is_err(), "poisoned rebuild must fail");

        // The original index must still answer — the failed rebuild rolled back.
        let hits = store.fts_search(&["budget".into()], 10).await.unwrap();
        assert_eq!(
            hits.len(),
            1,
            "a failed rebuild must leave the original FTS index intact"
        );
    }

    #[tokio::test]
    async fn porter_tokenizer_bridges_plural_queries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("porter.db");
        let store = SqliteStore::open_with_config(
            &path,
            &SqliteStoreConfig {
                fts_tokenizer: Some("porter unicode61".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "session_1:turn:0:user".into(),
            content: "I met with the doctor yesterday about my knee".into(),
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
            source_brain_id: None,
            signature: None,
        };
        store.write(&mem, &[]).await.unwrap();

        // Plural query matches singular content under porter stemming.
        let hits = store.fts_search(&["doctors".into()], 10).await.unwrap();
        assert_eq!(hits.len(), 1, "porter should bridge doctors→doctor");

        // Control: an explicit unstemmed tokenizer misses the same query.
        let dir2 = tempfile::tempdir().unwrap();
        let unstemmed_store = SqliteStore::open_with_config(
            &dir2.path().join("unstemmed.db"),
            &SqliteStoreConfig {
                fts_tokenizer: Some("unicode61".into()),
                ..Default::default()
            },
        )
        .unwrap();
        unstemmed_store.write(&mem, &[]).await.unwrap();
        let hits = unstemmed_store
            .fts_search(&["doctors".into()], 10)
            .await
            .unwrap();
        assert!(hits.is_empty(), "unicode61 tokenizer has no stemming");
    }

    #[tokio::test]
    async fn default_config_uses_porter_stemming() {
        // Porter is the library default: plural queries bridge to singular
        // content with no explicit configuration.
        let store = SqliteStore::open_in_memory().unwrap();
        let mem = Memory {
            id: "m1".into(),
            key: "session_1:turn:0:user".into(),
            content: "I met with the doctor yesterday about my knee".into(),
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
            source_brain_id: None,
            signature: None,
        };
        store.write(&mem, &[]).await.unwrap();

        let hits = store.fts_search(&["doctors".into()], 10).await.unwrap();
        assert_eq!(hits.len(), 1, "default config should porter-stem");

        // The recorded schema carries the porter tokenize clause.
        let sql: String = store
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("porter unicode61"),
            "FTS schema should record the porter tokenizer, got: {sql}"
        );
    }

    #[tokio::test]
    async fn tokenizer_migration_rebuilds_existing_fts() {
        // A database built with an unstemmed tokenizer is rebuilt to the
        // resolved (porter) default on reopen, and the index is repopulated —
        // existing memories become stem-matchable without re-ingest.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrate.db");

        let mem = Memory {
            id: "m1".into(),
            key: "session_1:turn:0:user".into(),
            content: "I met with the doctor yesterday about my knee".into(),
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
            source_brain_id: None,
            signature: None,
        };

        {
            let old_store = SqliteStore::open_with_config(
                &path,
                &SqliteStoreConfig {
                    fts_tokenizer: Some("unicode61".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            old_store.write(&mem, &[]).await.unwrap();
            let hits = old_store.fts_search(&["doctors".into()], 10).await.unwrap();
            assert!(hits.is_empty(), "pre-migration store has no stemming");
        }

        // Reopen with the default config → porter → one-time FTS rebuild.
        let migrated = SqliteStore::open(&path).unwrap();
        let hits = migrated.fts_search(&["doctors".into()], 10).await.unwrap();
        assert_eq!(
            hits.len(),
            1,
            "migrated index should porter-stem existing content"
        );

        // Idempotent: reopening again leaves the schema stable and data intact.
        drop(migrated);
        let reopened = SqliteStore::open(&path).unwrap();
        let hits = reopened.fts_search(&["doctors".into()], 10).await.unwrap();
        assert_eq!(hits.len(), 1, "second reopen should not disturb the index");
        let sql: String = reopened
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("porter unicode61"),
            "migrated schema should record the porter tokenizer, got: {sql}"
        );
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
    async fn fingerprint_search_populates_full_row() {
        // Regression: the projection was 14 columns parsed by the 20-column row
        // parser, so episode_id/description/source_brain_id/signature were always
        // None on this retrieval channel.
        let store = SqliteStore::open_in_memory().unwrap();
        let mk = |id: &str, key: &str, content: &str, hall: &str| Memory {
            id: id.into(),
            key: key.into(),
            content: content.into(),
            wing: Some("w".into()),
            hall: Some(hall.into()),
            signal_score: 0.7,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: Some("ep1".into()),
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
            content_hash: None,
            source_brain_id: None,
            signature: None,
        };
        store.write(&mk("m0", "k0", "anchor", "event"), &[]).await.unwrap();
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
        store
            .write(&mk("m1", "k1", "findme", "fact"), &[fp])
            .await
            .unwrap();
        store.set_description("m1", "a helpful description").await.unwrap();

        let hits = store
            .fingerprint_search("w", "fact", &["abc123".to_string()], 10)
            .await
            .unwrap();
        let m1 = hits
            .iter()
            .find(|h| h.id == "m1")
            .expect("m1 should be a fingerprint hit");
        assert_eq!(m1.episode_id.as_deref(), Some("ep1"), "episode_id must round-trip");
        assert_eq!(
            m1.description.as_deref(),
            Some("a helpful description"),
            "description must round-trip"
        );
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
    async fn wing_cache_invalidated_on_forget() {
        // Regression: `delete_memory_by_key` returned a full ForgetReceipt while
        // the wing cache kept serving the deleted memory from `wing_search` — a
        // right-to-be-forgotten violation.
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .write(&make_mem("m1", "k1", "apollo"), &[])
            .await
            .unwrap();

        // Prime the cache.
        assert_eq!(store.wing_search("apollo", &[], 10).await.unwrap().len(), 1);
        assert!(store
            .wing_cache
            .lock()
            .unwrap()
            .peek(&"apollo".to_string())
            .is_some());

        // Forget it — the receipt claims a purge, so the cache must not survive.
        let receipt = store.delete_memory_by_key("k1").await.unwrap();
        assert!(receipt.existed);
        assert_eq!(receipt.memory_rows, 1);

        let after = store.wing_search("apollo", &[], 10).await.unwrap();
        assert!(
            after.is_empty(),
            "a forgotten memory must not be served from the wing cache"
        );
    }

    #[tokio::test]
    async fn consolidate_abort_all_rolls_back_partial_edges() {
        // Regression: consolidate_into ran without a transaction, so an AbortAll
        // bail after an earlier source's edge was written left that source
        // consolidated (excluded from search) while the caller was told it failed.
        let store = SqliteStore::open_in_memory().unwrap();
        store.write(&make_mem("t", "target", "w"), &[]).await.unwrap();
        store.write(&make_mem("s1", "src1", "w"), &[]).await.unwrap();
        // src2 intentionally absent → AbortAll must bail.

        let opts = ConsolidateOpts {
            on_invalid_source: InvalidSourcePolicy::AbortAll,
            ..Default::default()
        };
        let res = store
            .consolidate_into(
                &["src1".to_string(), "src2_missing".to_string()],
                "target",
                &opts,
            )
            .await;
        assert!(res.is_err(), "AbortAll with a missing source must fail");

        // src1's edge must have been rolled back — nothing consolidated.
        let edges = store.list_consolidated(None).await.unwrap();
        assert!(
            edges.is_empty(),
            "a failed AbortAll must leave no consolidation edges behind"
        );
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
                source_brain_id: None,
                signature: None,
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

    // ── entity_fields ──

    const E1: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    #[tokio::test]
    async fn entity_field_write_and_read_with_provenance() {
        let store = SqliteStore::open_in_memory().unwrap();

        let applied = store
            .set_entity_field(E1, "email", "j@x.com", FieldSource::Manual, None)
            .await
            .unwrap();
        assert!(applied);
        let applied = store
            .set_entity_field(
                E1,
                "job_title",
                "Eng Lead",
                FieldSource::Enriched,
                Some("https://linkedin.com/in/jane"),
            )
            .await
            .unwrap();
        assert!(applied);

        let fields = store.get_entity_fields(E1).await.unwrap();
        assert_eq!(fields.len(), 2);
        // Ordered by field_name: email, job_title.
        let email = &fields[0];
        assert_eq!(email.field_name, "email");
        assert_eq!(email.value, "j@x.com");
        assert_eq!(email.source, FieldSource::Manual);
        assert_eq!(email.source_url, None);
        assert!(email.updated_at.ends_with('Z'));

        let job = &fields[1];
        assert_eq!(job.field_name, "job_title");
        assert_eq!(job.source, FieldSource::Enriched);
        assert_eq!(
            job.source_url.as_deref(),
            Some("https://linkedin.com/in/jane")
        );
    }

    #[tokio::test]
    async fn enriched_write_does_not_clobber_manual_field() {
        let store = SqliteStore::open_in_memory().unwrap();

        store
            .set_entity_field(E1, "company", "Acme", FieldSource::Manual, None)
            .await
            .unwrap();

        // Enriched write to the same field must be suppressed.
        let applied = store
            .set_entity_field(
                E1,
                "company",
                "Globex",
                FieldSource::Enriched,
                Some("https://example.com"),
            )
            .await
            .unwrap();
        assert!(!applied, "enriched write should be suppressed");

        let fields = store.get_entity_fields(E1).await.unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].value, "Acme");
        assert_eq!(fields[0].source, FieldSource::Manual);
        assert_eq!(fields[0].source_url, None);
    }

    #[tokio::test]
    async fn manual_write_overwrites_any_source() {
        let store = SqliteStore::open_in_memory().unwrap();

        store
            .set_entity_field(E1, "company", "Globex", FieldSource::Enriched, None)
            .await
            .unwrap();
        // Manual write always wins.
        let applied = store
            .set_entity_field(E1, "company", "Acme", FieldSource::Manual, None)
            .await
            .unwrap();
        assert!(applied);

        let fields = store.get_entity_fields(E1).await.unwrap();
        assert_eq!(fields[0].value, "Acme");
        assert_eq!(fields[0].source, FieldSource::Manual);
        // And a subsequent enriched write is now blocked.
        let applied = store
            .set_entity_field(E1, "company", "Initech", FieldSource::Enriched, None)
            .await
            .unwrap();
        assert!(!applied);
    }

    #[tokio::test]
    async fn enriched_write_updates_existing_enriched_field() {
        let store = SqliteStore::open_in_memory().unwrap();

        store
            .set_entity_field(E1, "job_title", "Engineer", FieldSource::Enriched, None)
            .await
            .unwrap();
        let applied = store
            .set_entity_field(
                E1,
                "job_title",
                "Senior Engineer",
                FieldSource::Enriched,
                None,
            )
            .await
            .unwrap();
        assert!(applied);

        let fields = store.get_entity_fields(E1).await.unwrap();
        assert_eq!(fields[0].value, "Senior Engineer");
    }

    #[tokio::test]
    async fn get_entity_fields_empty_for_unknown_entity() {
        let store = SqliteStore::open_in_memory().unwrap();
        let fields = store.get_entity_fields(E1).await.unwrap();
        assert!(fields.is_empty());
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
    async fn lift_suppresses_popularity_bias_where_raw_cocount_fails() {
        // The documented co-retrieval failure: a globally-popular memory
        // co-occurs with everything, so raw co_count ranks it top for every
        // seed (a generic blob crowds out specific associations).
        //
        // Scenario: "popular" appears in EVERY retrieval. Seed "A" is
        // specifically associated with "B" (they co-occur across several
        // A-specific retrievals) but B is otherwise rare. Distractors C..F
        // pad popular's global frequency.
        let store = SqliteStore::open_in_memory().unwrap();
        // A-specific sessions: A, B, popular co-occur (B is A's real associate).
        for _ in 0..4 {
            insert_retrieval_event(&store, &["A", "B", "popular"]).await;
        }
        // Many other sessions where "popular" co-occurs with unrelated memories
        // (this is what makes it globally popular but not A-specific).
        for other in ["C", "D", "E", "F", "G", "H"] {
            for _ in 0..4 {
                insert_retrieval_event(&store, &[other, "popular"]).await;
            }
        }
        store.rebuild_co_retrieval_index().await.unwrap();

        // Raw co_count: "popular" co-occurs with A 4 times, "B" with A 4 times —
        // a tie or popular-favored; popular is NOT suppressed.
        let raw = store.related_memories("A", 5).await.unwrap();
        let raw_top = &raw[0].memory_id;
        // popular appears among A's raw neighbors at full weight.
        assert!(
            raw.iter().any(|r| r.memory_id == "popular"),
            "popular should be a raw neighbor of A"
        );

        // Lift: B is specifically associated with A (low global occ), popular is
        // globally popular (huge occ) -> B outranks popular under lift.
        let rec = store.recommend_by_lift("A", 5, 1).await.unwrap();
        assert_eq!(
            rec[0].memory_id,
            "B",
            "lift should rank the A-specific memory B first, got {:?}",
            rec.iter()
                .map(|r| (&r.memory_id, r.lift))
                .collect::<Vec<_>>()
        );
        let b_lift = rec.iter().find(|r| r.memory_id == "B").unwrap().lift;
        let pop_lift = rec
            .iter()
            .find(|r| r.memory_id == "popular")
            .map(|r| r.lift)
            .unwrap_or(0.0);
        assert!(
            b_lift > pop_lift,
            "B's lift ({b_lift}) must exceed popular's ({pop_lift}) — popularity suppressed"
        );
        // The fix is real: raw put popular at/near the top; lift demotes it.
        assert!(
            raw_top == "popular" || raw_top == "B",
            "sanity: raw top is popular or B (co_count tie), got {raw_top}"
        );
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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
            source_brain_id: None,
            signature: None,
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

    // ── Consolidation tests ────────────────────────────────────────

    fn insert_memory_with_key(store: &SqliteStore, key: &str, score: f64) {
        let conn = store.conn();
        conn.execute(
            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
             VALUES (?1, ?2, ?3, 'w', 'fact', ?4, 'private')",
            params![format!("id_{key}"), key, format!("content of {key}"), score],
        )
        .unwrap();
    }

    fn score_of(store: &SqliteStore, key: &str) -> f64 {
        store
            .conn()
            .query_row(
                "SELECT signal_score FROM memories WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn reinforce_batch_matches_per_key_and_clamps() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "a", 0.30);
        insert_memory_with_key(&store, "b", 0.50);
        insert_memory_with_key(&store, "hot", 0.995); // near the 1.0 ceiling

        let n = store
            .reinforce_batch(
                &["a".into(), "b".into(), "hot".into(), "missing".into()],
                0.01,
            )
            .await
            .unwrap();

        // Only the three existing keys updated; the batch applies the SAME
        // MIN(score + 0.01, 1.0) nudge as the single-key path, clamping "hot".
        assert_eq!(n, 3, "only existing keys count toward the update total");
        assert!((score_of(&store, "a") - 0.31).abs() < 1e-9);
        assert!((score_of(&store, "b") - 0.51).abs() < 1e-9);
        assert!(
            (score_of(&store, "hot") - 1.0).abs() < 1e-9,
            "must clamp at 1.0"
        );

        // last_reinforced_at was stamped (decay clock reset) for a reinforced key.
        let stamped: Option<String> = store
            .conn()
            .query_row(
                "SELECT last_reinforced_at FROM memories WHERE key = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            stamped.is_some(),
            "batch reinforce must set last_reinforced_at"
        );

        // Empty batch is a no-op, not an error.
        assert_eq!(store.reinforce_batch(&[], 0.01).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn reinforce_batch_chunks_past_the_param_ceiling() {
        // 2000 keys > SQLite's 999 bound-parameter limit on older builds — a
        // single IN-clause would error ("too many SQL variables"). Chunking must
        // apply the nudge to every existing key without error.
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..2000 {
            insert_memory_with_key(&store, &format!("k{i}"), 0.20);
        }
        let keys: Vec<String> = (0..2000).map(|i| format!("k{i}")).collect();
        let updated = store.reinforce_batch(&keys, 0.01).await.unwrap();
        assert_eq!(updated, 2000, "every key reinforced across chunks");
        assert!((score_of(&store, "k0") - 0.21).abs() < 1e-9);
        assert!((score_of(&store, "k1999") - 0.21).abs() < 1e-9);
    }

    #[tokio::test]
    async fn consolidate_into_basic() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "source1", 0.3);
        insert_memory_with_key(&store, "source2", 0.4);
        insert_memory_with_key(&store, "target", 0.2);

        let result = store
            .consolidate_into(
                &["source1".into(), "source2".into()],
                "target",
                &ConsolidateOpts::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.consolidated, vec!["source1", "source2"]);
        assert!(result.skipped.is_empty());
        // 0.2 + 0.3 + 0.4 = 0.9
        assert!((result.target_score_after - 0.9).abs() < 0.01);
    }

    #[tokio::test]
    async fn consolidate_into_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "src", 0.3);
        insert_memory_with_key(&store, "tgt", 0.2);

        let r1 = store
            .consolidate_into(&["src".into()], "tgt", &ConsolidateOpts::default())
            .await
            .unwrap();
        let r2 = store
            .consolidate_into(&["src".into()], "tgt", &ConsolidateOpts::default())
            .await
            .unwrap();

        assert_eq!(r1.consolidated, vec!["src"]);
        assert_eq!(r2.consolidated, vec!["src"]);
    }

    #[tokio::test]
    async fn consolidate_into_target_not_found() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "src", 0.5);

        let err = store
            .consolidate_into(&["src".into()], "nonexistent", &ConsolidateOpts::default())
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn consolidate_into_skip_reasons() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "src", 0.3);
        insert_memory_with_key(&store, "tgt1", 0.2);
        insert_memory_with_key(&store, "tgt2", 0.2);

        // Consolidate src into tgt1
        store
            .consolidate_into(&["src".into()], "tgt1", &ConsolidateOpts::default())
            .await
            .unwrap();

        // Try to consolidate src into tgt2 — should skip (already consolidated elsewhere)
        let result = store
            .consolidate_into(
                &["src".into(), "nonexistent".into(), "tgt2".into()],
                "tgt2",
                &ConsolidateOpts::default(),
            )
            .await
            .unwrap();

        assert!(result.consolidated.is_empty());
        assert_eq!(result.skipped.len(), 3);
        assert!(matches!(
            result.skipped[0].1,
            SkipReason::AlreadyConsolidatedElsewhere(_)
        ));
        assert!(matches!(result.skipped[1].1, SkipReason::SourceNotFound));
        assert!(matches!(
            result.skipped[2].1,
            SkipReason::SourceEqualsTarget
        ));
    }

    #[tokio::test]
    async fn consolidate_into_chain_flattening() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "a", 0.1);
        insert_memory_with_key(&store, "b", 0.2);
        insert_memory_with_key(&store, "c", 0.3);

        // A→B
        store
            .consolidate_into(&["a".into()], "b", &ConsolidateOpts::default())
            .await
            .unwrap();

        // B→C should flatten A→C
        store
            .consolidate_into(&["b".into()], "c", &ConsolidateOpts::default())
            .await
            .unwrap();

        // list_consolidated for C should show both A and B
        let edges_c = store.list_consolidated(Some("c")).await.unwrap();
        let sources_c: Vec<&str> = edges_c.iter().map(|e| e.source_key.as_str()).collect();
        assert!(sources_c.contains(&"a"));
        assert!(sources_c.contains(&"b"));

        // Original A→B edge preserved for history
        let edges_b = store.list_consolidated(Some("b")).await.unwrap();
        let sources_b: Vec<&str> = edges_b.iter().map(|e| e.source_key.as_str()).collect();
        assert!(sources_b.contains(&"a"));
    }

    #[tokio::test]
    async fn consolidate_into_score_capped_at_one() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "src", 0.9);
        insert_memory_with_key(&store, "tgt", 0.8);

        let result = store
            .consolidate_into(&["src".into()], "tgt", &ConsolidateOpts::default())
            .await
            .unwrap();

        assert!((result.target_score_after - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn list_unconsolidated_excludes_sources() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "a", 0.5);
        insert_memory_with_key(&store, "b", 0.5);
        insert_memory_with_key(&store, "c", 0.5);

        store
            .consolidate_into(&["a".into()], "c", &ConsolidateOpts::default())
            .await
            .unwrap();

        let keys = store.list_unconsolidated(10).await.unwrap();
        assert!(keys.contains(&"b".to_string()));
        assert!(keys.contains(&"c".to_string()));
        assert!(!keys.contains(&"a".to_string()));
    }

    #[tokio::test]
    async fn consolidated_source_keys_returns_set() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_memory_with_key(&store, "a", 0.5);
        insert_memory_with_key(&store, "b", 0.5);
        insert_memory_with_key(&store, "c", 0.5);

        store
            .consolidate_into(&["a".into(), "b".into()], "c", &ConsolidateOpts::default())
            .await
            .unwrap();

        let sources = store.consolidated_source_keys().await.unwrap();
        assert!(sources.contains("a"));
        assert!(sources.contains("b"));
        assert!(!sources.contains("c"));
    }

    // ── FK CASCADE migration tests ───────────────────────────────────

    /// Helper: create a DB with the OLD schema (NO CASCADE on fingerprints/spectrogram)
    /// and insert deliberately orphaned rows.
    fn create_old_schema_db_with_orphans() -> Connection {
        let conn = Connection::open_in_memory().unwrap();

        // Disable FK enforcement so we can insert orphaned rows for testing
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

        // Minimal memories table
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL,
                wing TEXT,
                hall TEXT,
                signal_score REAL DEFAULT 0.5,
                visibility TEXT NOT NULL DEFAULT 'private',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                source TEXT DEFAULT NULL,
                device_id BLOB DEFAULT NULL,
                confidence REAL NOT NULL DEFAULT 1.0
            );
            CREATE TABLE constellation_fingerprints (
                id TEXT PRIMARY KEY,
                fingerprint_hash TEXT NOT NULL,
                anchor_memory_id TEXT NOT NULL,
                target_memory_id TEXT NOT NULL,
                wing TEXT,
                anchor_hall TEXT,
                target_hall TEXT,
                time_delta_bucket TEXT,
                created_at TEXT,
                FOREIGN KEY (anchor_memory_id) REFERENCES memories(id),
                FOREIGN KEY (target_memory_id) REFERENCES memories(id)
            );
            CREATE INDEX IF NOT EXISTS idx_fp_hash ON constellation_fingerprints(fingerprint_hash);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_hash ON constellation_fingerprints(wing, fingerprint_hash);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_anchor_hall ON constellation_fingerprints(wing, anchor_hall);
            CREATE INDEX IF NOT EXISTS idx_fp_wing_target_hall ON constellation_fingerprints(wing, target_hall);
            CREATE TABLE memory_spectrogram (
                memory_id TEXT PRIMARY KEY,
                entity_density REAL,
                action_type TEXT,
                decision_polarity REAL,
                causal_depth REAL,
                emotional_valence REAL,
                temporal_specificity REAL,
                novelty REAL,
                peak_dimensions TEXT,
                created_at TEXT DEFAULT (datetime('now')),
                FOREIGN KEY (memory_id) REFERENCES memories(id)
            );
            CREATE INDEX IF NOT EXISTS idx_spectrogram_action ON memory_spectrogram(action_type);
            CREATE TABLE memory_annotations (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL,
                description TEXT NOT NULL,
                who TEXT NOT NULL,
                why TEXT NOT NULL,
                where_ TEXT,
                when_ TEXT NOT NULL,
                how TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
            );
            CREATE TABLE consolidation_edges (
                source_key TEXT NOT NULL,
                target_key TEXT NOT NULL,
                consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (source_key, target_key),
                FOREIGN KEY (source_key) REFERENCES memories(key) ON DELETE CASCADE
            );
            CREATE TABLE co_retrieval_pairs (
                memory_id_a TEXT NOT NULL,
                memory_id_b TEXT NOT NULL,
                co_count INTEGER NOT NULL DEFAULT 0,
                last_updated TEXT NOT NULL,
                PRIMARY KEY (memory_id_a, memory_id_b)
            );",
        )
        .unwrap();

        // Insert 2 valid memories
        conn.execute_batch(
            "INSERT INTO memories (id, key, content) VALUES ('m1', 'k1', 'valid memory 1');
             INSERT INTO memories (id, key, content) VALUES ('m2', 'k2', 'valid memory 2');",
        )
        .unwrap();

        // Insert valid fingerprints (both anchored to existing memories)
        conn.execute_batch(
            "INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id)
             VALUES ('fp1', 'hash1', 'm1', 'm2');
             INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id)
             VALUES ('fp2', 'hash2', 'm2', 'm1');",
        )
        .unwrap();

        // Insert ORPHANED fingerprints (reference non-existent memories)
        conn.execute_batch(
            "INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id)
             VALUES ('fp_orphan1', 'hash3', 'DELETED_M', 'm1');
             INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id)
             VALUES ('fp_orphan2', 'hash4', 'm1', 'DELETED_M');
             INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id)
             VALUES ('fp_orphan3', 'hash5', 'GONE_A', 'GONE_B');",
        )
        .unwrap();

        // Insert valid spectrogram row
        conn.execute_batch(
            "INSERT INTO memory_spectrogram (memory_id, entity_density) VALUES ('m1', 0.5)",
        )
        .unwrap();

        // Insert ORPHANED spectrogram row
        conn.execute_batch(
            "INSERT INTO memory_spectrogram (memory_id, entity_density) VALUES ('DELETED_M', 0.8)",
        )
        .unwrap();

        conn
    }

    #[test]
    fn fk_cascade_migration_removes_orphans() {
        let conn = create_old_schema_db_with_orphans();

        // Pre-migration: 5 fingerprints (2 valid + 3 orphans), 2 spectrograms (1 valid + 1 orphan)
        let fp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fp_count, 5, "pre-migration: 5 fingerprints");

        let spec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))
            .unwrap();
        assert_eq!(spec_count, 2, "pre-migration: 2 spectrograms");

        // Run migration
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // Post-migration: orphans removed
        let fp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            fp_count, 2,
            "post-migration: only 2 valid fingerprints remain"
        );

        let spec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            spec_count, 1,
            "post-migration: only 1 valid spectrogram remains"
        );
    }

    #[test]
    fn fk_cascade_migration_surviving_rows_intact() {
        let conn = create_old_schema_db_with_orphans();
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // Valid fingerprints still present with correct data
        let hash: String = conn
            .query_row(
                "SELECT fingerprint_hash FROM constellation_fingerprints WHERE id = 'fp1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hash, "hash1");

        // Valid spectrogram still present
        let density: f64 = conn
            .query_row(
                "SELECT entity_density FROM memory_spectrogram WHERE memory_id = 'm1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!((density - 0.5).abs() < 0.001);
    }

    #[test]
    fn fk_cascade_migration_foreign_key_check_clean() {
        let conn = create_old_schema_db_with_orphans();
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // PRAGMA foreign_key_check should return 0 violations
        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0, "no FK violations after migration");
    }

    #[test]
    fn fk_cascade_active_after_migration() {
        let conn = create_old_schema_db_with_orphans();
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // Enable FK enforcement to test CASCADE behavior
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        // Delete parent memory m1 — its fingerprints and spectrogram should cascade-delete
        conn.execute("DELETE FROM memories WHERE id = 'm1'", [])
            .unwrap();

        // fp1 (anchor=m1, target=m2) and fp2 (anchor=m2, target=m1) should both be gone
        // because m1 is referenced as anchor in fp1 and target in fp2
        let fp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            fp_count, 0,
            "CASCADE deleted both fingerprints referencing m1"
        );

        // Spectrogram for m1 should be gone
        let spec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))
            .unwrap();
        assert_eq!(spec_count, 0, "CASCADE deleted spectrogram for m1");
    }

    #[test]
    fn fk_cascade_migration_idempotent() {
        let conn = create_old_schema_db_with_orphans();

        // Run migration twice
        SqliteStore::migrate_fk_cascade(&conn).unwrap();
        SqliteStore::migrate_fk_cascade(&conn).unwrap(); // second run: no-op

        // Still 2 valid fingerprints
        let fp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fp_count, 2);

        // Still 1 valid spectrogram
        let spec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))
            .unwrap();
        assert_eq!(spec_count, 1);

        // FK check still clean
        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    #[test]
    fn fk_cascade_ddl_contains_cascade_after_migration() {
        let conn = create_old_schema_db_with_orphans();
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // Verify the DDL now contains ON DELETE CASCADE
        let fp_ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='constellation_fingerprints'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let fp_lower = fp_ddl.to_lowercase();
        assert!(
            fp_lower.contains("on delete cascade"),
            "fingerprints DDL should contain CASCADE: {fp_ddl}"
        );

        let spec_ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='memory_spectrogram'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let spec_lower = spec_ddl.to_lowercase();
        assert!(
            spec_lower.contains("on delete cascade"),
            "spectrogram DDL should contain CASCADE: {spec_ddl}"
        );
    }

    #[test]
    fn audit_fk_orphans_reports_orphans() {
        let conn = create_old_schema_db_with_orphans();

        // Before migration, audit should find orphans
        let results = SqliteStore::audit_fk_orphans(&conn).unwrap();
        let anchor_orphans = results
            .iter()
            .find(|(desc, _)| desc.contains("anchor_memory_id"))
            .unwrap()
            .1;
        assert!(anchor_orphans > 0, "should find orphaned anchor references");
    }

    #[test]
    fn audit_fk_orphans_clean_after_migration() {
        let conn = create_old_schema_db_with_orphans();
        SqliteStore::migrate_fk_cascade(&conn).unwrap();

        // After migration, all FK orphan counts should be 0
        let results = SqliteStore::audit_fk_orphans(&conn).unwrap();
        for (desc, count) in &results {
            assert_eq!(*count, 0, "orphans remain on: {desc}");
        }
    }
}
