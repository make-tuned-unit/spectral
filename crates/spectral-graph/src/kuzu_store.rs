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

/// Result of a neighborhood BFS traversal.
#[derive(Debug)]
pub struct Neighborhood {
    /// All visited entities (including the start).
    pub entities: Vec<Entity>,
    /// All triples connecting visited entities (deduplicated).
    pub triples: Vec<Triple>,
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
                 e.created_at = $created,
                 e.updated_at = $updated,
                 e.weight = $weight",
        )?;
        conn.execute(
            &mut stmt,
            vec![
                ("id", kuzu::Value::String(entity.id.to_string())),
                ("type", kuzu::Value::String(entity.entity_type.clone())),
                ("canon", kuzu::Value::String(entity.canonical.clone())),
                (
                    "vis",
                    kuzu::Value::String(visibility_to_str(entity.visibility)),
                ),
                (
                    "created",
                    kuzu::Value::String(entity.created_at.to_rfc3339()),
                ),
                (
                    "updated",
                    kuzu::Value::String(entity.updated_at.to_rfc3339()),
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
                 asserted_at: $asserted,
                 visibility: $vis,
                 weight: $weight
             }]->(b)",
        )?;
        let doc_id_str = triple
            .source_doc_id
            .map(|b| bytes_to_hex(&b))
            .unwrap_or_default();
        conn.execute(
            &mut stmt,
            vec![
                ("from_id", kuzu::Value::String(triple.from.to_string())),
                ("to_id", kuzu::Value::String(triple.to.to_string())),
                ("pred", kuzu::Value::String(triple.predicate.clone())),
                ("conf", kuzu::Value::Double(triple.confidence)),
                ("doc_id", kuzu::Value::String(doc_id_str)),
                (
                    "brain_id",
                    kuzu::Value::String(triple.source_brain_id.to_string()),
                ),
                (
                    "asserted",
                    kuzu::Value::String(triple.asserted_at.to_rfc3339()),
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
            params.push(("from_id", kuzu::Value::String(f.to_string())));
        }
        if let Some(t) = to {
            conditions.push("b.id = $to_id");
            params.push(("to_id", kuzu::Value::String(t.to_string())));
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

        Ok(Neighborhood {
            entities: all_entities,
            triples: all_triples,
        })
    }

    // --- Private helpers ---

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
        let mut result =
            conn.execute(&mut stmt, vec![("id", kuzu::Value::String(id.to_string()))])?;
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
        let result = conn.execute(&mut stmt, vec![("id", kuzu::Value::String(id.to_string()))])?;
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

fn str_to_datetime(s: &str) -> Result<DateTime<Utc>, Error> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::Schema(format!("invalid timestamp: {e}")))
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_bytes(hex: &str) -> Result<[u8; 32], Error> {
    if hex.len() != 64 {
        return Err(Error::Schema(format!(
            "expected 64 hex chars, got {}",
            hex.len()
        )));
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| Error::Schema(format!("invalid hex at position {}", i * 2)))?;
    }
    Ok(bytes)
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

fn extract_optional_doc_id(val: &kuzu::Value) -> Result<Option<[u8; 32]>, Error> {
    match val {
        kuzu::Value::Null(_) => Ok(None),
        kuzu::Value::String(s) if s.is_empty() => Ok(None),
        kuzu::Value::String(s) => Ok(Some(hex_to_bytes(s)?)),
        _ => Err(Error::Schema(format!(
            "expected string or null, got {:?}",
            val
        ))),
    }
}

fn parse_entity_row(row: &[kuzu::Value]) -> Result<Entity, Error> {
    Ok(Entity {
        id: extract_string(&row[0])?.parse()?,
        entity_type: extract_string(&row[1])?,
        canonical: extract_string(&row[2])?,
        visibility: str_to_visibility(&extract_string(&row[3])?)?,
        created_at: str_to_datetime(&extract_string(&row[4])?)?,
        updated_at: str_to_datetime(&extract_string(&row[5])?)?,
        weight: extract_double(&row[6])?,
    })
}

fn parse_triple_row(row: &[kuzu::Value]) -> Result<Triple, Error> {
    Ok(Triple {
        from: extract_string(&row[0])?.parse()?,
        to: extract_string(&row[1])?.parse()?,
        predicate: extract_string(&row[2])?,
        confidence: extract_double(&row[3])?,
        source_doc_id: extract_optional_doc_id(&row[4])?,
        source_brain_id: BrainId::from_bytes(hex_to_bytes(&extract_string(&row[5])?)?),
        asserted_at: str_to_datetime(&extract_string(&row[6])?)?,
        visibility: str_to_visibility(&extract_string(&row[7])?)?,
        weight: extract_double(&row[8])?,
    })
}
