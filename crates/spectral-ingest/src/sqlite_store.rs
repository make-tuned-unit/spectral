//! Concrete `MemoryStore` implementation using rusqlite with FTS5.

use crate::{Fingerprint, Memory, MemoryHit, MemoryStore};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// SQLite-backed memory store with FTS5 search.
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish_non_exhaustive()
    }
}

impl SqliteStore {
    /// Open or create a memory database at the given path.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA temp_store   = MEMORY;",
        )?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database (useful for tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
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
                updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
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
                ON constellation_fingerprints(wing, fingerprint_hash);",
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
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

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            conn.execute(
                "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(key) DO UPDATE SET
                    content = excluded.content,
                    wing = excluded.wing,
                    hall = excluded.hall,
                    signal_score = excluded.signal_score,
                    visibility = excluded.visibility,
                    updated_at = datetime('now')",
                params![
                    memory.id,
                    memory.key,
                    memory.content,
                    memory.wing,
                    memory.hall,
                    memory.signal_score,
                    memory.visibility,
                ],
            )?;

            for fp in &fingerprints {
                conn.execute(
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
            let mut stmt = conn.prepare(
                "SELECT id, key, content, wing, hall, signal_score, visibility
                 FROM memories WHERE wing = ?1 AND signal_score >= ?2
                 ORDER BY signal_score DESC",
            )?;
            let rows = stmt.query_map(params![wing, min_signal], |row| {
                Ok(Memory {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    wing: row.get(3)?,
                    hall: row.get(4)?,
                    signal_score: row.get(5)?,
                    visibility: row.get::<_, String>(6).unwrap_or_else(|_| "private".into()),
                })
            })?;
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
        _hall: &str,
        hashes: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        let wing = wing.to_string();
        let hashes = hashes.to_vec();
        let conn = self.conn.clone();

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

            if hashes.is_empty() {
                return Ok(Vec::new());
            }

            let mut id_hits: HashMap<String, usize> = HashMap::new();
            let mut stmt = conn.prepare(
                "SELECT anchor_memory_id, target_memory_id
                 FROM constellation_fingerprints
                 WHERE wing = ?1 AND fingerprint_hash = ?2",
            )?;

            for hash in &hashes {
                let rows = stmt.query_map(params![wing, hash], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (anchor, target) = row?;
                    *id_hits.entry(anchor).or_insert(0) += 1;
                    *id_hits.entry(target).or_insert(0) += 1;
                }
            }

            let mut entries: Vec<_> = id_hits.into_iter().collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.1));
            entries.truncate(max_results);

            let mut results = Vec::new();
            for (id, hits) in entries {
                let mem: Option<Memory> = conn
                    .query_row(
                        "SELECT id, key, content, wing, hall, signal_score, visibility
                         FROM memories WHERE id = ?1",
                        params![id],
                        |row| {
                            Ok(Memory {
                                id: row.get(0)?,
                                key: row.get(1)?,
                                content: row.get(2)?,
                                wing: row.get(3)?,
                                hall: row.get(4)?,
                                signal_score: row.get(5)?,
                                visibility: row
                                    .get::<_, String>(6)
                                    .unwrap_or_else(|_| "private".into()),
                            })
                        },
                    )
                    .ok();
                if let Some(m) = mem {
                    results.push(MemoryHit {
                        id: m.id,
                        key: m.key,
                        content: m.content,
                        wing: m.wing,
                        hall: m.hall,
                        signal_score: m.signal_score,
                        visibility: m.visibility,
                        hits,
                    });
                }
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

        Box::pin(async move {
            let conn = conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let mut stmt = conn.prepare(
                "SELECT id, key, content, wing, hall, signal_score, visibility
                 FROM memories WHERE wing = ?1
                 ORDER BY signal_score DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![wing, max_results as i64], |row| {
                Ok(MemoryHit {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    wing: row.get(3)?,
                    hall: row.get(4)?,
                    signal_score: row.get(5)?,
                    visibility: row.get::<_, String>(6).unwrap_or_else(|_| "private".into()),
                    hits: 0,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
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
            let mut stmt = conn.prepare(
                "SELECT m.id, m.key, m.content, m.wing, m.hall, m.signal_score, m.visibility
                 FROM memories_fts fts
                 JOIN memories m ON m.rowid = fts.rowid
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![query, max_results as i64], |row| {
                Ok(MemoryHit {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    wing: row.get(3)?,
                    hall: row.get(4)?,
                    signal_score: row.get(5)?,
                    visibility: row.get::<_, String>(6).unwrap_or_else(|_| "private".into()),
                    hits: 0,
                })
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
                if let Ok(mem) = conn.query_row(
                    "SELECT id, key, content, wing, hall, signal_score, visibility
                     FROM memories WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(Memory {
                            id: row.get(0)?,
                            key: row.get(1)?,
                            content: row.get(2)?,
                            wing: row.get(3)?,
                            hall: row.get(4)?,
                            signal_score: row.get(5)?,
                            visibility: row
                                .get::<_, String>(6)
                                .unwrap_or_else(|_| "private".into()),
                        })
                    },
                ) {
                    results.push(mem);
                }
            }
            Ok(results)
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
}
