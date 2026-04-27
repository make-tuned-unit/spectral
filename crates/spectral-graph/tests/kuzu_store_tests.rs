use chrono::Utc;
use tempfile::TempDir;

use spectral_core::entity_id::entity_id;
use spectral_core::identity::BrainIdentity;
use spectral_core::visibility::Visibility;
use spectral_graph::kuzu_store::{Entity, KuzuStore, Triple};

fn make_entity(id_type: &str, name: &str) -> Entity {
    let now = Utc::now();
    Entity {
        id: entity_id(id_type, name),
        entity_type: id_type.into(),
        canonical: name.into(),
        visibility: Visibility::Private,
        created_at: now,
        updated_at: now,
        weight: 1.0,
    }
}

#[test]
fn open_fresh_db_on_disk() {
    let tmp = TempDir::new().unwrap();
    let _store = KuzuStore::open(&tmp.path().join("test.db")).unwrap();
}

#[test]
fn upsert_and_get_entity() {
    let store = KuzuStore::in_memory().unwrap();
    let entity = make_entity("person", "alice");
    let id = entity.id;

    store.upsert_entity(&entity).unwrap();

    let found = store.get_entity(&id).unwrap().unwrap();
    assert_eq!(found.entity_type, "person");
    assert_eq!(found.canonical, "alice");
    assert!((found.weight - 1.0).abs() < f64::EPSILON);
}

#[test]
fn upsert_is_idempotent() {
    let store = KuzuStore::in_memory().unwrap();
    let entity = make_entity("person", "alice");
    let id = entity.id;

    store.upsert_entity(&entity).unwrap();
    store.upsert_entity(&entity).unwrap();

    let found = store.get_entity(&id).unwrap().unwrap();
    assert_eq!(found.canonical, "alice");
}

#[test]
fn get_missing_entity_returns_none() {
    let store = KuzuStore::in_memory().unwrap();
    let id = entity_id("person", "nobody");
    assert!(store.get_entity(&id).unwrap().is_none());
}

#[test]
fn insert_and_find_triples() {
    let store = KuzuStore::in_memory().unwrap();
    let a = make_entity("person", "alice");
    let b = make_entity("project", "spectral");
    let brain = BrainIdentity::generate();
    let now = Utc::now();

    store.upsert_entity(&a).unwrap();
    store.upsert_entity(&b).unwrap();

    store
        .insert_triple(&Triple {
            from: a.id,
            to: b.id,
            predicate: "works_on".into(),
            confidence: 0.95,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    let triples = store.find_triples(Some(&a.id), None, None).unwrap();
    assert_eq!(triples.len(), 1);
    assert_eq!(triples[0].predicate, "works_on");
    assert_eq!(triples[0].from, a.id);
    assert_eq!(triples[0].to, b.id);
}

#[test]
fn find_triples_by_predicate() {
    let store = KuzuStore::in_memory().unwrap();
    let a = make_entity("person", "alice");
    let b = make_entity("person", "bob");
    let c = make_entity("project", "spectral");
    let brain = BrainIdentity::generate();
    let now = Utc::now();

    store.upsert_entity(&a).unwrap();
    store.upsert_entity(&b).unwrap();
    store.upsert_entity(&c).unwrap();

    store
        .insert_triple(&Triple {
            from: a.id,
            to: b.id,
            predicate: "knows".into(),
            confidence: 1.0,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    store
        .insert_triple(&Triple {
            from: a.id,
            to: c.id,
            predicate: "works_on".into(),
            confidence: 0.9,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    let knows = store.find_triples(None, None, Some("knows")).unwrap();
    assert_eq!(knows.len(), 1);

    let works = store.find_triples(None, None, Some("works_on")).unwrap();
    assert_eq!(works.len(), 1);
}

#[test]
fn neighborhood_2_hop_traversal() {
    let store = KuzuStore::in_memory().unwrap();
    let a = make_entity("person", "alice");
    let b = make_entity("person", "bob");
    let c = make_entity("project", "spectral");
    let brain = BrainIdentity::generate();
    let now = Utc::now();

    store.upsert_entity(&a).unwrap();
    store.upsert_entity(&b).unwrap();
    store.upsert_entity(&c).unwrap();

    // a -> b -> c
    store
        .insert_triple(&Triple {
            from: a.id,
            to: b.id,
            predicate: "knows".into(),
            confidence: 1.0,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    store
        .insert_triple(&Triple {
            from: b.id,
            to: c.id,
            predicate: "works_on".into(),
            confidence: 1.0,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    // 1-hop from a: should find a and b
    let hood1 = store.neighborhood(&a.id, 1).unwrap();
    assert_eq!(hood1.entities.len(), 2);

    // 2-hop from a: should find a, b, and c
    let hood2 = store.neighborhood(&a.id, 2).unwrap();
    assert_eq!(hood2.entities.len(), 3);
    assert_eq!(hood2.triples.len(), 2);
}
