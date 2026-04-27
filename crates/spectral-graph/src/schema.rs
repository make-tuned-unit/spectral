//! Kuzu schema definitions and migration runner.
//!
//! Defines the node and relationship tables that form the Spectral graph.
//! ID columns use `BLOB` (raw 32-byte hashes) and time columns use `TIMESTAMP`.
//!
//! # Deviations from spec
//!
//! - `Triple` has no user-defined compound `PRIMARY KEY` — Kuzu rel tables
//!   don't support custom primary keys.
//! - Timestamps are written via `cast($string_param, 'TIMESTAMP')` in Cypher
//!   because the `time` crate is not a direct dependency (Kuzu returns
//!   `time::OffsetDateTime` on read, which we convert to `chrono`).

use kuzu::Connection;

use crate::Error;

/// Current schema version.
const SCHEMA_VERSION: u32 = 1;

/// Returns the current schema version.
///
/// ```
/// use spectral_graph::schema::schema_version;
/// assert_eq!(schema_version(), 1);
/// ```
pub fn schema_version() -> u32 {
    SCHEMA_VERSION
}

/// Create all node and relationship tables. Idempotent.
///
/// # Fresh database
///
/// ```
/// use kuzu::{Database, SystemConfig, Connection};
/// use spectral_graph::schema::create_schema;
///
/// let db = Database::in_memory(SystemConfig::default()).unwrap();
/// let conn = Connection::new(&db).unwrap();
/// create_schema(&conn).unwrap();
/// ```
///
/// # Idempotent — safe to run twice
///
/// ```
/// use kuzu::{Database, SystemConfig, Connection};
/// use spectral_graph::schema::create_schema;
///
/// let db = Database::in_memory(SystemConfig::default()).unwrap();
/// let conn = Connection::new(&db).unwrap();
/// create_schema(&conn).unwrap();
/// create_schema(&conn).unwrap();
/// ```
///
/// # Tables are queryable after creation
///
/// ```
/// use kuzu::{Database, SystemConfig, Connection};
/// use spectral_graph::schema::create_schema;
///
/// let db = Database::in_memory(SystemConfig::default()).unwrap();
/// let conn = Connection::new(&db).unwrap();
/// create_schema(&conn).unwrap();
/// let result = conn.query("MATCH (e:Entity) RETURN e.id").unwrap();
/// assert_eq!(result.get_num_tuples(), 0);
/// ```
///
/// # Schema version is a compile-time constant
///
/// ```
/// use spectral_graph::schema::schema_version;
/// assert!(schema_version() > 0);
/// assert_eq!(schema_version(), schema_version());
/// ```
pub fn create_schema(conn: &Connection) -> Result<(), Error> {
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Entity(
            id BLOB PRIMARY KEY,
            entity_type STRING,
            canonical STRING,
            visibility STRING,
            created_at TIMESTAMP,
            updated_at TIMESTAMP,
            weight DOUBLE DEFAULT 1.0
        )",
    )?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Document(
            id BLOB PRIMARY KEY,
            source STRING,
            ingested_at TIMESTAMP,
            visibility STRING
        )",
    )?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS Triple(
            FROM Entity TO Entity,
            predicate STRING,
            confidence DOUBLE,
            source_doc_id BLOB,
            source_brain_id BLOB,
            asserted_at TIMESTAMP,
            visibility STRING,
            weight DOUBLE
        )",
    )?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS Mentions(
            FROM Document TO Entity,
            span_start INT64,
            span_end INT64,
            extracted_at TIMESTAMP
        )",
    )?;

    Ok(())
}
