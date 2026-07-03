//! Fingerprint index storage.
//!
//! `RecognitionStore` is the persistence seam: an in-memory implementation
//! for tests and embedding, and a SQLite implementation that can live in a
//! brain's `memory.db` or a sidecar file. Row growth is linear in enrolled
//! content (≈ max_peaks × fan_out pair rows + winnowed grams per memory) —
//! by design, unlike the retired quadratic constellation table.

use crate::extract::StimulusPrints;
use anyhow::Result;
use std::collections::HashMap;

/// A feature match returned by index lookup: (hash, memory_id, feature label,
/// document frequency of the hash across enrolled memories).
#[derive(Debug, Clone)]
pub struct FeatureMatch {
    pub hash: u64,
    pub memory_id: String,
    pub label: String,
    /// In how many enrolled memories this feature occurs (for rarity
    /// weighting at scoring time).
    pub doc_frequency: usize,
}

pub trait RecognitionStore {
    fn is_enrolled(&self, memory_id: &str) -> Result<bool>;
    fn enrolled_count(&self) -> Result<usize>;
    /// Index all fingerprints of a memory and mark it enrolled.
    fn index_memory(&mut self, memory_id: &str, prints: &StimulusPrints) -> Result<()>;
    /// All stored pair-feature matches for the given stimulus hashes.
    fn lookup_pairs(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>>;
    /// All stored gram-feature matches for the given stimulus hashes.
    fn lookup_grams(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>>;
}

// ── In-memory implementation ────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryRecognitionStore {
    enrolled: std::collections::HashSet<String>,
    /// hash → [(memory_id, label)]
    pairs: HashMap<u64, Vec<(String, String)>>,
    grams: HashMap<u64, Vec<(String, String)>>,
}

impl InMemoryRecognitionStore {
    fn lookup(
        index: &HashMap<u64, Vec<(String, String)>>,
        hashes: &[(u64, String)],
    ) -> Vec<FeatureMatch> {
        let mut out = Vec::new();
        for (h, _query_label) in hashes {
            if let Some(entries) = index.get(h) {
                let df = entries.len();
                for (memory_id, label) in entries {
                    out.push(FeatureMatch {
                        hash: *h,
                        memory_id: memory_id.clone(),
                        label: label.clone(),
                        doc_frequency: df,
                    });
                }
            }
        }
        out
    }
}

impl RecognitionStore for InMemoryRecognitionStore {
    fn is_enrolled(&self, memory_id: &str) -> Result<bool> {
        Ok(self.enrolled.contains(memory_id))
    }

    fn enrolled_count(&self) -> Result<usize> {
        Ok(self.enrolled.len())
    }

    fn index_memory(&mut self, memory_id: &str, prints: &StimulusPrints) -> Result<()> {
        self.enrolled.insert(memory_id.to_string());
        for (h, label) in &prints.pair_hashes {
            self.pairs
                .entry(*h)
                .or_default()
                .push((memory_id.to_string(), label.clone()));
        }
        for (h, label) in &prints.gram_hashes {
            self.grams
                .entry(*h)
                .or_default()
                .push((memory_id.to_string(), label.clone()));
        }
        Ok(())
    }

    fn lookup_pairs(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>> {
        Ok(Self::lookup(&self.pairs, hashes))
    }

    fn lookup_grams(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>> {
        Ok(Self::lookup(&self.grams, hashes))
    }
}

// ── SQLite implementation ───────────────────────────────────────────

/// SQLite-backed store. Owns its connection; point it at a brain's
/// `memory.db` or a sidecar `recognition.db`.
pub struct SqliteRecognitionStore {
    conn: rusqlite::Connection,
}

impl SqliteRecognitionStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS recognition_enrolled (
                memory_id TEXT PRIMARY KEY,
                enrolled_at TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE IF NOT EXISTS recognition_pairs (
                hash INTEGER NOT NULL,
                memory_id TEXT NOT NULL,
                label TEXT NOT NULL,
                PRIMARY KEY (hash, memory_id, label)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS recognition_grams (
                hash INTEGER NOT NULL,
                memory_id TEXT NOT NULL,
                label TEXT NOT NULL,
                PRIMARY KEY (hash, memory_id, label)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS idx_rec_pairs_hash ON recognition_pairs(hash);
             CREATE INDEX IF NOT EXISTS idx_rec_grams_hash ON recognition_grams(hash);",
        )?;
        Ok(Self { conn })
    }

    fn lookup_table(&self, table: &str, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        // SQLite stores u64 as i64; the bit pattern round-trips.
        let mut out = Vec::new();
        let sql = format!(
            "SELECT t.memory_id, t.label,
                    (SELECT COUNT(DISTINCT memory_id) FROM {table} d WHERE d.hash = t.hash) AS df
             FROM {table} t WHERE t.hash = ?1"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        for (h, _label) in hashes {
            let rows = stmt.query_map([*h as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (memory_id, label, df) = row?;
                out.push(FeatureMatch {
                    hash: *h,
                    memory_id,
                    label,
                    doc_frequency: df as usize,
                });
            }
        }
        Ok(out)
    }
}

impl RecognitionStore for SqliteRecognitionStore {
    fn is_enrolled(&self, memory_id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recognition_enrolled WHERE memory_id = ?1",
            [memory_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    fn enrolled_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM recognition_enrolled", [], |r| {
                r.get(0)
            })?;
        Ok(n as usize)
    }

    fn index_memory(&mut self, memory_id: &str, prints: &StimulusPrints) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO recognition_enrolled (memory_id) VALUES (?1)",
            [memory_id],
        )?;
        {
            let mut ins_pair = tx.prepare_cached(
                "INSERT OR IGNORE INTO recognition_pairs (hash, memory_id, label) VALUES (?1, ?2, ?3)",
            )?;
            for (h, label) in &prints.pair_hashes {
                ins_pair.execute(rusqlite::params![*h as i64, memory_id, label])?;
            }
            let mut ins_gram = tx.prepare_cached(
                "INSERT OR IGNORE INTO recognition_grams (hash, memory_id, label) VALUES (?1, ?2, ?3)",
            )?;
            for (h, label) in &prints.gram_hashes {
                ins_gram.execute(rusqlite::params![*h as i64, memory_id, label])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn lookup_pairs(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>> {
        self.lookup_table("recognition_pairs", hashes)
    }

    fn lookup_grams(&self, hashes: &[(u64, String)]) -> Result<Vec<FeatureMatch>> {
        self.lookup_table("recognition_grams", hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fingerprint_stimulus, RecognitionConfig};

    #[test]
    fn sqlite_store_roundtrip_matches_inmemory() {
        let cfg = RecognitionConfig::default();
        let dir = tempfile::tempdir().unwrap();
        let mut sq = SqliteRecognitionStore::open(&dir.path().join("rec.db")).unwrap();
        let mut mem = InMemoryRecognitionStore::default();

        let content = "The staging deploy failed with exit code 137 because the pod was OOMKilled";
        let prints = fingerprint_stimulus(content, &cfg);
        sq.index_memory("m1", &prints).unwrap();
        mem.index_memory("m1", &prints).unwrap();

        let query = fingerprint_stimulus("deploy failed exit code 137 OOMKilled", &cfg);
        let a = sq.lookup_pairs(&query.pair_hashes).unwrap();
        let b = mem.lookup_pairs(&query.pair_hashes).unwrap();
        assert_eq!(
            a.len(),
            b.len(),
            "both stores must return identical matches"
        );
        assert!(sq.is_enrolled("m1").unwrap());
        assert_eq!(sq.enrolled_count().unwrap(), 1);
    }

    #[test]
    fn sqlite_index_is_idempotent() {
        let cfg = RecognitionConfig::default();
        let dir = tempfile::tempdir().unwrap();
        let mut sq = SqliteRecognitionStore::open(&dir.path().join("rec.db")).unwrap();
        let prints = fingerprint_stimulus("hello unique world of testing", &cfg);
        sq.index_memory("m1", &prints).unwrap();
        sq.index_memory("m1", &prints).unwrap();
        let n: i64 = sq
            .conn
            .query_row("SELECT COUNT(*) FROM recognition_pairs", [], |r| r.get(0))
            .unwrap();
        let unique: i64 = sq
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT hash || memory_id || label) FROM recognition_pairs",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, unique, "re-indexing must not duplicate rows");
    }
}
