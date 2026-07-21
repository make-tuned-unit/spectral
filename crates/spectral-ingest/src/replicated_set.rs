//! Generic replicated set — the content-addressed have/want + relay primitive,
//! decoupled from memories.
//!
//! [`federation_sync`](crate::federation_sync) replicates *memory objects* between
//! brains: content-address, grow-only OR-Set union, tombstones, have/want
//! negotiation, and relay. This module is that same machinery over **opaque
//! blobs** — for callers that need to replicate objects Spectral does not model as
//! memories (Permagent's realm control-plane: `genesis` / `admin_chain_link` /
//! `realm_keyring`, keyed by `realm_id`).
//!
//! Design (mirrors the sovereignty/convergence properties of `federation_sync`,
//! see `docs/internal/federation-sync-design.md`):
//! - **Opaque + caller-addressed.** An object is `(object_hash, blob)`. Spectral
//!   never parses the blob and never recomputes the hash — the caller owns
//!   content-addressing (e.g. Permagent hashes its signed control objects). Use
//!   [`blake3_address`] if you just want a default addressing scheme.
//! - **Round-trip by construction.** The blob is stored and returned *verbatim*,
//!   so an imported object re-exports byte-identically and relays onward with no
//!   reconstruction step. This is why the memory layer needed the #207 identity
//!   round-trip fix and this layer does not: nothing is decomposed.
//! - **Namespaced + isolated.** Every object lives under a `namespace` (the
//!   realm/set id). The same `object_hash` in two namespaces is two independent
//!   rows with their own blob copies, so a tombstone in one namespace can never
//!   affect another — the cross-wing bleed class (#207 C3) is structurally absent.
//! - **Grow-only OR-Set + tombstones.** Union converges automatically; a tombstone
//!   dominates a later re-import (no resurrection).
//!
//! What stays the caller's (Permagent): identity, signature verification of blobs,
//! encryption of exported packs, and transport. This layer is crypto-agnostic —
//! it stores and moves bytes; it authenticates nothing.

use crate::sqlite_store::SqliteStore;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use crate::federation_sync::missing_locally;

/// An opaque, content-addressed object in a replicated set. `blob` is stored and
/// returned verbatim; `object_hash` is the caller's content address.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetObject {
    pub object_hash: String,
    pub blob: Vec<u8>,
}

/// A retraction, replicated like any object (OR-Set remove).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetTombstone {
    pub target_hash: String,
    pub ts: String,
}

/// A pack for one namespace — the payload the caller encrypts and ships.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetPack {
    pub namespace: String,
    pub objects: Vec<SetObject>,
    pub tombstones: Vec<SetTombstone>,
}

/// Optional default content-addressing helper: `blake3(blob)` as hex. Callers with
/// their own hashing (signed control objects) should pass their own hash instead —
/// Spectral treats `object_hash` as opaque either way.
pub fn blake3_address(blob: &[u8]) -> String {
    blake3::hash(blob).to_hex().to_string()
}

/// Create the replicated-set tables (idempotent). Members hold the opaque blobs;
/// tombstones are the OR-Set removals. Both are keyed by `(namespace, hash)` so
/// namespaces are fully isolated.
pub fn ensure_set_tables(store: &SqliteStore) -> Result<()> {
    let conn = store.conn();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS replicated_set_members (
            namespace   TEXT NOT NULL,
            object_hash TEXT NOT NULL,
            blob        BLOB NOT NULL,
            PRIMARY KEY (namespace, object_hash)
         );
         CREATE TABLE IF NOT EXISTS replicated_set_tombstones (
            namespace   TEXT NOT NULL,
            target_hash TEXT NOT NULL,
            ts          TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (namespace, target_hash)
         );",
    )?;
    Ok(())
}

/// Add an object to a namespace's set. Content-addressed and idempotent:
/// re-adding the same `(namespace, object_hash)` is a no-op. Returns `true` if the
/// object was newly inserted, `false` if it was already present.
pub fn put(store: &SqliteStore, namespace: &str, object_hash: &str, blob: &[u8]) -> Result<bool> {
    ensure_set_tables(store)?;
    let conn = store.conn();
    // A tombstone dominates: never re-add a retracted object. Without this the row
    // is resurrected at the storage layer — `enumerate` hides it, but `export_set`
    // re-ships it to peers, resurrecting a revoked object across the whole set.
    if is_tombstoned(&conn, namespace, object_hash)? {
        return Ok(false);
    }
    let n = conn.execute(
        "INSERT OR IGNORE INTO replicated_set_members (namespace, object_hash, blob)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![namespace, object_hash, blob],
    )?;
    Ok(n > 0)
}

/// Is `hash` retracted in `namespace`? A tombstone dominates a re-add (OR-Set
/// remove wins). Caller must already hold the connection lock (no re-lock).
fn is_tombstoned(conn: &rusqlite::Connection, namespace: &str, hash: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(1) FROM replicated_set_tombstones
         WHERE namespace = ?1 AND target_hash = ?2",
        rusqlite::params![namespace, hash],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// The object hashes currently live in a namespace (members minus tombstones),
/// sorted for deterministic have/want negotiation — the "have" advertisement.
pub fn enumerate(store: &SqliteStore, namespace: &str) -> Result<Vec<String>> {
    ensure_set_tables(store)?;
    let conn = store.conn();
    let mut stmt = conn.prepare(
        "SELECT object_hash FROM replicated_set_members
         WHERE namespace = ?1
           AND object_hash NOT IN
               (SELECT target_hash FROM replicated_set_tombstones WHERE namespace = ?1)
         ORDER BY object_hash",
    )?;
    let rows = stmt.query_map(rusqlite::params![namespace], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Export a namespace as a pack (blobs verbatim + tombstones) for the caller to
/// encrypt and ship. Round-trips by construction: the exported blob is the stored
/// blob, so a re-export elsewhere is byte-identical and relay (A→B→C) works.
pub fn export_set(store: &SqliteStore, namespace: &str) -> Result<SetPack> {
    ensure_set_tables(store)?;
    let conn = store.conn();
    let objects = {
        // Exclude tombstoned members: a retracted object must never ride out in a
        // pack (it would resurrect on the receiving peer). Mirrors `enumerate`.
        let mut stmt = conn.prepare(
            "SELECT object_hash, blob FROM replicated_set_members
             WHERE namespace = ?1
               AND object_hash NOT IN
                   (SELECT target_hash FROM replicated_set_tombstones WHERE namespace = ?1)",
        )?;
        let rows = stmt.query_map(rusqlite::params![namespace], |r| {
            Ok(SetObject {
                object_hash: r.get(0)?,
                blob: r.get(1)?,
            })
        })?;
        let objects: Vec<SetObject> = rows.filter_map(Result::ok).collect();
        objects
    };
    let tombstones = {
        let mut stmt = conn.prepare(
            "SELECT target_hash, ts FROM replicated_set_tombstones WHERE namespace = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![namespace], |r| {
            Ok(SetTombstone {
                target_hash: r.get(0)?,
                ts: r.get(1)?,
            })
        })?;
        let tombstones: Vec<SetTombstone> = rows.filter_map(Result::ok).collect();
        tombstones
    };
    Ok(SetPack {
        namespace: namespace.to_string(),
        objects,
        tombstones,
    })
}

/// Merge a pack into the local store (OR-Set union) under `pack.namespace`.
/// Idempotent per `(namespace, object_hash)`; blobs are stored verbatim.
/// Tombstones in the pack are applied (and dominate). One transaction for the whole
/// pack. Returns the number of newly-merged objects.
pub fn import_set(store: &SqliteStore, pack: &SetPack) -> Result<usize> {
    ensure_set_tables(store)?;
    // The set of hashes this pack retracts (its own tombstones) — an object and
    // its tombstone can arrive in the same pack, and the tombstone must win
    // regardless of order, so we skip these on the object insert below.
    let pack_retracted: std::collections::HashSet<&str> = pack
        .tombstones
        .iter()
        .map(|t| t.target_hash.as_str())
        .collect();
    let mut merged = 0usize;
    {
        let conn = store.conn();
        // One transaction for the whole pack — objects AND tombstones — so a
        // mid-pack failure leaves the set unchanged rather than half-merged with
        // tombstones half-applied.
        let tx = conn.unchecked_transaction()?;
        {
            let mut put_stmt = tx.prepare(
                "INSERT OR IGNORE INTO replicated_set_members (namespace, object_hash, blob)
                 VALUES (?1, ?2, ?3)",
            )?;
            for obj in &pack.objects {
                // A tombstone dominates: skip an object retracted in this pack or
                // already retracted locally. Otherwise re-import resurrects it.
                if pack_retracted.contains(obj.object_hash.as_str())
                    || is_tombstoned(&tx, &pack.namespace, &obj.object_hash)?
                {
                    continue;
                }
                merged += put_stmt.execute(rusqlite::params![
                    pack.namespace,
                    obj.object_hash,
                    obj.blob
                ])?;
            }
        }
        for t in &pack.tombstones {
            apply_tombstone_tx(&tx, &pack.namespace, &t.target_hash)?;
        }
        tx.commit()?;
    }
    Ok(merged)
}

/// Have/want: given a remote's advertised hashes for a namespace, the subset we
/// lack and should request. A thin convenience over [`missing_locally`].
///
/// Pure over the two lists — it does not know about local tombstones, so a peer
/// still advertising an object you have *retracted* is reported as wanted. Use
/// [`want_scoped`] to suppress those; re-importing a retracted object is harmless
/// (import refuses to resurrect it) but re-requesting it every round is wasted.
pub fn want(local: &[String], remote: &[String]) -> Vec<String> {
    missing_locally(local, remote)
}

/// Have/want that also subtracts objects you have locally tombstoned, so a peer
/// that has not yet seen your retraction does not make you re-request it forever.
pub fn want_scoped(store: &SqliteStore, namespace: &str, remote: &[String]) -> Result<Vec<String>> {
    ensure_set_tables(store)?;
    let have = enumerate(store, namespace)?;
    let conn = store.conn();
    let mut wanted = Vec::new();
    let have_set: std::collections::HashSet<&String> = have.iter().collect();
    for h in remote {
        if !have_set.contains(h) && !is_tombstoned(&conn, namespace, h)? {
            wanted.push(h.clone());
        }
    }
    Ok(wanted)
}

/// Retract an object from a namespace (OR-Set remove): record the tombstone and
/// drop the local blob. Namespace-scoped — a copy of the same object in another
/// namespace is untouched. The tombstone dominates, so a later re-import cannot
/// resurrect it.
pub fn tombstone_set(store: &SqliteStore, namespace: &str, target_hash: &str) -> Result<()> {
    ensure_set_tables(store)?;
    apply_tombstone(store, namespace, target_hash)
}

fn apply_tombstone(store: &SqliteStore, namespace: &str, target_hash: &str) -> Result<()> {
    let conn = store.conn();
    let tx = conn.unchecked_transaction()?;
    apply_tombstone_tx(&tx, namespace, target_hash)?;
    tx.commit()?;
    Ok(())
}

/// Record a retraction and drop the local blob, atomically within `tx`. Both
/// statements share the caller's transaction so a crash between them cannot leave
/// a tombstoned-but-still-stored (and thus still-exportable) object.
fn apply_tombstone_tx(tx: &rusqlite::Connection, namespace: &str, target_hash: &str) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO replicated_set_tombstones (namespace, target_hash)
         VALUES (?1, ?2)",
        rusqlite::params![namespace, target_hash],
    )?;
    // Namespace-scoped delete: the blob is keyed by (namespace, object_hash), so
    // this can never touch another namespace's copy of the same hash.
    tx.execute(
        "DELETE FROM replicated_set_members WHERE namespace = ?1 AND object_hash = ?2",
        rusqlite::params![namespace, target_hash],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(bytes: &[u8]) -> SetObject {
        SetObject {
            object_hash: blake3_address(bytes),
            blob: bytes.to_vec(),
        }
    }

    #[test]
    fn put_enumerate_and_want() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(put(&store, "realm:a", "h1", b"one").unwrap());
        assert!(put(&store, "realm:a", "h2", b"two").unwrap());
        // idempotent re-put
        assert!(!put(&store, "realm:a", "h1", b"one").unwrap());

        let have = enumerate(&store, "realm:a").unwrap();
        assert_eq!(have, vec!["h1".to_string(), "h2".to_string()]);

        // a peer that advertises h2 + h3 → we want only h3
        let remote = vec!["h2".to_string(), "h3".to_string()];
        assert_eq!(want(&have, &remote), vec!["h3".to_string()]);
    }

    #[test]
    fn export_import_round_trips_blobs_verbatim() {
        let a = SqliteStore::open_in_memory().unwrap();
        let o1 = obj(b"genesis-blob");
        let o2 = obj(b"admin-chain-link-blob");
        put(&a, "realm:x", &o1.object_hash, &o1.blob).unwrap();
        put(&a, "realm:x", &o2.object_hash, &o2.blob).unwrap();

        let pack = export_set(&a, "realm:x").unwrap();

        let b = SqliteStore::open_in_memory().unwrap();
        assert_eq!(import_set(&b, &pack).unwrap(), 2);

        // Re-export from B must be byte-identical to A's export (relay round-trip).
        let mut re = export_set(&b, "realm:x").unwrap();
        let mut orig = pack.clone();
        re.objects.sort_by(|x, y| x.object_hash.cmp(&y.object_hash));
        orig.objects
            .sort_by(|x, y| x.object_hash.cmp(&y.object_hash));
        assert_eq!(re.objects, orig.objects, "blobs must round-trip verbatim");
    }

    #[test]
    fn relay_across_three_stores_converges() {
        let a = SqliteStore::open_in_memory().unwrap();
        let o = obj(b"realm-keyring-epoch-1");
        put(&a, "realm:r", &o.object_hash, &o.blob).unwrap();
        let pa = export_set(&a, "realm:r").unwrap();

        // B holds it only by import, then relays to C.
        let b = SqliteStore::open_in_memory().unwrap();
        import_set(&b, &pa).unwrap();
        let pb = export_set(&b, "realm:r").unwrap();

        let c = SqliteStore::open_in_memory().unwrap();
        import_set(&c, &pb).unwrap();

        assert_eq!(
            enumerate(&c, "realm:r").unwrap(),
            enumerate(&a, "realm:r").unwrap(),
            "A and C (relayed through B) converge on the same set"
        );
        // idempotent re-import
        assert_eq!(import_set(&c, &pb).unwrap(), 0);
    }

    #[test]
    fn tombstone_is_namespace_scoped_and_blocks_resurrection() {
        let store = SqliteStore::open_in_memory().unwrap();
        let o = obj(b"shared-control-object");
        // same object hash lives in two realms as independent copies
        put(&store, "realm:a", &o.object_hash, &o.blob).unwrap();
        put(&store, "realm:b", &o.object_hash, &o.blob).unwrap();

        tombstone_set(&store, "realm:a", &o.object_hash).unwrap();

        assert!(
            enumerate(&store, "realm:a").unwrap().is_empty(),
            "realm:a retracted the object"
        );
        assert_eq!(
            enumerate(&store, "realm:b").unwrap(),
            vec![o.object_hash.clone()],
            "realm:b's copy must be untouched (no cross-namespace bleed)"
        );

        // Re-importing the object into realm:a must not resurrect it.
        let pack = SetPack {
            namespace: "realm:a".into(),
            objects: vec![o.clone()],
            tombstones: vec![],
        };
        import_set(&store, &pack).unwrap();
        assert!(
            enumerate(&store, "realm:a").unwrap().is_empty(),
            "a tombstoned object must not be resurrectable by re-import"
        );
        // Storage-level, not just enumerate-hidden: a resurrected row would ride
        // out in the export and re-infect peers. The export must be empty too.
        assert!(
            export_set(&store, "realm:a").unwrap().objects.is_empty(),
            "a tombstoned object must not reappear in an exported pack"
        );
    }

    #[test]
    fn put_after_tombstone_is_refused() {
        let store = SqliteStore::open_in_memory().unwrap();
        let o = obj(b"revoked-keyring-epoch");
        put(&store, "realm:k", &o.object_hash, &o.blob).unwrap();
        tombstone_set(&store, "realm:k", &o.object_hash).unwrap();

        // Re-publishing a retracted hash must be refused, not silently stored and
        // then leaked out via export.
        assert!(!put(&store, "realm:k", &o.object_hash, &o.blob).unwrap());
        assert!(export_set(&store, "realm:k").unwrap().objects.is_empty());
    }

    #[test]
    fn same_pack_object_and_tombstone_lets_tombstone_win() {
        let store = SqliteStore::open_in_memory().unwrap();
        let o = obj(b"racing-object");
        let pack = SetPack {
            namespace: "realm:z".into(),
            objects: vec![o.clone()],
            tombstones: vec![SetTombstone {
                target_hash: o.object_hash.clone(),
                ts: "2026-01-01 00:00:00".into(),
            }],
        };
        import_set(&store, &pack).unwrap();
        assert!(
            enumerate(&store, "realm:z").unwrap().is_empty(),
            "an object retracted in the same pack must not survive import"
        );
    }

    #[test]
    fn want_scoped_suppresses_locally_tombstoned() {
        let store = SqliteStore::open_in_memory().unwrap();
        let o = obj(b"retracted-here");
        put(&store, "realm:w", &o.object_hash, &o.blob).unwrap();
        tombstone_set(&store, "realm:w", &o.object_hash).unwrap();

        // A peer that has not seen our retraction still advertises the object.
        let remote = vec![o.object_hash.clone(), "brand-new".to_string()];
        // Pure `want` (over the have-list) would re-request the tombstoned hash...
        let have = enumerate(&store, "realm:w").unwrap();
        assert!(want(&have, &remote).contains(&o.object_hash));
        // ...but the scoped variant suppresses it, requesting only the genuinely new one.
        assert_eq!(
            want_scoped(&store, "realm:w", &remote).unwrap(),
            vec!["brand-new".to_string()]
        );
    }

    #[test]
    fn tombstones_propagate_via_export_import() {
        let a = SqliteStore::open_in_memory().unwrap();
        let keep = obj(b"keep");
        let drop = obj(b"drop");
        put(&a, "realm:t", &keep.object_hash, &keep.blob).unwrap();
        put(&a, "realm:t", &drop.object_hash, &drop.blob).unwrap();

        let b = SqliteStore::open_in_memory().unwrap();
        import_set(&b, &export_set(&a, "realm:t").unwrap()).unwrap();
        assert_eq!(enumerate(&b, "realm:t").unwrap().len(), 2);

        // Retract on A, propagate via a fresh pack.
        tombstone_set(&a, "realm:t", &drop.object_hash).unwrap();
        import_set(&b, &export_set(&a, "realm:t").unwrap()).unwrap();
        assert_eq!(
            enumerate(&b, "realm:t").unwrap(),
            vec![keep.object_hash.clone()],
            "the tombstone must propagate and remove the object on B"
        );
    }
}
