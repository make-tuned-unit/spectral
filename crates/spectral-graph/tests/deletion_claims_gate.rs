//! Claims gate: pins `docs/DELETION_GUARANTEES.md` to evidence.
//!
//! Every claim heading (D1..D5) in the public doc must map — through
//! `docs/deletion-guarantees-inventory.json` — to test functions that
//! actually exist in `tests/deletion_guarantees.rs`. A public statement
//! without an enforcing test fails the build; so does an inventory entry
//! pointing at a renamed/deleted test. Same discipline as the recognition
//! proof suite: no claim without a test enforcing it.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    let path = repo_path(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Claim identifiers appearing as markdown headings ("## D1 — ...") in the
/// public doc.
fn doc_claims(doc: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in doc.lines() {
        let line = line.trim();
        if !line.starts_with('#') {
            continue;
        }
        let heading = line.trim_start_matches('#').trim();
        // "D1 — ...", "D3 (a–c)" etc. — take a leading D<digit> token.
        if let Some(rest) = heading.strip_prefix('D') {
            if let Some(d) = rest.chars().next() {
                if d.is_ascii_digit() {
                    out.insert(format!("D{d}"));
                }
            }
        }
    }
    out
}

#[test]
fn every_public_deletion_claim_maps_to_an_existing_test() {
    let doc = read("docs/DELETION_GUARANTEES.md");
    let inventory: serde_json::Value =
        serde_json::from_str(&read("docs/deletion-guarantees-inventory.json"))
            .expect("inventory must be valid JSON");
    let suite_src = read("crates/spectral-graph/tests/deletion_guarantees.rs");

    let claims = doc_claims(&doc);
    let expected: BTreeSet<String> = ["D1", "D2", "D3", "D4", "D5"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        claims, expected,
        "the public doc must state exactly the pre-registered claims D1..D5"
    );

    let inv_claims = inventory["claims"]
        .as_object()
        .expect("inventory.claims must be an object");
    let inv_keys: BTreeSet<String> = inv_claims.keys().cloned().collect();
    assert_eq!(
        inv_keys, claims,
        "inventory claims must match the doc's claim headings exactly"
    );

    for (claim, tests) in inv_claims {
        let tests = tests
            .as_array()
            .unwrap_or_else(|| panic!("inventory claim {claim} must list tests"));
        assert!(!tests.is_empty(), "claim {claim} has no enforcing test");
        for t in tests {
            let name = t.as_str().expect("test names must be strings");
            let needle = format!("fn {name}(");
            assert!(
                suite_src.contains(&needle),
                "claim {claim}: test `{name}` does not exist in deletion_guarantees.rs — \
                 the public claim lost its evidence"
            );
            // The mapped function must actually be a test, not a helper.
            let fn_pos = suite_src.find(&needle).unwrap();
            let preceding = &suite_src[..fn_pos];
            let last_attr = preceding.rfind("#[test]");
            assert!(
                last_attr.is_some() && preceding[last_attr.unwrap()..].lines().count() <= 3,
                "claim {claim}: `{name}` is not annotated #[test]"
            );
        }
    }

    // The inventory must point at the suite file that exists.
    let suite_rel = inventory["test_file"].as_str().unwrap();
    assert!(
        repo_path(suite_rel).exists(),
        "inventory test_file missing: {suite_rel}"
    );
}
