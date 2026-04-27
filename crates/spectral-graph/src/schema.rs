//! Kuzu schema definitions and migration runner.
//!
//! Defines the node and relationship tables that form the Spectral graph.
//!
//! # Deviations from spec
//!
//! - ID columns use `STRING` (hex-encoded blake3) instead of `BLOB` to avoid
//!   potential primary-key restrictions on binary types.
//! - Timestamp columns use `STRING` (RFC 3339) instead of `TIMESTAMP` because
//!   the `time` crate is not a direct dependency.
//! - `Triple` has no user-defined compound `PRIMARY KEY` — Kuzu rel tables
//!   don't support custom primary keys.

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
            id STRING PRIMARY KEY,
            entity_type STRING,
            canonical STRING,
            visibility STRING,
            created_at STRING,
            updated_at STRING,
            weight DOUBLE DEFAULT 1.0
        )",
    )?;

    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS Document(
            id STRING PRIMARY KEY,
            source STRING,
            ingested_at STRING,
            visibility STRING
        )",
    )?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS Triple(
            FROM Entity TO Entity,
            predicate STRING,
            confidence DOUBLE,
            source_doc_id STRING,
            source_brain_id STRING,
            asserted_at STRING,
            visibility STRING,
            weight DOUBLE
        )",
    )?;

    conn.query(
        "CREATE REL TABLE IF NOT EXISTS Mentions(
            FROM Document TO Entity,
            span_start INT64,
            span_end INT64,
            extracted_at STRING
        )",
    )?;

    Ok(())
}
