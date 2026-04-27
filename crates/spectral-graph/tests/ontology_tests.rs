use std::path::Path;

use spectral_graph::ontology::Ontology;

#[test]
fn load_from_fixture() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    assert_eq!(ont.version, 1);
    assert_eq!(ont.entities.len(), 2);
    assert_eq!(ont.predicates.len(), 1);
}

#[test]
fn resolve_alias_case_insensitive() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();

    let entity = ont.resolve_alias("sophie").unwrap();
    assert_eq!(entity.canonical, "sophie-sharratt");

    let entity = ont.resolve_alias("SOPHIE").unwrap();
    assert_eq!(entity.canonical, "sophie-sharratt");
}

#[test]
fn resolve_canonical_as_alias() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    let entity = ont.resolve_alias("sophie-sharratt").unwrap();
    assert_eq!(entity.entity_type, "person");
}

#[test]
fn resolve_unknown_returns_none() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    assert!(ont.resolve_alias("nobody").is_none());
}

#[test]
fn entity_id_is_deterministic() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    let id1 = ont.entity_id_for(&ont.entities[0]);
    let id2 = ont.entity_id_for(&ont.entities[0]);
    assert_eq!(id1, id2);
}

#[test]
fn validate_triple_valid() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    ont.validate_triple("works_on", "person", "project")
        .unwrap();
}

#[test]
fn validate_triple_unknown_predicate() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    assert!(ont
        .validate_triple("flies_to", "person", "project")
        .is_err());
}

#[test]
fn validate_triple_wrong_domain() {
    let ont = Ontology::load(Path::new("tests/fixtures/test_ontology.toml")).unwrap();
    assert!(ont
        .validate_triple("works_on", "project", "person")
        .is_err());
}

#[test]
fn invalid_version_rejected() {
    let result = Ontology::from_toml("version = 2");
    assert!(result.is_err());
}

#[test]
fn duplicate_canonical_rejected() {
    let toml = r#"
version = 1
[[entity]]
type = "person"
canonical = "alice"
visibility = "private"
[[entity]]
type = "person"
canonical = "alice"
visibility = "private"
"#;
    assert!(Ontology::from_toml(toml).is_err());
}
