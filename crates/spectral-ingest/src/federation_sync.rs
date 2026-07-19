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
        // `orig_key`/`supersedes` are the wire object's ORIGINAL identity, kept so
        // an imported object can be re-exported and round-trip its `object_hash`
        // (the local `mem_key` is a synthetic, injective storage key for imports
        // and would not re-hash to the manifest entry). For a natively-shared
        // memory these equal its own key / `NULL`.
        "CREATE TABLE IF NOT EXISTS shared_wing_members (
            wing_id     TEXT NOT NULL,
            object_hash TEXT NOT NULL,
            mem_key     TEXT NOT NULL,
            orig_key    TEXT,
            supersedes  TEXT,
            PRIMARY KEY (wing_id, object_hash)
         );
         CREATE TABLE IF NOT EXISTS sync_tombstones (
            wing_id     TEXT NOT NULL,
            target_hash TEXT NOT NULL,
            ts          TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (wing_id, target_hash)
         );",
    )?;
    // Idempotent migration for manifests created before the round-trip identity
    // columns existed. ADD COLUMN errors if the column is already present; that
    // (and only that) is the expected no-op, so the result is intentionally
    // discarded.
    let _ = conn.execute(
        "ALTER TABLE shared_wing_members ADD COLUMN orig_key TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE shared_wing_members ADD COLUMN supersedes TEXT",
        [],
    );
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
    // For a native share the local `mem_key` IS the wire key and there is no
    // supersede, so `orig_key = mem_key` and `supersedes = NULL` — recorded
    // explicitly so export reconstructs from the same columns for shared and
    // imported objects alike.
    conn.execute(
        "INSERT OR IGNORE INTO shared_wing_members
            (wing_id, object_hash, mem_key, orig_key, supersedes)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![wing_id, oh, mem_key, obj.key, obj.supersedes],
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
    let conn = store.conn();
    // One set-based join, not an N+1 (was: fetch the manifest keys, then a
    // per-member SELECT). Each wire object is reconstructed from the manifest's
    // stored ORIGINAL identity (`orig_key`/`supersedes`) joined to the memory's
    // shipped fields, so an imported object re-hashes to its manifest entry and
    // can be relayed onward. `COALESCE(orig_key, mem_key)` handles rows written
    // before the identity columns existed (native shares, whose key was lossless).
    let objects = {
        let mut stmt = conn.prepare(
            "SELECT s.object_hash, COALESCE(s.orig_key, s.mem_key), s.supersedes,
                    m.content, m.source_brain_id, m.created_at, m.visibility
             FROM shared_wing_members s
             JOIN memories m ON m.key = s.mem_key
             WHERE s.wing_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![wing_id], |r| {
            let expected_hash: String = r.get(0)?;
            let author: Option<Vec<u8>> = r.get(4)?;
            let obj = MemoryObject {
                key: r.get(1)?,
                content: r.get(3)?,
                author_id: author.and_then(|v| v.try_into().ok()),
                created_at: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                visibility: r.get(6)?,
                supersedes: r.get(2)?,
            };
            Ok((expected_hash, obj))
        })?;
        let mut objects = Vec::new();
        for row in rows {
            let (expected_hash, obj) = row?;
            // Integrity: the reconstructed object must still hash to its manifest
            // entry (defends against a locally-mutated content/identity row).
            if obj.object_hash() == expected_hash {
                objects.push(obj);
            }
        }
        objects
    };
    let tombstones = {
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
/// objects are stored under an object-scoped local key (`author::key::hash`) so
/// distinct contributions accumulate rather than overwrite, while the SAME object
/// shared into several wings still maps to one local row (`id = object_hash`
/// dedups). FTS re-indexing happens via the memories AFTER-INSERT trigger.
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
        // One transaction for the whole pack: without it each INSERT is its own
        // autocommit (an fsync + FTS-trigger firing per row) and a mid-pack error
        // would leave a half-merged wing. Prepare the two INSERTs once, not per
        // object.
        let tx = conn.unchecked_transaction()?;
        {
            let mut mem_stmt = tx.prepare(
                "INSERT OR IGNORE INTO memories
                    (id, key, content, visibility, created_at, content_hash,
                     source_brain_id, wing, hall, signal_score, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1.0)",
            )?;
            let mut man_stmt = tx.prepare(
                "INSERT OR IGNORE INTO shared_wing_members
                    (wing_id, object_hash, mem_key, orig_key, supersedes)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for obj in &pack.objects {
                let oh = obj.object_hash();
                // Object-scoped, injective local key: a prefix of the
                // (content-addressed, author-bound) object hash makes two DISTINCT
                // objects never collide on the UNIQUE `memories.key`. Without it a
                // same-author update or a 4-byte `author_short` collision maps two
                // different-content objects to one key, and `INSERT OR IGNORE`
                // silently drops the second while its manifest entry survives —
                // a lost version + a phantom hash that diverges across brains.
                // The key is deliberately NOT wing-scoped: the same object shared
                // into several wings must dedup to ONE local row (by `id = oh`),
                // so every wing's manifest entry resolves to it.
                let local_key = format!(
                    "{}::{}::{}",
                    author_short(&obj.author_id),
                    obj.key,
                    &oh[..16]
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
                let n = mem_stmt.execute(rusqlite::params![
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
                ])?;
                merged += n;
                // Persist the wire object's ORIGINAL identity so it round-trips on
                // re-export (`orig_key`/`supersedes`) even though it is stored
                // under the synthetic `local_key`.
                man_stmt.execute(rusqlite::params![
                    pack.wing_id,
                    oh,
                    local_key,
                    obj.key,
                    obj.supersedes,
                ])?;
            }
        }
        tx.commit()?;
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
    // Hard-delete the local copy (id = hash) ONLY if no OTHER wing still shares
    // this object. The same content-addressed object can be a member of several
    // wings; a wing-scoped retraction must not destroy the copy the other wings
    // still serve. The `wing_id <> ?2` guard ignores this wing's own manifest row
    // (removed just below), so a single-wing object is still deleted.
    conn.execute(
        "DELETE FROM memories WHERE id = ?1
           AND NOT EXISTS (
             SELECT 1 FROM shared_wing_members
             WHERE object_hash = ?1 AND wing_id <> ?2
           )",
        rusqlite::params![target_hash, wing_id],
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

/// Batched provenance for a set of memory keys — the recall hot path. Resolves
/// every key in one set-based query instead of the per-key N+1 (`provenance` in a
/// loop): one `ensure_sync_tables`, one `LEFT JOIN` per chunk. Keys absent from
/// any shared wing map to [`Origin::Private`]. A key in several wings resolves to
/// one of them (stable within a call).
pub fn provenance_batch(
    store: &SqliteStore,
    mem_keys: &[&str],
) -> Result<std::collections::HashMap<String, Origin>> {
    use std::collections::HashMap;
    let mut out: HashMap<String, Origin> = mem_keys
        .iter()
        .map(|k| (k.to_string(), Origin::Private))
        .collect();
    if mem_keys.is_empty() {
        return Ok(out);
    }
    ensure_sync_tables(store)?;
    let conn = store.conn();
    // Chunk under SQLite's bound-variable ceiling (~999).
    for chunk in mem_keys.chunks(400) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT s.mem_key, s.wing_id, m.source_brain_id
             FROM shared_wing_members s
             LEFT JOIN memories m ON m.key = s.mem_key
             WHERE s.mem_key IN ({placeholders})
             ORDER BY s.wing_id",
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|k| k as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |r| {
            let mem_key: String = r.get(0)?;
            let wing_id: String = r.get(1)?;
            let author: Option<Vec<u8>> = r.get(2)?;
            Ok((mem_key, wing_id, author))
        })?;
        for row in rows {
            let (mem_key, wing_id, author) = row?;
            out.insert(
                mem_key,
                Origin::Shared {
                    wing_id,
                    author_id: author.and_then(|v| v.try_into().ok()),
                },
            );
        }
    }
    Ok(out)
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

    /// `provenance_batch` resolves a mixed set in one query and agrees with the
    /// per-key `provenance` on every key, including keys absent from any wing.
    #[tokio::test]
    async fn provenance_batch_matches_per_key_and_defaults_missing_to_private() {
        let store = SqliteStore::open_in_memory().unwrap();
        insert_local(&store, "mine", "my own private note", None);
        insert_local(&store, "ours", "a shared team note", Some([7u8; 32]));
        share(&store, "ours", "team").unwrap();

        // empty input is a no-op, not an error
        assert!(provenance_batch(&store, &[]).unwrap().is_empty());

        let keys = ["mine", "ours", "never_seen"];
        let batch = provenance_batch(&store, &keys).unwrap();
        assert_eq!(batch.len(), 3);
        for key in keys {
            assert_eq!(
                batch.get(key).cloned().unwrap_or(Origin::Private),
                provenance(&store, key).unwrap(),
                "batch disagrees with per-key provenance for {key}"
            );
        }
        // a key never inserted anywhere still resolves to Private
        assert_eq!(batch["never_seen"], Origin::Private);
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
        // Imported rows are stored under an object-scoped local key
        // `{author_short}::{orig_key}::{hash}` (deliberately not wing-prefixed).
        let (wing, hall, signal): (Option<String>, Option<String>, f64) = b
            .conn()
            .query_row(
                "SELECT wing, hall, signal_score FROM memories WHERE key LIKE '%::k1::%'",
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

    /// C1: two DIFFERENT authors whose 4-byte `author_short` collides, same key,
    /// same wing. The pre-fix local key `{wing}::{author_short}::{key}` was
    /// identical for both, so `INSERT OR IGNORE` on the UNIQUE `memories.key`
    /// silently dropped the second author's memory while its manifest entry
    /// survived (phantom hash / lost content / cross-brain divergence). The
    /// object-hash suffix keeps them distinct, restoring the cross-author
    /// accumulation guarantee even under a short-id collision.
    #[tokio::test]
    async fn cross_author_short_id_collision_still_accumulates() {
        let mut a1 = [0u8; 32];
        a1[..4].copy_from_slice(&[0xAB, 0xCD, 0xEF, 0x01]);
        a1[10] = 1;
        let mut a2 = [0u8; 32];
        a2[..4].copy_from_slice(&[0xAB, 0xCD, 0xEF, 0x01]);
        a2[10] = 2;
        assert_eq!(
            &a1[..4],
            &a2[..4],
            "authors must share their short-id prefix"
        );
        assert_ne!(a1, a2, "but be distinct principals");

        let o1 = MemoryObject {
            key: "policy".into(),
            content: "A says ship on green".into(),
            author_id: Some(a1),
            created_at: "2026/01/01 (Thu) 10:00".into(),
            visibility: "team".into(),
            supersedes: None,
        };
        let o2 = MemoryObject {
            key: "policy".into(),
            content: "B says ship on friday".into(),
            author_id: Some(a2),
            created_at: "2026/01/01 (Thu) 10:00".into(),
            visibility: "team".into(),
            supersedes: None,
        };
        let pack = Pack {
            wing_id: "team".into(),
            objects: vec![o1, o2],
            tombstones: vec![],
        };

        let m = SqliteStore::open_in_memory().unwrap();
        assert_eq!(
            import_pack(&m, &pack).unwrap(),
            2,
            "both authors' memories must be stored, not silently dropped"
        );
        assert_eq!(enumerate(&m, "team").unwrap().len(), 2);
        let hits = m.fts_search(&["ship".into()], 10).await.unwrap();
        assert!(hits.iter().any(|h| h.content.contains("green")));
        assert!(hits.iter().any(|h| h.content.contains("friday")));
    }

    /// C2: an imported object must survive re-export so it can be RELAYED onward
    /// (A -> B -> C). The re-exported object must round-trip its `object_hash` —
    /// which requires reconstructing the wire object from the stored ORIGINAL
    /// identity (`orig_key`/`supersedes`), not from the synthetic local key.
    #[tokio::test]
    async fn imported_objects_relay_and_round_trip_their_hash() {
        let a = SqliteStore::open_in_memory().unwrap();
        insert_local(&a, "deploy", "deploy needs two approvals", Some([3u8; 32]));
        share(&a, "deploy", "team").unwrap();
        let pa = export_pack(&a, "team").unwrap();
        let a_hash = pa.objects[0].object_hash();

        // B holds the object ONLY by import, then re-exports it.
        let b = SqliteStore::open_in_memory().unwrap();
        import_pack(&b, &pa).unwrap();
        let pb = export_pack(&b, "team").unwrap();
        assert_eq!(
            pb.objects.len(),
            1,
            "imported object was dropped from re-export (relay broken)"
        );
        assert_eq!(
            pb.objects[0].object_hash(),
            a_hash,
            "re-exported object must round-trip its content hash"
        );
        assert_eq!(
            pb.objects[0].key, "deploy",
            "the original wire key must be restored, not the local storage key"
        );

        // C imports from B (the relay) and converges on A's object set.
        let c = SqliteStore::open_in_memory().unwrap();
        import_pack(&c, &pb).unwrap();
        assert_eq!(
            enumerate(&c, "team").unwrap(),
            enumerate(&a, "team").unwrap(),
            "A and C (relayed through B) must converge on the same object set"
        );
        let hits = c.fts_search(&["deploy".into()], 10).await.unwrap();
        assert!(hits.iter().any(|h| h.content.contains("two approvals")));
    }

    /// C3: the same object shared into two wings, tombstoned from one. The
    /// wing-scoped retraction must NOT destroy the local copy the OTHER wing
    /// still serves (the old global `DELETE FROM memories WHERE id = hash` did).
    #[tokio::test]
    async fn tombstone_is_scoped_and_spares_other_wings() {
        let src = SqliteStore::open_in_memory().unwrap();
        insert_local(&src, "policy", "rotate creds quarterly", Some([5u8; 32]));
        let h = share(&src, "policy", "wing-a").unwrap();
        share(&src, "policy", "wing-b").unwrap(); // SAME object, second wing
        let pack_a = export_pack(&src, "wing-a").unwrap();
        let pack_b = export_pack(&src, "wing-b").unwrap();

        // m holds the object (one local row, id = hash) as a member of both wings.
        let m = SqliteStore::open_in_memory().unwrap();
        import_pack(&m, &pack_a).unwrap();
        import_pack(&m, &pack_b).unwrap();

        tombstone(&m, "wing-a", &h).unwrap();

        assert!(
            enumerate(&m, "wing-a").unwrap().is_empty(),
            "wing-a must no longer serve the retracted object"
        );
        assert_eq!(
            enumerate(&m, "wing-b").unwrap(),
            vec![h.clone()],
            "wing-b must still serve the object"
        );
        let pack_b2 = export_pack(&m, "wing-b").unwrap();
        assert_eq!(
            pack_b2.objects.len(),
            1,
            "wing-a's tombstone destroyed the copy wing-b still shares"
        );
        assert!(pack_b2.objects[0].content.contains("rotate creds"));
    }
}
