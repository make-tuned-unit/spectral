//! SQLite-backed entity/knowledge graph store.
//!
//! Replaces the former Kuzu-backed `KuzuStore`. Kuzu was archived upstream
//! (continued as LadybugDB) and, measured on real LongMemEval, its graph
//! retrieval path was *inferior* to cascade (see
//! docs/internal/graph-vs-cascade-retrieval-2026-07-14.md). The graph surface
//! is small — entity/triple/mention writes plus a 2-hop `neighborhood()` read,
//! all off the default recall path — so it collapses cleanly onto the SQLite
//! store the Brain already runs, dropping a second embedded engine, its mmap
//! contention, and its abort bug.
//!
//! The public API is preserved verbatim so callers are unaffected.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use spectral_core::entity_id::EntityId;
use spectral_core::identity::BrainId;
use spectral_core::visibility::Visibility;

use crate::Error;

/// A node in the knowledge graph.
#[derive(Debug, Clone)]
pub struct Entity {
    /// Content-addressed entity identifier.
    pub id: EntityId,
    /// Entity type (e.g. "person", "project").
    pub entity_type: String,
    /// Canonical name.
    pub canonical: String,
    /// Visibility level.
    pub visibility: Visibility,
    /// When the entity was first created.
    pub created_at: DateTime<Utc>,
    /// When the entity was last updated.
    pub updated_at: DateTime<Utc>,
    /// Importance weight (default 1.0).
    pub weight: f64,
    /// Optional human-readable description (set by Librarian).
    pub description: Option<String>,
}

/// A directed edge in the knowledge graph.
#[derive(Debug, Clone)]
pub struct Triple {
    /// Source entity.
    pub from: EntityId,
    /// Target entity.
    pub to: EntityId,
    /// Predicate name (e.g. "works_on").
    pub predicate: String,
    /// Confidence score in [0, 1].
    pub confidence: f64,
    /// Blake3 hash of the source document, if any.
    pub source_doc_id: Option<[u8; 32]>,
    /// Brain that asserted this triple.
    pub source_brain_id: BrainId,
    /// When the assertion was made.
    pub asserted_at: DateTime<Utc>,
    /// Visibility level.
    pub visibility: Visibility,
    /// Importance weight (default 1.0).
    pub weight: f64,
}

/// A Document node surfaced during neighborhood traversal.
#[derive(Debug, Clone)]
pub struct DocumentNode {
    /// Blake3 content hash (primary key).
    pub id: [u8; 32],
    /// Source identifier (filename, URI, etc.).
    pub source: String,
    /// When the document was ingested.
    pub ingested_at: DateTime<Utc>,
    /// Visibility level.
    pub visibility: Visibility,
}

/// Result of a neighborhood BFS traversal.
#[derive(Debug)]
pub struct Neighborhood {
    /// All visited entities (including the start).
    pub entities: Vec<Entity>,
    /// All triples connecting visited entities (deduplicated).
    pub triples: Vec<Triple>,
    /// Documents that mention entities in the neighborhood (terminal — not further expanded).
    pub documents: Vec<DocumentNode>,
}

/// SQLite-backed knowledge-graph store.
pub struct GraphStore {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for GraphStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphStore").finish_non_exhaustive()
    }
}

impl GraphStore {
    /// Open or create a graph database at the given path. Runs schema creation.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        create_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an existing graph database read-only. No DDL runs; writes fail at
    /// the engine level. Fails if the database does not exist.
    pub fn open_read_only(path: &Path) -> Result<Self, Error> {
        if !path.exists() {
            return Err(Error::Schema(format!(
                "read-only open requires an existing graph database: {} not found",
                path.display()
            )));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory graph database (useful for tests).
    pub fn in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Schema("graph store mutex poisoned".into()))
    }

    /// Insert or update an entity. Idempotent on EntityId. Preserves an existing
    /// description (that is owned by `set_entity_description`).
    pub fn upsert_entity(&self, entity: &Entity) -> Result<(), Error> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO entity
                 (id, entity_type, canonical, visibility, created_at, updated_at, weight)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 entity_type = excluded.entity_type,
                 canonical   = excluded.canonical,
                 visibility  = excluded.visibility,
                 created_at  = excluded.created_at,
                 updated_at  = excluded.updated_at,
                 weight      = excluded.weight",
            params![
                entity.id.as_bytes().to_vec(),
                entity.entity_type,
                entity.canonical,
                visibility_to_str(entity.visibility),
                entity.created_at.to_rfc3339(),
                entity.updated_at.to_rfc3339(),
                entity.weight,
            ],
        )?;
        Ok(())
    }

    /// Look up an entity by its EntityId.
    pub fn get_entity(&self, id: &EntityId) -> Result<Option<Entity>, Error> {
        let conn = self.lock()?;
        get_entity_conn(&conn, id)
    }

    /// Set the description on an entity. Idempotent. If the entity does not yet
    /// exist, a stub row is created (matching the prior MERGE semantics).
    pub fn set_entity_description(&self, id: &EntityId, description: &str) -> Result<(), Error> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO entity
                 (id, entity_type, canonical, visibility, created_at, updated_at, weight, description)
             VALUES (?1, '', '', 'private', ?2, ?2, 1.0, ?3)
             ON CONFLICT(id) DO UPDATE SET description = excluded.description",
            params![id.as_bytes().to_vec(), now, description],
        )?;
        Ok(())
    }

    /// Insert a triple between two existing entities. Matches the prior
    /// semantics: the edge is created only if both endpoints exist, and
    /// duplicate edges are permitted (dedup happens on read).
    pub fn insert_triple(&self, triple: &Triple) -> Result<(), Error> {
        let conn = self.lock()?;
        let doc_id: Option<Vec<u8>> = triple.source_doc_id.map(|b| b.to_vec());
        conn.execute(
            "INSERT INTO triple
                 (from_id, to_id, predicate, confidence, source_doc_id,
                  source_brain_id, asserted_at, visibility, weight)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
             WHERE EXISTS (SELECT 1 FROM entity WHERE id = ?1)
               AND EXISTS (SELECT 1 FROM entity WHERE id = ?2)",
            params![
                triple.from.as_bytes().to_vec(),
                triple.to.as_bytes().to_vec(),
                triple.predicate,
                triple.confidence,
                doc_id,
                triple.source_brain_id.as_bytes().to_vec(),
                triple.asserted_at.to_rfc3339(),
                visibility_to_str(triple.visibility),
                triple.weight,
            ],
        )?;
        Ok(())
    }

    /// Find triples matching a pattern. `None` values are wildcards.
    pub fn find_triples(
        &self,
        from: Option<&EntityId>,
        to: Option<&EntityId>,
        predicate: Option<&str>,
    ) -> Result<Vec<Triple>, Error> {
        let conn = self.lock()?;
        let mut conditions = Vec::new();
        let mut vals: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(f) = from {
            conditions.push(format!("from_id = ?{}", vals.len() + 1));
            vals.push(f.as_bytes().to_vec().into());
        }
        if let Some(t) = to {
            conditions.push(format!("to_id = ?{}", vals.len() + 1));
            vals.push(t.as_bytes().to_vec().into());
        }
        if let Some(p) = predicate {
            conditions.push(format!("predicate = ?{}", vals.len() + 1));
            vals.push(p.to_string().into());
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let sql = format!(
            "SELECT from_id, to_id, predicate, confidence, source_doc_id,
                    source_brain_id, asserted_at, visibility, weight
             FROM triple{where_clause}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(vals), |r| Ok(triple_from_row(r)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// BFS up to `max_hops` from a starting entity. Returns visited entities and
    /// the triples that connect them, plus documents mentioning any visited
    /// entity (terminal, capped).
    pub fn neighborhood(&self, start: &EntityId, max_hops: u32) -> Result<Neighborhood, Error> {
        let conn = self.lock()?;
        let mut visited = HashSet::new();
        let mut seen_edges: HashSet<(EntityId, EntityId, String)> = HashSet::new();
        let mut all_entities = Vec::new();
        let mut all_triples = Vec::new();

        if let Some(e) = get_entity_conn(&conn, start)? {
            all_entities.push(e);
        }
        visited.insert(*start);
        let mut frontier = vec![*start];

        for _ in 0..max_hops {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier = Vec::new();
            for id in &frontier {
                for triple in find_triples_directed(&conn, id, true)? {
                    if visited.insert(triple.to) {
                        next_frontier.push(triple.to);
                        if let Some(e) = get_entity_conn(&conn, &triple.to)? {
                            all_entities.push(e);
                        }
                    }
                    let key = (triple.from, triple.to, triple.predicate.clone());
                    if seen_edges.insert(key) {
                        all_triples.push(triple);
                    }
                }
                for triple in find_triples_directed(&conn, id, false)? {
                    if visited.insert(triple.from) {
                        next_frontier.push(triple.from);
                        if let Some(e) = get_entity_conn(&conn, &triple.from)? {
                            all_entities.push(e);
                        }
                    }
                    let key = (triple.from, triple.to, triple.predicate.clone());
                    if seen_edges.insert(key) {
                        all_triples.push(triple);
                    }
                }
            }
            frontier = next_frontier;
        }

        // Documents mentioning any visited entity (terminal — not expanded).
        const MAX_DOCUMENTS: usize = 100;
        let mut seen_docs: HashSet<[u8; 32]> = HashSet::new();
        let mut all_documents = Vec::new();
        'doc_scan: for entity_id in &visited {
            for doc in find_mentioning_documents(&conn, entity_id)? {
                if seen_docs.insert(doc.id) {
                    all_documents.push(doc);
                    if all_documents.len() >= MAX_DOCUMENTS {
                        break 'doc_scan;
                    }
                }
            }
        }

        Ok(Neighborhood {
            entities: all_entities,
            triples: all_triples,
            documents: all_documents,
        })
    }

    /// Upsert a document node.
    pub fn upsert_document(
        &self,
        id: &[u8; 32],
        source: &str,
        visibility: Visibility,
    ) -> Result<(), Error> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO document (id, source, ingested_at, visibility)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 source = excluded.source,
                 ingested_at = excluded.ingested_at,
                 visibility = excluded.visibility",
            params![
                id.to_vec(),
                source,
                Utc::now().to_rfc3339(),
                visibility_to_str(visibility),
            ],
        )?;
        Ok(())
    }

    /// Insert a Mentions edge from a document to an entity. Created only if both
    /// endpoints exist; idempotent on the (doc, entity) pair.
    pub fn insert_mention(
        &self,
        doc_id: &[u8; 32],
        entity_id: &EntityId,
        span_start: i64,
        span_end: i64,
    ) -> Result<(), Error> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO mention (doc_id, entity_id, span_start, span_end, extracted_at)
             SELECT ?1, ?2, ?3, ?4, ?5
             WHERE EXISTS (SELECT 1 FROM document WHERE id = ?1)
               AND EXISTS (SELECT 1 FROM entity WHERE id = ?2)
             ON CONFLICT(doc_id, entity_id) DO UPDATE SET
                 span_start = excluded.span_start,
                 span_end = excluded.span_end,
                 extracted_at = excluded.extracted_at",
            params![
                doc_id.to_vec(),
                entity_id.as_bytes().to_vec(),
                span_start,
                span_end,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Count Mentions edges from a document to an entity (0 or 1, since the pair
    /// is unique).
    pub fn count_mentions(&self, doc_id: &[u8; 32], entity_id: &EntityId) -> Result<usize, Error> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM mention WHERE doc_id = ?1 AND entity_id = ?2",
            params![doc_id.to_vec(), entity_id.as_bytes().to_vec()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

// --- Schema ---

fn create_schema(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entity (
             id           BLOB PRIMARY KEY,
             entity_type  TEXT NOT NULL,
             canonical    TEXT NOT NULL,
             visibility   TEXT NOT NULL,
             created_at   TEXT NOT NULL,
             updated_at   TEXT NOT NULL,
             weight       REAL NOT NULL DEFAULT 1.0,
             description  TEXT
         );
         CREATE TABLE IF NOT EXISTS document (
             id           BLOB PRIMARY KEY,
             source       TEXT NOT NULL,
             ingested_at  TEXT NOT NULL,
             visibility   TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS triple (
             from_id         BLOB NOT NULL,
             to_id           BLOB NOT NULL,
             predicate       TEXT NOT NULL,
             confidence      REAL NOT NULL,
             source_doc_id   BLOB,
             source_brain_id BLOB NOT NULL,
             asserted_at     TEXT NOT NULL,
             visibility      TEXT NOT NULL,
             weight          REAL NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_triple_from ON triple(from_id);
         CREATE INDEX IF NOT EXISTS idx_triple_to   ON triple(to_id);
         CREATE TABLE IF NOT EXISTS mention (
             doc_id       BLOB NOT NULL,
             entity_id    BLOB NOT NULL,
             span_start   INTEGER NOT NULL,
             span_end     INTEGER NOT NULL,
             extracted_at TEXT NOT NULL,
             PRIMARY KEY (doc_id, entity_id)
         );
         CREATE INDEX IF NOT EXISTS idx_mention_entity ON mention(entity_id);",
    )?;
    Ok(())
}

// --- Private query helpers (operate on a locked connection) ---

fn get_entity_conn(conn: &Connection, id: &EntityId) -> Result<Option<Entity>, Error> {
    conn.query_row(
        "SELECT id, entity_type, canonical, visibility, created_at, updated_at, weight, description
         FROM entity WHERE id = ?1",
        params![id.as_bytes().to_vec()],
        |r| Ok(entity_from_row(r)),
    )
    .optional()?
    .transpose()
}

fn find_triples_directed(
    conn: &Connection,
    id: &EntityId,
    outgoing: bool,
) -> Result<Vec<Triple>, Error> {
    let sql = if outgoing {
        "SELECT from_id, to_id, predicate, confidence, source_doc_id,
                source_brain_id, asserted_at, visibility, weight
         FROM triple WHERE from_id = ?1"
    } else {
        "SELECT from_id, to_id, predicate, confidence, source_doc_id,
                source_brain_id, asserted_at, visibility, weight
         FROM triple WHERE to_id = ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![id.as_bytes().to_vec()], |r| Ok(triple_from_row(r)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

fn find_mentioning_documents(
    conn: &Connection,
    entity_id: &EntityId,
) -> Result<Vec<DocumentNode>, Error> {
    let mut stmt = conn.prepare(
        "SELECT d.id, d.source, d.ingested_at, d.visibility
         FROM document d
         JOIN mention m ON m.doc_id = d.id
         WHERE m.entity_id = ?1",
    )?;
    let rows = stmt.query_map(params![entity_id.as_bytes().to_vec()], |r| {
        Ok(document_from_row(r))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

// --- Row parsing (each returns Result<T, Error> wrapped so rusqlite closures stay infallible) ---

fn entity_from_row(r: &rusqlite::Row<'_>) -> Result<Entity, Error> {
    let id_blob: Vec<u8> = r.get(0)?;
    let description: Option<String> = r.get(7)?;
    Ok(Entity {
        id: entity_id_from_blob(&id_blob)?,
        entity_type: r.get(1)?,
        canonical: r.get(2)?,
        visibility: str_to_visibility(&r.get::<_, String>(3)?)?,
        created_at: parse_dt(&r.get::<_, String>(4)?)?,
        updated_at: parse_dt(&r.get::<_, String>(5)?)?,
        weight: r.get(6)?,
        description: description.filter(|s| !s.is_empty()),
    })
}

fn triple_from_row(r: &rusqlite::Row<'_>) -> Result<Triple, Error> {
    let from_blob: Vec<u8> = r.get(0)?;
    let to_blob: Vec<u8> = r.get(1)?;
    let doc_blob: Option<Vec<u8>> = r.get(4)?;
    let brain_blob: Vec<u8> = r.get(5)?;
    Ok(Triple {
        from: entity_id_from_blob(&from_blob)?,
        to: entity_id_from_blob(&to_blob)?,
        predicate: r.get(2)?,
        confidence: r.get(3)?,
        source_doc_id: doc_blob.map(|v| blob32(&v)).transpose()?,
        source_brain_id: BrainId::from_bytes(blob32(&brain_blob)?),
        asserted_at: parse_dt(&r.get::<_, String>(6)?)?,
        visibility: str_to_visibility(&r.get::<_, String>(7)?)?,
        weight: r.get(8)?,
    })
}

fn document_from_row(r: &rusqlite::Row<'_>) -> Result<DocumentNode, Error> {
    let id_blob: Vec<u8> = r.get(0)?;
    Ok(DocumentNode {
        id: blob32(&id_blob)?,
        source: r.get(1)?,
        ingested_at: parse_dt(&r.get::<_, String>(2)?)?,
        visibility: str_to_visibility(&r.get::<_, String>(3)?)?,
    })
}

// --- Conversions ---

fn visibility_to_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Team => "team",
        Visibility::Org => "org",
        Visibility::Public => "public",
    }
}

fn str_to_visibility(s: &str) -> Result<Visibility, Error> {
    match s {
        "private" => Ok(Visibility::Private),
        "team" => Ok(Visibility::Team),
        "org" => Ok(Visibility::Org),
        "public" => Ok(Visibility::Public),
        _ => Err(Error::Schema(format!("invalid visibility: {s}"))),
    }
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>, Error> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::Schema(format!("invalid timestamp '{s}': {e}")))
}

fn blob32(v: &[u8]) -> Result<[u8; 32], Error> {
    v.try_into()
        .map_err(|_| Error::Schema(format!("expected 32-byte blob, got {} bytes", v.len())))
}

fn entity_id_from_blob(v: &[u8]) -> Result<EntityId, Error> {
    let bytes = blob32(v)?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    hex.parse().map_err(Error::Core)
}
