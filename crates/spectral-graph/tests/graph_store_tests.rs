use chrono::Utc;
use tempfile::TempDir;

use spectral_core::entity_id::entity_id;
use spectral_core::identity::BrainIdentity;
use spectral_core::visibility::Visibility;
use spectral_graph::graph_store::{Entity, GraphStore, Triple};

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
        description: None,
    }
}

#[test]
fn open_fresh_db_on_disk() {
    let tmp = TempDir::new().unwrap();
    let _store = GraphStore::open(&tmp.path().join("test.db")).unwrap();
}

#[test]
fn upsert_and_get_entity() {
    let store = GraphStore::in_memory().unwrap();
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
    let store = GraphStore::in_memory().unwrap();
    let entity = make_entity("person", "alice");
    let id = entity.id;

    store.upsert_entity(&entity).unwrap();
    store.upsert_entity(&entity).unwrap();

    let found = store.get_entity(&id).unwrap().unwrap();
    assert_eq!(found.canonical, "alice");
}

#[test]
fn get_missing_entity_returns_none() {
    let store = GraphStore::in_memory().unwrap();
    let id = entity_id("person", "nobody");
    assert!(store.get_entity(&id).unwrap().is_none());
}

#[test]
fn insert_and_find_triples() {
    let store = GraphStore::in_memory().unwrap();
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
    let store = GraphStore::in_memory().unwrap();
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
    let store = GraphStore::in_memory().unwrap();
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

#[test]
fn insert_mention_is_idempotent() {
    let store = GraphStore::in_memory().unwrap();
    let entity = make_entity("person", "alice");
    store.upsert_entity(&entity).unwrap();

    let doc_id = *blake3::hash(b"hello world").as_bytes();
    store
        .upsert_document(&doc_id, "test.txt", Visibility::Private)
        .unwrap();

    // Insert the same mention twice
    store.insert_mention(&doc_id, &entity.id, 0, 5).unwrap();
    store.insert_mention(&doc_id, &entity.id, 0, 5).unwrap();

    // Should have exactly one edge, not two
    let count = store.count_mentions(&doc_id, &entity.id).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn insert_mention_distinct_entities_both_kept() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    let bob = make_entity("person", "bob");
    store.upsert_entity(&alice).unwrap();
    store.upsert_entity(&bob).unwrap();

    let doc_id = *blake3::hash(b"alice and bob").as_bytes();
    store
        .upsert_document(&doc_id, "test.txt", Visibility::Private)
        .unwrap();

    store.insert_mention(&doc_id, &alice.id, 0, 5).unwrap();
    store.insert_mention(&doc_id, &bob.id, 10, 13).unwrap();

    // Both edges should exist
    let count_alice = store.count_mentions(&doc_id, &alice.id).unwrap();
    let count_bob = store.count_mentions(&doc_id, &bob.id).unwrap();
    assert_eq!(count_alice, 1);
    assert_eq!(count_bob, 1);
}

#[test]
fn neighborhood_surfaces_mentioning_documents() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    store.upsert_entity(&alice).unwrap();

    let doc_id = *blake3::hash(b"alice doc content").as_bytes();
    store
        .upsert_document(&doc_id, "notes.txt", Visibility::Private)
        .unwrap();
    store.insert_mention(&doc_id, &alice.id, 0, 5).unwrap();

    let hood = store.neighborhood(&alice.id, 1).unwrap();
    assert_eq!(hood.entities.len(), 1); // alice
    assert_eq!(hood.documents.len(), 1); // notes.txt
    assert_eq!(hood.documents[0].source, "notes.txt");
    assert_eq!(hood.documents[0].id, doc_id);
}

#[test]
fn neighborhood_documents_are_terminal() {
    let store = GraphStore::in_memory().unwrap();

    let alice = make_entity("person", "alice");
    let bob = make_entity("person", "bob");
    store.upsert_entity(&alice).unwrap();
    store.upsert_entity(&bob).unwrap();

    // doc1 mentions alice, doc1 also mentions bob
    // If Documents expanded, bob would be reachable from alice via doc1.
    // But Documents are terminal — bob should NOT appear.
    let doc_id = *blake3::hash(b"shared doc").as_bytes();
    store
        .upsert_document(&doc_id, "shared.txt", Visibility::Private)
        .unwrap();
    store.insert_mention(&doc_id, &alice.id, 0, 5).unwrap();
    store.insert_mention(&doc_id, &bob.id, 10, 13).unwrap();

    // No Triple edges — alice and bob are not connected via Entity graph
    let hood = store.neighborhood(&alice.id, 2).unwrap();
    // alice only (bob not reachable via Triple edges)
    assert_eq!(hood.entities.len(), 1);
    assert_eq!(hood.entities[0].canonical, "alice");
    // Document surfaces because it mentions alice
    assert_eq!(hood.documents.len(), 1);
    assert_eq!(hood.documents[0].source, "shared.txt");
}

#[test]
fn neighborhood_combined_triples_and_documents() {
    let store = GraphStore::in_memory().unwrap();
    let brain = BrainIdentity::generate();
    let now = Utc::now();

    let alice = make_entity("person", "alice");
    let spectral = make_entity("project", "spectral");
    store.upsert_entity(&alice).unwrap();
    store.upsert_entity(&spectral).unwrap();

    // alice -> spectral via Triple
    store
        .insert_triple(&Triple {
            from: alice.id,
            to: spectral.id,
            predicate: "works_on".into(),
            confidence: 0.9,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .unwrap();

    // doc mentions alice
    let doc_id = *blake3::hash(b"alice meeting notes").as_bytes();
    store
        .upsert_document(&doc_id, "meeting.txt", Visibility::Private)
        .unwrap();
    store.insert_mention(&doc_id, &alice.id, 0, 5).unwrap();

    let hood = store.neighborhood(&alice.id, 2).unwrap();
    // Both entities reachable via Triple
    assert_eq!(hood.entities.len(), 2);
    assert_eq!(hood.triples.len(), 1);
    // Document surfaces via Mentions
    assert_eq!(hood.documents.len(), 1);
    assert_eq!(hood.documents[0].source, "meeting.txt");
}

#[test]
fn set_entity_description_write_then_read() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    store.upsert_entity(&alice).unwrap();

    store
        .set_entity_description(&alice.id, "Alice is an engineer")
        .unwrap();

    let fetched = store.get_entity(&alice.id).unwrap().unwrap();
    assert_eq!(
        fetched.description,
        Some("Alice is an engineer".to_string())
    );
}

#[test]
fn set_entity_description_idempotent() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    store.upsert_entity(&alice).unwrap();

    store
        .set_entity_description(&alice.id, "Alice is an engineer")
        .unwrap();
    store
        .set_entity_description(&alice.id, "Alice is an engineer")
        .unwrap();

    let fetched = store.get_entity(&alice.id).unwrap().unwrap();
    assert_eq!(
        fetched.description,
        Some("Alice is an engineer".to_string())
    );
}

#[test]
fn entity_with_null_description() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    store.upsert_entity(&alice).unwrap();

    let fetched = store.get_entity(&alice.id).unwrap().unwrap();
    assert_eq!(fetched.description, None);
}

#[test]
fn set_entity_description_overwrites() {
    let store = GraphStore::in_memory().unwrap();
    let alice = make_entity("person", "alice");
    store.upsert_entity(&alice).unwrap();

    store
        .set_entity_description(&alice.id, "first version")
        .unwrap();
    store
        .set_entity_description(&alice.id, "improved version")
        .unwrap();

    let fetched = store.get_entity(&alice.id).unwrap().unwrap();
    assert_eq!(fetched.description, Some("improved version".to_string()));
}
