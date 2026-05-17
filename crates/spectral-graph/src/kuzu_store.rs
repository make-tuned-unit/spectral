//! Typed wrapper around the Kuzu graph database.
//!
//! Provides a high-level API for storing and querying entities and triples,
//! backed by an embedded Kuzu database.
//!
//! # Design note
//!
//! `KuzuStore` stores only the `Database` handle and creates fresh
//! `Connection` instances per operation. Kuzu's `Connection<'a>` borrows
//! the `Database`, so they cannot live in the same struct.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use kuzu::{Connection, Database, SystemConfig};

use spectral_core::entity_id::EntityId;
use spectral_core::identity::BrainId;
use spectral_core::visibility::Visibility;

use crate::schema::create_schema;
use crate::Error;

/// A node in the knowledge graph.
///
/// ```
/// use spectral_core::entity_id::entity_id;
/// use spectral_core::visibility::Visibility;
/// use spectral_graph::kuzu_store::Entity;
/// use chrono::Utc;
///
/// let entity = Entity {
///     id: entity_id("person", "alice"),
///     entity_type: "person".into(),
///     canonical: "alice".into(),
///     visibility: Visibility::Private,
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
///     weight: 1.0,
/// };
/// assert_eq!(entity.entity_type, "person");
/// ```
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

/// Typed wrapper around a Kuzu database.
///
/// # Open an in-memory store
///
/// ```
/// use spectral_graph::kuzu_store::KuzuStore;
/// let store = KuzuStore::in_memory().unwrap();
/// ```
///
/// # Upsert and retrieve an entity
///
/// ```
/// use spectral_core::entity_id::entity_id;
/// use spectral_core::visibility::Visibility;
/// use spectral_graph::kuzu_store::{KuzuStore, Entity};
/// use chrono::Utc;
///
/// let store = KuzuStore::in_memory().unwrap();
/// let id = entity_id("person", "alice");
/// store.upsert_entity(&Entity {
///     id, entity_type: "person".into(), canonical: "alice".into(),
///     visibility: Visibility::Private,
///     created_at: Utc::now(), updated_at: Utc::now(), weight: 1.0,
/// }).unwrap();
/// let found = store.get_entity(&id).unwrap();
/// assert!(found.is_some());
/// ```
///
/// # Missing entity returns None
///
/// ```
/// use spectral_core::entity_id::entity_id;
/// use spectral_graph::kuzu_store::KuzuStore;
///
/// let store = KuzuStore::in_memory().unwrap();
/// let id = entity_id("person", "nobody");
/// assert!(store.get_entity(&id).unwrap().is_none());
/// ```
///
/// # Insert and query triples
///
/// ```
/// use spectral_core::entity_id::entity_id;
/// use spectral_core::identity::BrainIdentity;
/// use spectral_core::visibility::Visibility;
/// use spectral_graph::kuzu_store::{KuzuStore, Entity, Triple};
/// use chrono::Utc;
///
/// let store = KuzuStore::in_memory().unwrap();
/// let a = entity_id("person", "alice");
/// let b = entity_id("project", "spectral");
/// let brain = BrainIdentity::generate();
/// let now = Utc::now();
///
/// for (id, ty, name) in [(a, "person", "alice"), (b, "project", "spectral")] {
///     store.upsert_entity(&Entity {
///         id, entity_type: ty.into(), canonical: name.into(),
///         visibility: Visibility::Private,
///         created_at: now, updated_at: now, weight: 1.0,
///     }).unwrap();
/// }
///
/// store.insert_triple(&Triple {
///     from: a, to: b, predicate: "works_on".into(), confidence: 0.9,
///     source_doc_id: None, source_brain_id: *brain.brain_id(),
///     asserted_at: now, visibility: Visibility::Private, weight: 1.0,
/// }).unwrap();
///
/// let found = store.find_triples(Some(&a), None, None).unwrap();
/// assert_eq!(found.len(), 1);
/// assert_eq!(found[0].predicate, "works_on");
/// ```
pub struct KuzuStore {
    db: Database,
}

impl std::fmt::Debug for KuzuStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KuzuStore").finish_non_exhaustive()
    }
}

impl KuzuStore {
    /// Open or create a Kuzu database at the given path.
    /// Runs schema creation on first open.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = Database::new(path, SystemConfig::default())?;
        {
            let conn = Connection::new(&db)?;
            create_schema(&conn)?;
        }
        Ok(Self { db })
    }

    /// Create an in-memory Kuzu database (useful for tests).
    pub fn in_memory() -> Result<Self, Error> {
        let db = Database::in_memory(SystemConfig::default())?;
        {
            let conn = Connection::new(&db)?;
            create_schema(&conn)?;
        }
        Ok(Self { db })
    }

    fn connection(&self) -> Result<Connection<'_>, Error> {
        Ok(Connection::new(&self.db)?)
    }

    /// Insert or update an entity. Idempotent on EntityId.
    pub fn upsert_entity(&self, entity: &Entity) -> Result<(), Error> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "MERGE (e:Entity {id: $id})
             SET e.entity_type = $type,
                 e.canonical = $canon,
                 e.visibility = $vis,
                 e.created_at = cast($created, 'TIMESTAMP'),
                 e.updated_at = cast($updated, 'TIMESTAMP'),
                 e.weight = $weight",
        )?;
        conn.execute(
            &mut stmt,
            vec![
                ("id", kuzu::Value::Blob(entity.id.as_bytes().to_vec())),
                ("type", kuzu::Value::String(entity.entity_type.clone())),
                ("canon", kuzu::Value::String(entity.canonical.clone())),
                (
                    "vis",
                    kuzu::Value::String(visibility_to_str(entity.visibility)),
                ),
                (
                    "created",
                    kuzu::Value::String(datetime_to_kuzu_str(&entity.created_at)),
                ),
                (
                    "updated",
                    kuzu::Value::String(datetime_to_kuzu_str(&entity.updated_at)),
                ),
                ("weight", kuzu::Value::Double(entity.weight)),
            ],
        )?;
        Ok(())
    }

    /// Look up an entity by its EntityId.
    pub fn get_entity(&self, id: &EntityId) -> Result<Option<Entity>, Error> {
        let conn = self.connection()?;
        self.get_entity_with_conn(&conn, id)
    }

    /// Insert a triple between two existing entities.
    pub fn insert_triple(&self, triple: &Triple) -> Result<(), Error> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "MATCH (a:Entity), (b:Entity)
             WHERE a.id = $from_id AND b.id = $to_id
             CREATE (a)-[:Triple {
                 predicate: $pred,
                 confidence: $conf,
                 source_doc_id: $doc_id,
                 source_brain_id: $brain_id,
                 asserted_at: cast($asserted, 'TIMESTAMP'),
                 visibility: $vis,
                 weight: $weight
             }]->(b)",
        )?;
        let doc_id_val = match triple.source_doc_id {
            Some(bytes) => kuzu::Value::Blob(bytes.to_vec()),
            None => kuzu::Value::Null(kuzu::LogicalType::Blob),
        };
        conn.execute(
            &mut stmt,
            vec![
                (
                    "from_id",
                    kuzu::Value::Blob(triple.from.as_bytes().to_vec()),
                ),
                ("to_id", kuzu::Value::Blob(triple.to.as_bytes().to_vec())),
                ("pred", kuzu::Value::String(triple.predicate.clone())),
                ("conf", kuzu::Value::Double(triple.confidence)),
                ("doc_id", doc_id_val),
                (
                    "brain_id",
                    kuzu::Value::Blob(triple.source_brain_id.as_bytes().to_vec()),
                ),
                (
                    "asserted",
                    kuzu::Value::String(datetime_to_kuzu_str(&triple.asserted_at)),
                ),
                (
                    "vis",
                    kuzu::Value::String(visibility_to_str(triple.visibility)),
                ),
                ("weight", kuzu::Value::Double(triple.weight)),
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
        let conn = self.connection()?;

        let mut conditions = Vec::new();
        let mut params: Vec<(&str, kuzu::Value)> = Vec::new();

        if let Some(f) = from {
            conditions.push("a.id = $from_id");
            params.push(("from_id", kuzu::Value::Blob(f.as_bytes().to_vec())));
        }
        if let Some(t) = to {
            conditions.push("b.id = $to_id");
            params.push(("to_id", kuzu::Value::Blob(t.as_bytes().to_vec())));
        }
        if let Some(p) = predicate {
            conditions.push("t.predicate = $predicate");
            params.push(("predicate", kuzu::Value::String(p.to_string())));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            "MATCH (a:Entity)-[t:Triple]->(b:Entity){} \
             RETURN a.id, b.id, t.predicate, t.confidence, \
                    t.source_doc_id, t.source_brain_id, t.asserted_at, \
                    t.visibility, t.weight",
            where_clause
        );

        let mut stmt = conn.prepare(&query)?;
        let result = conn.execute(&mut stmt, params)?;
        let mut triples = Vec::new();
        for row in result {
            triples.push(parse_triple_row(&row)?);
        }
        Ok(triples)
    }

    /// BFS up to `max_hops` from a starting entity.
    /// Returns visited entities and the triples that connect them.
    pub fn neighborhood(&self, start: &EntityId, max_hops: u32) -> Result<Neighborhood, Error> {
        let conn = self.connection()?;
        let mut visited = HashSet::new();
        let mut seen_edges: HashSet<(EntityId, EntityId, String)> = HashSet::new();
        let mut all_entities = Vec::new();
        let mut all_triples = Vec::new();

        if let Some(e) = self.get_entity_with_conn(&conn, start)? {
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
                // Outgoing triples
                for triple in self.find_triples_directed(&conn, id, true)? {
                    if visited.insert(triple.to) {
                        next_frontier.push(triple.to);
                        if let Some(e) = self.get_entity_with_conn(&conn, &triple.to)? {
                            all_entities.push(e);
                        }
                    }
                    let key = (triple.from, triple.to, triple.predicate.clone());
                    if seen_edges.insert(key) {
                        all_triples.push(triple);
                    }
                }

                // Incoming triples
                for triple in self.find_triples_directed(&conn, id, false)? {
                    if visited.insert(triple.from) {
                        next_frontier.push(triple.from);
                        if let Some(e) = self.get_entity_with_conn(&conn, &triple.from)? {
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

        // After BFS: find Documents mentioning any visited entity (terminal — not expanded)
        let mut seen_docs: HashSet<[u8; 32]> = HashSet::new();
        let mut all_documents = Vec::new();
        for entity_id in &visited {
            for doc in self.find_mentioning_documents(&conn, entity_id)? {
                if seen_docs.insert(doc.id) {
                    all_documents.push(doc);
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
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "MERGE (d:Document {id: $id})
             SET d.source = $source,
                 d.ingested_at = cast($ingested, 'TIMESTAMP'),
                 d.visibility = $vis",
        )?;
        conn.execute(
            &mut stmt,
            vec![
                ("id", kuzu::Value::Blob(id.to_vec())),
                ("source", kuzu::Value::String(source.to_string())),
                (
                    "ingested",
                    kuzu::Value::String(datetime_to_kuzu_str(&Utc::now())),
                ),
                ("vis", kuzu::Value::String(visibility_to_str(visibility))),
            ],
        )?;
        Ok(())
    }

    /// Insert a Mentions edge from a document to an entity. Idempotent on (doc, entity) pair.
    pub fn insert_mention(
        &self,
        doc_id: &[u8; 32],
        entity_id: &EntityId,
        span_start: i64,
        span_end: i64,
    ) -> Result<(), Error> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "MATCH (d:Document), (e:Entity)
             WHERE d.id = $doc_id AND e.id = $entity_id
             MERGE (d)-[m:Mentions]->(e)
             SET m.span_start = $span_start,
                 m.span_end = $span_end,
                 m.extracted_at = cast($extracted, 'TIMESTAMP')",
        )?;
        conn.execute(
            &mut stmt,
            vec![
                ("doc_id", kuzu::Value::Blob(doc_id.to_vec())),
                (
                    "entity_id",
                    kuzu::Value::Blob(entity_id.as_bytes().to_vec()),
                ),
                ("span_start", kuzu::Value::Int64(span_start)),
                ("span_end", kuzu::Value::Int64(span_end)),
                (
                    "extracted",
                    kuzu::Value::String(datetime_to_kuzu_str(&Utc::now())),
                ),
            ],
        )?;
        Ok(())
    }

    /// Count Mentions edges from a document to an entity.
    pub fn count_mentions(&self, doc_id: &[u8; 32], entity_id: &EntityId) -> Result<usize, Error> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "MATCH (d:Document)-[m:Mentions]->(e:Entity)
             WHERE d.id = $doc_id AND e.id = $entity_id
             RETURN count(m)",
        )?;
        let result = conn.execute(
            &mut stmt,
            vec![
                ("doc_id", kuzu::Value::Blob(doc_id.to_vec())),
                (
                    "entity_id",
                    kuzu::Value::Blob(entity_id.as_bytes().to_vec()),
                ),
            ],
        )?;
        for row in result {
            if let kuzu::Value::Int64(n) = row[0] {
                return Ok(n as usize);
            }
        }
        Ok(0)
    }

    // --- Private helpers ---

    fn find_mentioning_documents(
        &self,
        conn: &Connection<'_>,
        entity_id: &EntityId,
    ) -> Result<Vec<DocumentNode>, Error> {
        let mut stmt = conn.prepare(
            "MATCH (d:Document)-[:Mentions]->(e:Entity)
             WHERE e.id = $id
             RETURN d.id, d.source, d.ingested_at, d.visibility",
        )?;
        let result = conn.execute(
            &mut stmt,
            vec![("id", kuzu::Value::Blob(entity_id.as_bytes().to_vec()))],
        )?;
        let mut docs = Vec::new();
        for row in result {
            let id_blob = match &row[0] {
                kuzu::Value::Blob(v) => {
                    let bytes: [u8; 32] = v.as_slice().try_into().map_err(|_| {
                        Error::Schema(format!("expected 32-byte doc id, got {} bytes", v.len()))
                    })?;
                    bytes
                }
                _ => return Err(Error::Schema("expected blob for doc id".into())),
            };
            let source = match &row[1] {
                kuzu::Value::String(s) => s.clone(),
                _ => return Err(Error::Schema("expected string for source".into())),
            };
            let ingested_at = extract_timestamp(&row[2])?;
            let visibility = match &row[3] {
                kuzu::Value::String(s) => str_to_visibility(s)?,
                _ => return Err(Error::Schema("expected string for visibility".into())),
            };
            docs.push(DocumentNode {
                id: id_blob,
                source,
                ingested_at,
                visibility,
            });
        }
        Ok(docs)
    }

    fn get_entity_with_conn(
        &self,
        conn: &Connection<'_>,
        id: &EntityId,
    ) -> Result<Option<Entity>, Error> {
        let mut stmt = conn.prepare(
            "MATCH (e:Entity) WHERE e.id = $id
             RETURN e.id, e.entity_type, e.canonical, e.visibility,
                    e.created_at, e.updated_at, e.weight",
        )?;
        let mut result = conn.execute(
            &mut stmt,
            vec![("id", kuzu::Value::Blob(id.as_bytes().to_vec()))],
        )?;
        match result.next() {
            Some(row) => Ok(Some(parse_entity_row(&row)?)),
            None => Ok(None),
        }
    }

    fn find_triples_directed(
        &self,
        conn: &Connection<'_>,
        id: &EntityId,
        outgoing: bool,
    ) -> Result<Vec<Triple>, Error> {
        let query = if outgoing {
            "MATCH (a:Entity)-[t:Triple]->(b:Entity) WHERE a.id = $id \
             RETURN a.id, b.id, t.predicate, t.confidence, \
                    t.source_doc_id, t.source_brain_id, t.asserted_at, \
                    t.visibility, t.weight"
        } else {
            "MATCH (a:Entity)-[t:Triple]->(b:Entity) WHERE b.id = $id \
             RETURN a.id, b.id, t.predicate, t.confidence, \
                    t.source_doc_id, t.source_brain_id, t.asserted_at, \
                    t.visibility, t.weight"
        };
        let mut stmt = conn.prepare(query)?;
        let result = conn.execute(
            &mut stmt,
            vec![("id", kuzu::Value::Blob(id.as_bytes().to_vec()))],
        )?;
        let mut triples = Vec::new();
        for row in result {
            triples.push(parse_triple_row(&row)?);
        }
        Ok(triples)
    }
}

// --- Conversion helpers ---

fn visibility_to_str(v: Visibility) -> String {
    match v {
        Visibility::Private => "private",
        Visibility::Team => "team",
        Visibility::Org => "org",
        Visibility::Public => "public",
    }
    .to_string()
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

/// Format a chrono DateTime for Kuzu's `cast(str, 'TIMESTAMP')`.
fn datetime_to_kuzu_str(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
}

/// Extract a chrono DateTime from a Kuzu TIMESTAMP value.
fn extract_timestamp(val: &kuzu::Value) -> Result<DateTime<Utc>, Error> {
    match val {
        kuzu::Value::Timestamp(odt) => {
            let secs = odt.unix_timestamp();
            let nanos = odt.nanosecond();
            chrono::DateTime::from_timestamp(secs, nanos)
                .ok_or_else(|| Error::Schema("invalid timestamp value".into()))
        }
        _ => Err(Error::Schema(format!("expected timestamp, got {:?}", val))),
    }
}

/// Extract an EntityId from a BLOB value.
fn extract_entity_id(val: &kuzu::Value) -> Result<EntityId, Error> {
    match val {
        kuzu::Value::Blob(v) => {
            let bytes: [u8; 32] = v.as_slice().try_into().map_err(|_| {
                Error::Schema(format!("expected 32-byte blob, got {} bytes", v.len()))
            })?;
            // Reconstruct EntityId from raw bytes via hex round-trip
            let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            hex.parse().map_err(Error::Core)
        }
        _ => Err(Error::Schema(format!("expected blob, got {:?}", val))),
    }
}

/// Extract a BrainId from a BLOB value.
fn extract_brain_id(val: &kuzu::Value) -> Result<BrainId, Error> {
    match val {
        kuzu::Value::Blob(v) => {
            let bytes: [u8; 32] = v.as_slice().try_into().map_err(|_| {
                Error::Schema(format!("expected 32-byte blob, got {} bytes", v.len()))
            })?;
            Ok(BrainId::from_bytes(bytes))
        }
        _ => Err(Error::Schema(format!("expected blob, got {:?}", val))),
    }
}

/// Extract an optional doc ID from a BLOB value (NULL → None).
fn extract_optional_doc_id(val: &kuzu::Value) -> Result<Option<[u8; 32]>, Error> {
    match val {
        kuzu::Value::Null(_) => Ok(None),
        kuzu::Value::Blob(v) => {
            let bytes: [u8; 32] = v.as_slice().try_into().map_err(|_| {
                Error::Schema(format!("expected 32-byte blob, got {} bytes", v.len()))
            })?;
            Ok(Some(bytes))
        }
        _ => Err(Error::Schema(format!(
            "expected blob or null, got {:?}",
            val
        ))),
    }
}

fn extract_string(val: &kuzu::Value) -> Result<String, Error> {
    match val {
        kuzu::Value::String(s) => Ok(s.clone()),
        _ => Err(Error::Schema(format!("expected string, got {:?}", val))),
    }
}

fn extract_double(val: &kuzu::Value) -> Result<f64, Error> {
    match val {
        kuzu::Value::Double(d) => Ok(*d),
        _ => Err(Error::Schema(format!("expected double, got {:?}", val))),
    }
}

fn parse_entity_row(row: &[kuzu::Value]) -> Result<Entity, Error> {
    Ok(Entity {
        id: extract_entity_id(&row[0])?,
        entity_type: extract_string(&row[1])?,
        canonical: extract_string(&row[2])?,
        visibility: str_to_visibility(&extract_string(&row[3])?)?,
        created_at: extract_timestamp(&row[4])?,
        updated_at: extract_timestamp(&row[5])?,
        weight: extract_double(&row[6])?,
    })
}

fn parse_triple_row(row: &[kuzu::Value]) -> Result<Triple, Error> {
    Ok(Triple {
        from: extract_entity_id(&row[0])?,
        to: extract_entity_id(&row[1])?,
        predicate: extract_string(&row[2])?,
        confidence: extract_double(&row[3])?,
        source_doc_id: extract_optional_doc_id(&row[4])?,
        source_brain_id: extract_brain_id(&row[5])?,
        asserted_at: extract_timestamp(&row[6])?,
        visibility: str_to_visibility(&extract_string(&row[7])?)?,
        weight: extract_double(&row[8])?,
    })
}
