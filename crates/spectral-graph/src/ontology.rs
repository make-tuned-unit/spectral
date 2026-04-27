//! TOML ontology loader and validation.
//!
//! Loads entity and predicate definitions from a TOML file, validates
//! structural invariants, and provides lookup methods for alias resolution
//! and triple validation.

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use spectral_core::entity_id::{self, EntityId};
use spectral_core::visibility::Visibility;

use crate::Error;

/// A loaded and validated ontology.
///
/// # Load from TOML string
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let toml = r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice"
/// aliases = ["Alice"]
/// visibility = "private"
///
/// [[predicate]]
/// name = "knows"
/// domain = ["person"]
/// range = ["person"]
/// "#;
/// let ont = Ontology::from_toml(toml).unwrap();
/// assert_eq!(ont.version, 1);
/// assert_eq!(ont.entities.len(), 1);
/// ```
///
/// # Resolve an alias (case-insensitive)
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let toml = r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice-smith"
/// aliases = ["Alice", "Alice Smith"]
/// visibility = "private"
/// "#;
/// let ont = Ontology::from_toml(toml).unwrap();
/// let entity = ont.resolve_alias("alice").unwrap();
/// assert_eq!(entity.canonical, "alice-smith");
/// ```
///
/// # Validate a triple against ontology constraints
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let toml = r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice"
/// visibility = "private"
/// [[entity]]
/// type = "project"
/// canonical = "spectral"
/// visibility = "public"
/// [[predicate]]
/// name = "works_on"
/// domain = ["person"]
/// range = ["project"]
/// "#;
/// let ont = Ontology::from_toml(toml).unwrap();
/// assert!(ont.validate_triple("works_on", "person", "project").is_ok());
/// ```
///
/// # Error: unknown predicate
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let toml = r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice"
/// visibility = "private"
/// "#;
/// let ont = Ontology::from_toml(toml).unwrap();
/// assert!(ont.validate_triple("unknown", "person", "project").is_err());
/// ```
///
/// # Error: unsupported version
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let result = Ontology::from_toml("version = 99");
/// assert!(result.is_err());
/// ```
///
/// # Compute deterministic EntityId for an ontology entity
///
/// ```
/// use spectral_graph::ontology::Ontology;
///
/// let toml = r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice"
/// visibility = "private"
/// "#;
/// let ont = Ontology::from_toml(toml).unwrap();
/// let id1 = ont.entity_id_for(&ont.entities[0]);
/// let id2 = ont.entity_id_for(&ont.entities[0]);
/// assert_eq!(id1, id2);
/// ```
#[derive(Debug, Deserialize)]
pub struct Ontology {
    /// Schema version (must be 1).
    pub version: u32,
    /// Entity definitions.
    #[serde(rename = "entity", default)]
    pub entities: Vec<OntologyEntity>,
    /// Predicate definitions.
    #[serde(rename = "predicate", default)]
    pub predicates: Vec<OntologyPredicate>,
}

/// An entity definition in the ontology.
#[derive(Debug, Clone, Deserialize)]
pub struct OntologyEntity {
    /// Entity type (e.g. "person", "project").
    #[serde(rename = "type")]
    pub entity_type: String,
    /// Canonical name (e.g. "sophie-sharratt").
    pub canonical: String,
    /// Known aliases for this entity.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Default visibility for this entity.
    pub visibility: Visibility,
}

/// A predicate definition in the ontology.
#[derive(Debug, Clone, Deserialize)]
pub struct OntologyPredicate {
    /// Predicate name (e.g. "works_on").
    pub name: String,
    /// Entity types allowed as subject.
    pub domain: Vec<String>,
    /// Entity types allowed as object.
    pub range: Vec<String>,
    /// Whether the predicate is symmetric.
    #[serde(default)]
    pub symmetric: bool,
}

impl Ontology {
    /// Load and validate an ontology from a TOML file.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }

    /// Parse and validate an ontology from a TOML string.
    pub fn from_toml(content: &str) -> Result<Self, Error> {
        let ontology: Ontology = toml::from_str(content)?;
        ontology.validate()?;
        Ok(ontology)
    }

    /// Validate structural invariants.
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::Ontology(format!(
                "unsupported ontology version: {} (expected 1)",
                self.version
            )));
        }

        let mut entity_types: HashSet<&str> = HashSet::new();
        let mut canonicals_per_type: HashSet<(&str, &str)> = HashSet::new();

        for entity in &self.entities {
            if entity.entity_type.is_empty() {
                return Err(Error::Ontology("entity has empty type".into()));
            }
            if entity.canonical.is_empty() {
                return Err(Error::Ontology("entity has empty canonical".into()));
            }
            entity_types.insert(&entity.entity_type);
            if !canonicals_per_type.insert((&entity.entity_type, &entity.canonical)) {
                return Err(Error::Ontology(format!(
                    "duplicate canonical '{}' for type '{}'",
                    entity.canonical, entity.entity_type
                )));
            }
        }

        let mut predicate_names: HashSet<&str> = HashSet::new();
        for pred in &self.predicates {
            if !predicate_names.insert(&pred.name) {
                return Err(Error::Ontology(format!(
                    "duplicate predicate: '{}'",
                    pred.name
                )));
            }
            for d in &pred.domain {
                if !entity_types.contains(d.as_str()) {
                    return Err(Error::Ontology(format!(
                        "predicate '{}' domain references unknown type '{}'",
                        pred.name, d
                    )));
                }
            }
            for r in &pred.range {
                if !entity_types.contains(r.as_str()) {
                    return Err(Error::Ontology(format!(
                        "predicate '{}' range references unknown type '{}'",
                        pred.name, r
                    )));
                }
            }
        }

        Ok(())
    }

    /// Look up an entity by any of its aliases (case-insensitive exact match).
    /// The canonical name is implicitly an alias.
    pub fn resolve_alias(&self, mention: &str) -> Option<&OntologyEntity> {
        let lower = mention.to_lowercase();
        self.entities.iter().find(|e| {
            e.canonical.to_lowercase() == lower
                || e.aliases.iter().any(|a| a.to_lowercase() == lower)
        })
    }

    /// Compute the EntityId for an ontology entity.
    pub fn entity_id_for(&self, entity: &OntologyEntity) -> EntityId {
        entity_id::entity_id(&entity.entity_type, &entity.canonical)
    }

    /// Validate a triple's predicate against domain/range constraints.
    pub fn validate_triple(
        &self,
        predicate: &str,
        subject_type: &str,
        object_type: &str,
    ) -> Result<(), Error> {
        let pred = self
            .predicates
            .iter()
            .find(|p| p.name == predicate)
            .ok_or_else(|| Error::Ontology(format!("unknown predicate: '{predicate}'")))?;

        if !pred.domain.iter().any(|d| d == subject_type) {
            return Err(Error::Ontology(format!(
                "subject type '{subject_type}' not in domain of '{predicate}'"
            )));
        }
        if !pred.range.iter().any(|r| r == object_type) {
            return Err(Error::Ontology(format!(
                "object type '{object_type}' not in range of '{predicate}'"
            )));
        }

        Ok(())
    }
}
