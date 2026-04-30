use spectral_graph::canonicalize::{Canonicalizer, MatchKind};
use spectral_graph::ontology::Ontology;
use std::path::Path;

fn load_ontology() -> Ontology {
    Ontology::load(Path::new("tests/fixtures/brain_ontology.toml")).unwrap()
}

#[test]
fn exact_alias_case_insensitive() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    let m = c.resolve_one("carol").unwrap();
    assert_eq!(m.canonical, "carol-doe");
    assert!(matches!(m.match_kind, MatchKind::Exact));

    let m = c.resolve_one("MARK").unwrap();
    assert_eq!(m.canonical, "mark-smith");
}

#[test]
fn multi_word_alias_prefers_longest() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    // "Carol Doe" should match as one entity, not "Carol" alone
    let result = c.canonicalize("Carol Doe works here");
    assert_eq!(result.matched.len(), 1);
    assert_eq!(result.matched[0].mention, "Carol Doe");
    assert_eq!(result.matched[0].canonical, "carol-doe");
}

#[test]
fn fuzzy_match_within_threshold() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    // "Spectrl" → "Spectral" (1 char missing from 8 = 0.875 similarity)
    let m = c.resolve_one("Spectrl").unwrap();
    assert_eq!(m.canonical, "spectral");
    assert!(matches!(m.match_kind, MatchKind::Fuzzy { score } if score > 0.85));
}

#[test]
fn fuzzy_match_below_threshold_unresolved() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    // "Caral" vs "Carol": distance ~1/5 = 0.8 similarity → unresolved with nearest
    let result = c.canonicalize("Caral is here");
    assert!(result.matched.is_empty() || !result.matched.iter().any(|m| m.mention == "Caral"));
    let unresolved = result.unresolved.iter().find(|u| u.mention == "Caral");
    assert!(unresolved.is_some());
    let u = unresolved.unwrap();
    assert!(u.nearest.is_some());
    assert_eq!(u.nearest.as_ref().unwrap().entity_type, "person");
}

#[test]
fn unresolved_no_nearest_for_unrelated() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    // A completely unrelated word won't have a nearest match
    let result = c.canonicalize("Xyzzyplugh runs fast");
    // "Xyzzyplugh" has no similarity to any alias
    let unresolved_xyz = result.unresolved.iter().find(|u| u.mention == "Xyzzyplugh");
    // Either not reported (score too low) or reported with nearest: None
    if let Some(u) = unresolved_xyz {
        assert!(u.nearest.is_none());
    }
}

#[test]
fn span_offsets_track_bytes() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    let text = "Hello Carol and Mark";
    let result = c.canonicalize(text);
    assert_eq!(result.matched.len(), 2);

    let carol = result
        .matched
        .iter()
        .find(|m| m.canonical == "carol-doe")
        .unwrap();
    assert_eq!(&text[carol.span.0..carol.span.1], "Carol");

    let mark = result
        .matched
        .iter()
        .find(|m| m.canonical == "mark-smith")
        .unwrap();
    assert_eq!(&text[mark.span.0..mark.span.1], "Mark");
}

#[test]
fn empty_text_no_panic() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    let result = c.canonicalize("");
    assert!(result.matched.is_empty());
    assert!(result.unresolved.is_empty());
}

#[test]
fn resolve_one_returns_none_for_unknown() {
    let ont = load_ontology();
    let c = Canonicalizer::new(&ont);

    assert!(c.resolve_one("zzzzzzz").is_none());
}
