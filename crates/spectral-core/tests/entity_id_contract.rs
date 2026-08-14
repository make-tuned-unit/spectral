//! `EntityId` — the content-addressing contract the graph is built on.
//!
//! Entity ids are derived, not assigned: `blake3(SALT + type + ':' + canonical)`.
//! Three properties follow from that and none were tested:
//!
//! 1. **Determinism** — the same (type, canonical) must always yield the same
//!    id, or the graph fragments across runs and machines.
//! 2. **Field separation** — the `:` separator should keep the two fields
//!    distinct. It does NOT in general: see the known-limitation test below,
//!    which pins a real field-boundary collision.
//! 3. **Exact round-trip** — an id written to the DB as hex must parse back
//!    byte-identically, and anything malformed must be *rejected* rather than
//!    silently coerced into a valid-looking id.

use spectral_core::entity_id::{entity_id, EntityId};
use std::str::FromStr;

// ── derivation ─────────────────────────────────────────────────────

#[test]
fn the_same_inputs_always_derive_the_same_id() {
    let a = entity_id("person", "ada-lovelace");
    let b = entity_id("person", "ada-lovelace");
    assert_eq!(a, b, "entity id derivation is not deterministic");
    assert_eq!(a.as_bytes(), b.as_bytes());
}

#[test]
fn the_type_participates_in_the_id() {
    assert_ne!(
        entity_id("person", "mercury"),
        entity_id("project", "mercury"),
        "two different entity TYPES with the same canonical name collided"
    );
}

#[test]
fn the_canonical_name_participates_in_the_id() {
    assert_ne!(entity_id("person", "ada"), entity_id("person", "grace"));
}

/// **KNOWN LIMITATION — field-boundary ambiguity.**
///
/// The id is `blake3(SALT ‖ type ‖ ':' ‖ canonical)`: a bare one-byte
/// delimiter that is itself a legal character in both fields. So any pair that
/// re-splits to the same byte stream collides — `("person", "a:b")` and
/// `("person:a", "b")` are ONE id, meaning two distinct entities would share a
/// single graph node.
///
/// This is inconsistent with the rest of the codebase, which length-prefixes
/// exactly to avoid this: `memory_signing_payload` ("length-prefixed to
/// prevent field-boundary ambiguity") and `object_hash` ("length-prefixed
/// fields (no delimiter ambiguity)").
///
/// Reachability is low — it needs an entity TYPE containing `:`, and types come
/// from the ontology, where they are conventionally bare words — but nothing
/// validates against it.
///
/// NOT fixed here: length-prefixing changes every entity id ever derived, so it
/// is a data migration rather than a patch. Pinned so the limitation is
/// executable and visible instead of latent, and so a future fix has to
/// confront this test deliberately.
#[test]
fn known_limitation_the_bare_separator_allows_field_boundary_collisions() {
    assert_eq!(
        entity_id("person", "a:b"),
        entity_id("person:a", "b"),
        "the collision documented here no longer reproduces — if the derivation \
         was length-prefixed, update this test and plan the id migration"
    );
}

/// The separator does still separate in the ordinary case, where neither field
/// contains it — which is every real entity type in the ontology.
#[test]
fn the_separator_distinguishes_fields_that_do_not_contain_it() {
    assert_ne!(entity_id("person", "ab"), entity_id("persona", "b"));
    assert_ne!(entity_id("a", "bc"), entity_id("ab", "c"));
}

#[test]
fn derivation_is_case_and_whitespace_sensitive() {
    assert_ne!(entity_id("person", "Ada"), entity_id("person", "ada"));
    assert_ne!(entity_id("person", "ada"), entity_id("person", "ada "));
}

#[test]
fn empty_fields_still_derive_a_well_formed_id() {
    let id = entity_id("", "");
    assert_eq!(id.to_string().len(), 64);
    // And it is still distinct from a non-empty pair.
    assert_ne!(id, entity_id("person", ""));
}

// ── string round-trip ──────────────────────────────────────────────

#[test]
fn display_and_from_str_round_trip_exactly() {
    let original = entity_id("person", "ada-lovelace");
    let text = original.to_string();
    assert_eq!(text.len(), 64, "expected 64 hex chars");
    assert!(text
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));

    let parsed = EntityId::from_str(&text).expect("a rendered id must parse back");
    assert_eq!(parsed, original);
    assert_eq!(parsed.as_bytes(), original.as_bytes());
}

/// Every byte value must survive the hex round-trip, including 0x00 and 0xff
/// at the ends — a naive formatter that dropped a leading zero would corrupt
/// exactly these.
#[test]
fn every_byte_value_survives_the_hex_round_trip() {
    for probe in ["a", "bb", "ccc", "zero", "ffff", "x"] {
        let id = entity_id("t", probe);
        let round = EntityId::from_str(&id.to_string()).unwrap();
        assert_eq!(round.as_bytes(), id.as_bytes(), "probe {probe}");
    }
}

// ── rejection of malformed input ───────────────────────────────────

#[test]
fn a_wrong_length_string_is_rejected() {
    assert!(EntityId::from_str("").is_err());
    assert!(EntityId::from_str("abc").is_err());
    assert!(EntityId::from_str(&"a".repeat(63)).is_err());
    assert!(EntityId::from_str(&"a".repeat(65)).is_err());
}

#[test]
fn non_hex_characters_are_rejected() {
    // 64 chars, but 'z' is not a hex digit.
    let mut bad = "a".repeat(63);
    bad.push('z');
    assert!(
        EntityId::from_str(&bad).is_err(),
        "a non-hex character was accepted"
    );
}

/// `u8::from_str_radix` would accept a leading `+` and surrounding
/// whitespace; the parser rejects them deliberately so that round-tripping is
/// exact. Pinned because accepting them would let two distinct strings map to
/// one id.
#[test]
fn plus_signs_and_whitespace_are_rejected() {
    for bad in [
        "+".to_string() + &"a".repeat(63),
        " ".to_string() + &"a".repeat(63),
    ] {
        assert!(
            EntityId::from_str(&bad).is_err(),
            "accepted a non-canonical encoding: {bad:?}"
        );
    }
}

/// The parser operates on raw bytes because a 64-*byte* string may not be 64
/// *chars*. A multi-byte character must produce an error, never a panic on a
/// char boundary.
#[test]
fn multibyte_input_of_the_right_byte_length_errors_without_panicking() {
    // 'é' is 2 bytes in UTF-8, so this is 64 bytes but 63 chars.
    let s = format!("{}é", "a".repeat(62));
    assert_eq!(s.len(), 64, "test fixture should be 64 BYTES");
    assert!(
        EntityId::from_str(&s).is_err(),
        "multibyte input should be rejected as non-hex"
    );
}

// ── serde ──────────────────────────────────────────────────────────

#[test]
fn serde_round_trips_through_the_hex_string_form() {
    let id = entity_id("project", "spectral");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(
        json,
        format!("\"{id}\""),
        "an EntityId should serialise as its hex string, not as a byte array"
    );
    let back: EntityId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}

#[test]
fn deserialising_a_malformed_id_is_an_error_not_a_default() {
    assert!(serde_json::from_str::<EntityId>("\"not-an-id\"").is_err());
    assert!(serde_json::from_str::<EntityId>("\"\"").is_err());
}

/// `Debug` wraps `Display` — used in error messages, so it must stay readable
/// and must contain the full id.
#[test]
fn debug_contains_the_full_hex_id() {
    let id = entity_id("person", "ada");
    let debug = format!("{id:?}");
    assert!(debug.contains(&id.to_string()), "got {debug}");
    assert!(debug.starts_with("EntityId("));
}
