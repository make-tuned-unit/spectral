//! `DeviceId` — the second content-addressed identifier, alongside `EntityId`.
//!
//! It is derived from a single descriptor rather than two fields, so it has no
//! field-boundary ambiguity (see `entity_id_contract.rs`, which pins a real
//! collision in the two-field case). What it shares is the storage contract:
//! it is written to the DB as hex and read back, so the round-trip must be
//! exact and malformed input must be *rejected* rather than coerced into a
//! valid-looking id.
//!
//! The in-file unit tests already cover three parse edge cases; this covers
//! the derivation properties, the accessors, and serde.

use spectral_core::device_id::DeviceId;
use std::str::FromStr;

// ── derivation ─────────────────────────────────────────────────────

#[test]
fn the_same_descriptor_always_derives_the_same_id() {
    assert_eq!(
        DeviceId::from_descriptor("laptop-abc"),
        DeviceId::from_descriptor("laptop-abc"),
        "device id derivation is not deterministic"
    );
}

#[test]
fn different_descriptors_derive_different_ids() {
    assert_ne!(
        DeviceId::from_descriptor("laptop-abc"),
        DeviceId::from_descriptor("desktop-xyz")
    );
}

/// Derivation is exact: near-miss descriptors must not collide, or two
/// machines whose hostnames differ only in case would share provenance.
#[test]
fn derivation_is_case_and_whitespace_sensitive() {
    assert_ne!(
        DeviceId::from_descriptor("Laptop"),
        DeviceId::from_descriptor("laptop")
    );
    assert_ne!(
        DeviceId::from_descriptor("laptop"),
        DeviceId::from_descriptor("laptop ")
    );
    assert_ne!(
        DeviceId::from_descriptor(""),
        DeviceId::from_descriptor(" ")
    );
}

#[test]
fn an_empty_descriptor_still_derives_a_wellformed_id() {
    let id = DeviceId::from_descriptor("");
    assert_eq!(id.to_string().len(), 64);
}

/// The salt is versioned, so a `DeviceId` must never equal a raw blake3 of the
/// descriptor — otherwise an unsalted hash from elsewhere could impersonate a
/// device.
#[test]
fn derivation_is_salted_and_not_a_bare_hash_of_the_descriptor() {
    let bare = blake3::hash(b"laptop-abc").to_hex().to_string();
    assert_ne!(
        DeviceId::from_descriptor("laptop-abc").to_string(),
        bare,
        "the device salt is not being applied"
    );
}

// ── accessors ──────────────────────────────────────────────────────

#[test]
fn from_bytes_and_as_bytes_round_trip() {
    let raw = [7u8; 32];
    let id = DeviceId::from_bytes(raw);
    assert_eq!(id.as_bytes(), &raw);
    // And it renders as the hex of those bytes.
    assert_eq!(id.to_string(), "07".repeat(32));
}

#[test]
fn as_bytes_of_a_derived_id_matches_its_rendered_hex() {
    let id = DeviceId::from_descriptor("my-host");
    let from_hex = DeviceId::from_str(&id.to_string()).unwrap();
    assert_eq!(from_hex.as_bytes(), id.as_bytes());
}

// ── string round-trip ──────────────────────────────────────────────

#[test]
fn display_and_from_str_round_trip_exactly() {
    let id = DeviceId::from_descriptor("my-host");
    let text = id.to_string();
    assert_eq!(text.len(), 64);
    assert!(
        text.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "expected lowercase hex, got {text}"
    );
    assert_eq!(DeviceId::from_str(&text).unwrap(), id);
}

/// Boundary byte values must survive: an all-zero and an all-0xff id both
/// round-trip, which a formatter dropping leading zeros would break.
#[test]
fn boundary_byte_values_survive_the_round_trip() {
    for raw in [[0u8; 32], [0xffu8; 32]] {
        let id = DeviceId::from_bytes(raw);
        assert_eq!(
            DeviceId::from_str(&id.to_string()).unwrap().as_bytes(),
            &raw
        );
    }
    assert_eq!(DeviceId::from_bytes([0u8; 32]).to_string(), "0".repeat(64));
}

// ── rejection of malformed input ───────────────────────────────────

#[test]
fn a_wrong_length_string_is_rejected() {
    assert!(DeviceId::from_str("").is_err());
    assert!(DeviceId::from_str("abc").is_err());
    assert!(DeviceId::from_str(&"a".repeat(63)).is_err());
    assert!(DeviceId::from_str(&"a".repeat(65)).is_err());
}

#[test]
fn non_hex_characters_are_rejected_at_either_nibble() {
    // Bad character in the high nibble of the last byte...
    let mut bad = "a".repeat(62);
    bad.push_str("za");
    assert!(
        DeviceId::from_str(&bad).is_err(),
        "high nibble not validated"
    );

    // ... and in the low nibble.
    let mut bad = "a".repeat(62);
    bad.push_str("az");
    assert!(
        DeviceId::from_str(&bad).is_err(),
        "low nibble not validated"
    );
}

/// Parsing is case-insensitive while `Display` always emits lowercase, so the
/// canonical rendering is stable and an uppercase spelling of the same id
/// parses to the same value.
///
/// This is NOT the `EntityId` situation: there, two *distinct entities* can
/// collide onto one id, which is a defect. Here one value simply has two
/// textual spellings, which is ordinary lenient hex parsing.
#[test]
fn parsing_is_case_insensitive_but_rendering_is_canonical_lowercase() {
    let id = DeviceId::from_descriptor("my-host");
    let lower = id.to_string();
    let upper = lower.to_uppercase();

    assert_eq!(
        DeviceId::from_str(&upper).unwrap(),
        id,
        "an uppercase spelling should parse to the same id"
    );
    assert_eq!(
        DeviceId::from_str(&upper).unwrap().to_string(),
        lower,
        "Display must always emit the canonical lowercase form"
    );
}

// ── serde ──────────────────────────────────────────────────────────

#[test]
fn serde_round_trips_through_the_hex_string_form() {
    let id = DeviceId::from_descriptor("my-host");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(
        json,
        format!("\"{id}\""),
        "a DeviceId should serialise as its hex string, not a byte array"
    );
    assert_eq!(serde_json::from_str::<DeviceId>(&json).unwrap(), id);
}

#[test]
fn deserialising_a_malformed_id_is_an_error_not_a_default() {
    assert!(serde_json::from_str::<DeviceId>("\"not-an-id\"").is_err());
    assert!(serde_json::from_str::<DeviceId>("\"\"").is_err());
    assert!(
        serde_json::from_str::<DeviceId>("12345").is_err(),
        "a non-string JSON value should not deserialise into a DeviceId"
    );
}

/// `Debug` is what appears in error messages, so it must carry the full id
/// rather than eliding it.
#[test]
fn debug_contains_the_full_hex_id() {
    let id = DeviceId::from_descriptor("my-host");
    let debug = format!("{id:?}");
    assert!(debug.starts_with("DeviceId("), "got {debug}");
    assert!(debug.contains(&id.to_string()), "got {debug}");
}
