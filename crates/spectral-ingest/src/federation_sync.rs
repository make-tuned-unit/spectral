//! Federation sync — content-addressed, git-style replication of *shared wings*
//! between brains. See `docs/internal/federation-sync-design.md`.
//!
//! This is the plaintext, local, crypto-agnostic half: object model,
//! content-addressing, the export gate (the sovereignty invariant), and the
//! OR-Set merge. Identity, encryption of exported packs, and transport are the
//! caller's (Permagent) — this layer never sees keys, identity, or the network.
//!
//! Model:
//! - A **memory object** is a memory-version's source fields, content-addressed
//!   by [`object_hash`]. Derived indexes (FTS/BM25, fingerprints) are NOT part of
//!   the hash — the importer re-derives them locally.
//! - A **shared wing** is a manifest referencing member object-hashes (like a git
//!   tree references blobs). A memory that no manifest references is `Local` and
//!   is structurally unexportable — the sovereignty guarantee.
//! - Merge is an **OR-Set** of immutable content-addressed objects: union
//!   converges automatically; tombstones remove.

use crate::sqlite_store::SqliteStore;
use anyhow::Result;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

/// A memory object as it travels on the wire — source fields only. The importer
/// re-derives all indexes. Content-addressed by [`object_hash`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryObject {
    pub key: String,
    pub content: String,
    /// Authoring brain (32-byte BrainId), or `None` for unsigned/legacy.
    pub author_id: Option<[u8; 32]>,
    pub created_at: String,
    pub visibility: String,
    /// Prior object-hash this version supersedes (same-author chain), if any.
    pub supersedes: Option<String>,
}

/// A retraction: itself replicated like any object (OR-Set remove).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tombstone {
    pub target_hash: String,
    pub author_id: Option<[u8; 32]>,
    pub ts: String,
}

/// A pack for one shared wing — the "have you got these objects" payload the
/// caller encrypts and ships. Contains source objects + tombstones.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pack {
    pub wing_id: String,
    pub objects: Vec<MemoryObject>,
    pub tombstones: Vec<Tombstone>,
}

const OBJ_DOMAIN: &[u8] = b"spectral.federation.memobj.v1";

/// Content-address of a memory object: `blake3` over a canonical,
/// length-prefixed serialization of the SOURCE fields only. Deterministic and
/// config-independent, so identical content hashes identically on every brain.
pub fn object_hash(obj: &MemoryObject) -> String {
    let mut h = blake3::Hasher::new();
    h.update(OBJ_DOMAIN);
    // length-prefixed fields (no delimiter ambiguity)
    let field = |h: &mut blake3::Hasher, b: &[u8]| {
        h.update(&(b.len() as u64).to_le_bytes());
        h.update(b);
    };
    match obj.author_id {
        Some(a) => {
            h.update(&[1u8]);
            field(&mut h, &a);
        }
        None => {
            h.update(&[0u8]);
        }
    }
    field(&mut h, obj.key.as_bytes());
    field(&mut h, obj.created_at.as_bytes());
    field(&mut h, obj.content.as_bytes());
    field(&mut h, obj.visibility.as_bytes());
    field(&mut h, obj.supersedes.as_deref().unwrap_or("").as_bytes());
    h.finalize().to_hex().to_string()
}

impl MemoryObject {
    pub fn object_hash(&self) -> String {
        object_hash(self)
    }
}

fn author_short(author_id: &Option<[u8; 32]>) -> String {
    match author_id {
        Some(a) => hex8(&a[..4]),
        None => "anon".into(),
    }
}
fn hex8(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Create the sync tables (idempotent). A shared wing is its manifest; tombstones
/// are the OR-Set removals.
pub fn ensure_sync_tables(store: &SqliteStore) -> Result<()> {
    let conn = store.conn();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS shared_wing_members (
            wing_id     TEXT NOT NULL,
            object_hash TEXT NOT NULL,
            mem_key     TEXT NOT NULL,
            PRIMARY KEY (wing_id, object_hash)
         );
         CREATE TABLE IF NOT EXISTS sync_tombstones (
            wing_id     TEXT NOT NULL,
            target_hash TEXT NOT NULL,
            ts          TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (wing_id, target_hash)
         );",
    )?;
    Ok(())
}

/// Reconstruct a memory's object from its stored row (by local key).
fn object_for_key(store: &SqliteStore, mem_key: &str) -> Result<Option<MemoryObject>> {
    let conn = store.conn();
    let row = conn
        .query_row(
            "SELECT key, content, source_brain_id, created_at, visibility
             FROM memories WHERE key = ?1",
            rusqlite::params![mem_key],
            |r| {
                let author: Option<Vec<u8>> = r.get(2)?;
                Ok(MemoryObject {
                    key: r.get(0)?,
                    content: r.get(1)?,
                    author_id: author.and_then(|v| v.try_into().ok()),
                    created_at: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    visibility: r.get(4)?,
                    supersedes: None,
                })
            },
        )
        .ok();
    Ok(row)
}

/// Share a local memory (by its local key) into a shared wing: add its object to
/// the wing manifest. Returns the object hash. Only an existing memory can be
/// shared; a memory referenced by no manifest stays `Local` (unexportable).
pub fn share(store: &SqliteStore, mem_key: &str, wing_id: &str) -> Result<String> {
    ensure_sync_tables(store)?;
    let obj = object_for_key(store, mem_key)?
        .ok_or_else(|| anyhow::anyhow!("no memory with key {mem_key}"))?;
    let oh = obj.object_hash();
    let conn = store.conn();
    conn.execute(
        "INSERT OR IGNORE INTO shared_wing_members (wing_id, object_hash, mem_key)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![wing_id, oh, mem_key],
    )?;
    Ok(oh)
}

/// The object hashes currently live in a shared wing (manifest minus tombstones),
/// sorted for deterministic have/want negotiation.
pub fn enumerate(store: &SqliteStore, wing_id: &str) -> Result<Vec<String>> {
    ensure_sync_tables(store)?;
    let conn = store.conn();
    let mut stmt = conn.prepare(
        "SELECT object_hash FROM shared_wing_members
         WHERE wing_id = ?1
           AND object_hash NOT IN (SELECT target_hash FROM sync_tombstones WHERE wing_id = ?1)
         ORDER BY object_hash",
    )?;
    let rows = stmt.query_map(rusqlite::params![wing_id], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Export a shared wing as a pack. **Sovereignty gate:** only objects the wing
/// manifest references are packable — a `Local` memory (in no manifest) can never
/// appear here. Ships source fields only; the importer re-derives indexes.
pub fn export_pack(store: &SqliteStore, wing_id: &str) -> Result<Pack> {
    ensure_sync_tables(store)?;
    let members: Vec<(String, String)> = {
        let conn = store.conn();
        let mut stmt = conn
            .prepare("SELECT object_hash, mem_key FROM shared_wing_members WHERE wing_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![wing_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(Result::ok).collect()
    };
    let mut objects = Vec::new();
    for (expected_hash, mem_key) in members {
        if let Some(obj) = object_for_key(store, &mem_key)? {
            // Integrity: the stored row must still hash to the manifest entry.
            if obj.object_hash() == expected_hash {
                objects.push(obj);
            }
        }
    }
    let tombstones = {
        let conn = store.conn();
        let mut stmt =
            conn.prepare("SELECT target_hash, ts FROM sync_tombstones WHERE wing_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![wing_id], |r| {
            Ok(Tombstone {
                target_hash: r.get(0)?,
                author_id: None,
                ts: r.get(1)?,
            })
        })?;
        rows.filter_map(Result::ok).collect()
    };
    Ok(Pack {
        wing_id: wing_id.to_string(),
        objects,
        tombstones,
    })
}

/// Merge a pack into the local store (OR-Set union) and re-index. Imported
/// objects are stored under an author-scoped local key so cross-author same-key
/// contributions accumulate rather than overwrite; `id = object_hash` dedups
/// re-imports. FTS re-indexing happens via the memories AFTER-INSERT trigger.
/// Returns the number of new objects merged.
pub fn import_pack(store: &SqliteStore, pack: &Pack) -> Result<usize> {
    ensure_sync_tables(store)?;
    let mut merged = 0usize;
    // Re-derive classification locally, exactly as native ingest does — the
    // design's "ship source, re-derive on import" rule. Wing/hall/signal are
    // corpus/config-relative and are never shipped in the pack.
    let ingest_cfg = crate::ingest::IngestConfig::default();
    {
        let conn = store.conn();
        for obj in &pack.objects {
            let oh = obj.object_hash();
            let local_key = format!(
                "{}::{}::{}",
                pack.wing_id,
                author_short(&obj.author_id),
                obj.key
            );
            let content_hash = blake3::hash(obj.content.as_bytes()).to_hex().to_string();
            let author_blob: Option<Vec<u8>> = obj.author_id.map(|a| a.to_vec());
            let wing = crate::classifier::classify_wing(
                &obj.key,
                &obj.content,
                "core",
                &ingest_cfg.wing_rules,
            );
            let hall = crate::classifier::classify_hall(&obj.content, &ingest_cfg.hall_rules);
            let signal = crate::signal::score_memory(&obj.content, &hall);
            let n = conn.execute(
                "INSERT OR IGNORE INTO memories
                    (id, key, content, visibility, created_at, content_hash,
                     source_brain_id, wing, hall, signal_score, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1.0)",
                rusqlite::params![
                    oh,
                    local_key,
                    obj.content,
                    obj.visibility,
                    obj.created_at,
                    content_hash,
                    author_blob,
                    wing,
                    hall,
                    signal,
                ],
            )?;
            merged += n;
            conn.execute(
                "INSERT OR IGNORE INTO shared_wing_members (wing_id, object_hash, mem_key)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![pack.wing_id, oh, local_key],
            )?;
        }
    }
    for t in &pack.tombstones {
        apply_tombstone(store, &pack.wing_id, &t.target_hash)?;
    }
    Ok(merged)
}

/// Retract an object from a shared wing (OR-Set remove): record the tombstone,
/// drop it from the manifest, and hard-delete the local copy (FTS purge fires via
/// the AFTER-DELETE trigger). The tombstone dominates, so a later re-import of the
/// same object cannot resurrect it.
pub fn tombstone(store: &SqliteStore, wing_id: &str, target_hash: &str) -> Result<()> {
    ensure_sync_tables(store)?;
    apply_tombstone(store, wing_id, target_hash)
}

fn apply_tombstone(store: &SqliteStore, wing_id: &str, target_hash: &str) -> Result<()> {
    let conn = store.conn();
    conn.execute(
        "INSERT OR IGNORE INTO sync_tombstones (wing_id, target_hash) VALUES (?1, ?2)",
        rusqlite::params![wing_id, target_hash],
    )?;
    // The local rows this object was stored under (there may be one; id=hash).
    conn.execute(
        "DELETE FROM memories WHERE id = ?1",
        rusqlite::params![target_hash],
    )?;
    conn.execute(
        "DELETE FROM shared_wing_members WHERE wing_id = ?1 AND object_hash = ?2",
        rusqlite::params![wing_id, target_hash],
    )?;
    Ok(())
}

/// Where a recalled memory came from — the provenance a scope-spanning recall
/// attaches so the agent/UI can tell "team knowledge" from "mine."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The user's own private memory (referenced by no shared wing).
    Private,
    /// A memory merged from a shared wing, tagged with its authoring brain.
    Shared {
        wing_id: String,
        author_id: Option<[u8; 32]>,
    },
}

/// Have/want negotiation: the object hashes `remote` advertises that we lack —
/// the "want" set to request. Symmetric: swap the arguments for the "send" set.
pub fn missing_locally(local: &[String], remote: &[String]) -> Vec<String> {
    let have: std::collections::HashSet<&String> = local.iter().collect();
    remote
        .iter()
        .filter(|h| !have.contains(h))
        .cloned()
        .collect()
}

/// Provenance of a stored memory (by its local key): which shared wing it belongs
/// to and its authoring brain, or [`Origin::Private`] for a local-only memory.
/// Safe on a brain that has never shared anything (creates the sync tables).
pub fn provenance(store: &SqliteStore, mem_key: &str) -> Result<Origin> {
    ensure_sync_tables(store)?;
    let conn = store.conn();
    let wing: Option<String> = conn
        .query_row(
            "SELECT wing_id FROM shared_wing_members WHERE mem_key = ?1 LIMIT 1",
            rusqlite::params![mem_key],
            |r| r.get(0),
        )
        .optional()?;
    match wing {
        None => Ok(Origin::Private),
        Some(wing_id) => {
            let author: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT source_brain_id FROM memories WHERE key = ?1",
                    rusqlite::params![mem_key],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            Ok(Origin::Shared {
                wing_id,
                author_id: author.and_then(|v| v.try_into().ok()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore; // fts_search (trait method)

    fn insert_local(store: &SqliteStore, key: &str, content: &str, author: Option<[u8; 32]>) {
        let conn = store.conn();
        let blob: Option<Vec<u8>> = author.map(|a| a.to_vec());
        conn.execute(
            "INSERT INTO memories (id, key, content, visibility, created_at, source_brain_id)
             VALUES (?1, ?2, ?3, 'team', '2026/01/01 (Thu) 10:00', ?4)",
            rusqlite::params![format!("local_{key}"), key, content, blob],
        )
        .unwrap();
    }

    #[test]
    fn missing_locally_is_the_want_set() {
        let local = vec!["a".to_string(), "b".to_string()];
        let remote = vec!["b".to_string(), "c".to_string(), "d".to_string()];
        assert_eq!(missing_locally(&local, &remote), vec!["c", "d"]);
        // symmetric: 'a' is local-only, so remote lacks it
        assert_eq!(missing_locally(&remote, &local), vec!["a"]);
        assert!(missing_locally(&local, &local).is_empty());
    }

    #[tokio::test]
    async fn provenance_distinguishes_shared_from_private() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_local(&store, "mine", "my own private note", None);
        insert_local(&store, "ours", "a shared team note", Some([7u8; 32]));
        share(&store, "ours", "team").unwrap();

        assert_eq!(provenance(&store, "mine").unwrap(), Origin::Private);
        match provenance(&store, "ours").unwrap() {
            Origin::Shared { wing_id, author_id } => {
                assert_eq!(wing_id, "team");
                assert_eq!(author_id, Some([7u8; 32]));
            }
            Origin::Private => panic!("shared memory tagged Private"),
        }
    }

    /// SOVEREIGNTY: a memory that is never shared (no manifest entry) can never
    /// appear in any export pack — the structural privacy guarantee.
    #[tokio::test]
    async fn local_memory_is_never_exportable() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_local(
            &store,
            "secret",
            "rotate the prod credentials on friday",
            None,
        );
        insert_local(&store, "public", "the deploy runbook lives in notion", None);
        share(&store, "public", "team-ops").unwrap(); // only 'public' is shared

        let pack = export_pack(&store, "team-ops").unwrap();
        let contents: Vec<&str> = pack.objects.iter().map(|o| o.content.as_str()).collect();
        assert!(
            !contents.iter().any(|c| c.contains("rotate the prod")),
            "a never-shared (Local) memory leaked into the export pack"
        );
        assert!(
            contents.iter().any(|c| c.contains("deploy runbook")),
            "the shared memory should be in the pack"
        );
    }

    /// CONVERGENCE: two brains importing the same pack reach the identical shared
    /// object set, and re-import is idempotent (content-addressed OR-Set union).
    #[tokio::test]
    async fn two_brains_converge_on_the_same_object_set() {
        // Author A's brain: two shared memories.
        let a = SqliteStore::open_in_memory().unwrap();
        insert_local(
            &a,
            "k1",
            "quarterly planning moved to thursday",
            Some([1u8; 32]),
        );
        insert_local(
            &a,
            "k2",
            "the staging cluster is in frankfurt",
            Some([1u8; 32]),
        );
        share(&a, "k1", "team").unwrap();
        share(&a, "k2", "team").unwrap();
        let pack = export_pack(&a, "team").unwrap();

        // Two fresh brains import it independently.
        let b = SqliteStore::open_in_memory().unwrap();
        let c = SqliteStore::open_in_memory().unwrap();
        import_pack(&b, &pack).unwrap();
        import_pack(&c, &pack).unwrap();

        let ea = enumerate(&a, "team").unwrap();
        let eb = enumerate(&b, "team").unwrap();
        let ec = enumerate(&c, "team").unwrap();
        assert_eq!(
            eb, ec,
            "replicas B and C must converge on the same object set"
        );
        assert_eq!(ea, eb, "importers must match the source wing's object set");
        assert_eq!(ea.len(), 2);

        // Re-import is idempotent — no duplication, still converged.
        let n = import_pack(&b, &pack).unwrap();
        assert_eq!(n, 0, "re-import merges nothing new");
        assert_eq!(enumerate(&b, "team").unwrap(), ea);

        // Imported content is recall-indexed (FTS trigger fired on insert).
        let hits = b.fts_search(&["frankfurt".into()], 10).await.unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("frankfurt")),
            "imported shared memory should be searchable locally"
        );
    }

    /// Imported memories are re-classified locally (wing/hall/signal derived at
    /// import, not shipped) — the "re-derive on import" rule, so reranking and
    /// TACT tiers treat imported content as first-class, not unclassified blobs.
    #[tokio::test]
    async fn import_rederives_classification_locally() {
        let a = SqliteStore::open_in_memory().unwrap();
        insert_local(
            &a,
            "k1",
            "we decided to deploy the api on friday",
            Some([1u8; 32]),
        );
        share(&a, "k1", "team").unwrap();
        let pack = export_pack(&a, "team").unwrap();

        let b = SqliteStore::open_in_memory().unwrap();
        import_pack(&b, &pack).unwrap();
        let (wing, hall, signal): (Option<String>, Option<String>, f64) = b
            .conn()
            .query_row(
                "SELECT wing, hall, signal_score FROM memories WHERE key LIKE 'team::%'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(
            wing.is_some(),
            "imported memory must get a locally-derived wing"
        );
        assert!(
            hall.is_some(),
            "imported memory must get a locally-derived hall"
        );
        assert!(
            signal > 0.0,
            "imported memory must get a derived signal score"
        );
    }

    /// Cross-author same-key contributions ACCUMULATE (no destructive overwrite):
    /// two authors' `deploy-process` are distinct objects and both survive.
    #[tokio::test]
    async fn cross_author_same_key_accumulates() {
        let a = SqliteStore::open_in_memory().unwrap();
        insert_local(
            &a,
            "deploy-process",
            "A says: deploy via the release channel",
            Some([1u8; 32]),
        );
        share(&a, "deploy-process", "team").unwrap();
        let pa = export_pack(&a, "team").unwrap();

        let b = SqliteStore::open_in_memory().unwrap();
        insert_local(
            &b,
            "deploy-process",
            "B says: deploy needs two approvals",
            Some([2u8; 32]),
        );
        share(&b, "deploy-process", "team").unwrap();
        let pb = export_pack(&b, "team").unwrap();

        // A merged brain imports both.
        let m = SqliteStore::open_in_memory().unwrap();
        import_pack(&m, &pa).unwrap();
        import_pack(&m, &pb).unwrap();
        assert_eq!(
            enumerate(&m, "team").unwrap().len(),
            2,
            "both authors' same-key memories accumulate as distinct objects"
        );
        let hits = m.fts_search(&["deploy".into()], 10).await.unwrap();
        assert!(hits.iter().any(|h| h.content.contains("release channel")));
        assert!(hits.iter().any(|h| h.content.contains("two approvals")));
    }

    /// TOMBSTONE: a retracted object leaves the wing across replicas and cannot be
    /// resurrected by a later re-import of the original pack.
    #[tokio::test]
    async fn tombstone_removes_and_prevents_resurrection() {
        let a = SqliteStore::open_in_memory().unwrap();
        insert_local(&a, "k1", "keep this shared note", Some([1u8; 32]));
        insert_local(&a, "k2", "retract this shared note", Some([1u8; 32]));
        share(&a, "k1", "team").unwrap();
        let doomed = share(&a, "k2", "team").unwrap();
        let pack = export_pack(&a, "team").unwrap();

        let b = SqliteStore::open_in_memory().unwrap();
        import_pack(&b, &pack).unwrap();
        assert_eq!(enumerate(&b, "team").unwrap().len(), 2);

        // Retract on A, propagate the tombstone via a fresh pack.
        tombstone(&a, "team", &doomed).unwrap();
        let pack2 = export_pack(&a, "team").unwrap();
        import_pack(&b, &pack2).unwrap();
        let after = enumerate(&b, "team").unwrap();
        assert!(
            !after.contains(&doomed),
            "tombstoned object must leave the wing"
        );
        assert_eq!(after.len(), 1);

        // Re-importing the ORIGINAL pack must not resurrect it (tombstone dominates).
        import_pack(&b, &pack).unwrap();
        assert!(
            !enumerate(&b, "team").unwrap().contains(&doomed),
            "a tombstoned object must not be resurrectable by re-import"
        );
    }
}
