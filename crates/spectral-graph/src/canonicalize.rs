//! Canonicalization: resolve free-text mentions to ontology entities.
//!
//! The [`Canonicalizer`] is the input layer of the Brain API. Callers pass
//! natural-language text; the canonicalizer resolves mentions to [`EntityId`]s
//! using exact and fuzzy string matching against ontology aliases.

use spectral_core::entity_id::EntityId;

use crate::ontology::{Ontology, OntologyEntity};

/// Result of canonicalizing a text.
#[derive(Debug)]
pub struct CanonicalizeResult {
    /// Mentions successfully matched to ontology entities.
    pub matched: Vec<MatchedMention>,
    /// Mentions that could not be resolved.
    pub unresolved: Vec<UnresolvedMention>,
}

/// A mention successfully matched to an ontology entity.
#[derive(Debug, Clone)]
pub struct MatchedMention {
    /// The original text of the mention.
    pub mention: String,
    /// Byte offsets (start, end) in the input text.
    pub span: (usize, usize),
    /// The resolved entity ID.
    pub entity_id: EntityId,
    /// Entity type (e.g. "person").
    pub entity_type: String,
    /// Canonical name of the matched entity.
    pub canonical: String,
    /// How the match was made.
    pub match_kind: MatchKind,
}

/// A mention that could not be resolved to an ontology entity.
#[derive(Debug, Clone)]
pub struct UnresolvedMention {
    /// The original text of the mention.
    pub mention: String,
    /// Byte offsets (start, end) in the input text.
    pub span: (usize, usize),
    /// Top suggestion if any alias was within 0.5 similarity.
    pub nearest: Option<NearestMatch>,
}

/// A near-miss suggestion for an unresolved mention.
#[derive(Debug, Clone)]
pub struct NearestMatch {
    /// Canonical name of the nearest entity.
    pub canonical: String,
    /// Entity type of the nearest entity.
    pub entity_type: String,
    /// Similarity score (0.0–1.0).
    pub score: f64,
}

/// How a mention was matched.
#[derive(Debug, Clone)]
pub enum MatchKind {
    /// Case-insensitive exact match against an alias.
    Exact,
    /// Fuzzy match (Damerau-Levenshtein similarity).
    Fuzzy {
        /// Similarity score (0.0–1.0).
        score: f64,
    },
}

/// Default fuzzy matching threshold (Damerau-Levenshtein normalized similarity).
const DEFAULT_FUZZY_THRESHOLD: f64 = 0.85;

/// Minimum similarity to report as an unresolved mention with a nearest suggestion.
const NEAREST_THRESHOLD: f64 = 0.5;

/// Minimum word length to consider for fuzzy matching.
const MIN_WORD_LEN: usize = 3;

/// Resolves free-text mentions against an ontology.
///
/// # Exact alias match
///
/// ```
/// use spectral_graph::ontology::Ontology;
/// use spectral_graph::canonicalize::Canonicalizer;
///
/// let ont = Ontology::from_toml(r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "alice"
/// aliases = ["Alice"]
/// visibility = "private"
/// "#).unwrap();
///
/// let c = Canonicalizer::new(&ont);
/// let result = c.canonicalize("Alice is here");
/// assert_eq!(result.matched.len(), 1);
/// assert_eq!(result.matched[0].canonical, "alice");
/// ```
///
/// # Fuzzy match
///
/// ```
/// use spectral_graph::ontology::Ontology;
/// use spectral_graph::canonicalize::{Canonicalizer, MatchKind};
///
/// let ont = Ontology::from_toml(r#"
/// version = 1
/// [[entity]]
/// type = "project"
/// canonical = "spectral"
/// aliases = ["Spectral"]
/// visibility = "public"
/// "#).unwrap();
///
/// let c = Canonicalizer::new(&ont);
/// // "Spectrl" is close enough (missing one char from 8)
/// let m = c.resolve_one("Spectrl").unwrap();
/// assert_eq!(m.canonical, "spectral");
/// assert!(matches!(m.match_kind, MatchKind::Fuzzy { .. }));
/// ```
///
/// # Empty text returns empty result
///
/// ```
/// use spectral_graph::ontology::Ontology;
/// use spectral_graph::canonicalize::Canonicalizer;
///
/// let ont = Ontology::from_toml("version = 1").unwrap();
/// let result = Canonicalizer::new(&ont).canonicalize("");
/// assert!(result.matched.is_empty());
/// assert!(result.unresolved.is_empty());
/// ```
///
/// # Unresolved mention with nearest suggestion
///
/// ```
/// use spectral_graph::ontology::Ontology;
/// use spectral_graph::canonicalize::Canonicalizer;
///
/// let ont = Ontology::from_toml(r#"
/// version = 1
/// [[entity]]
/// type = "person"
/// canonical = "sophie"
/// aliases = ["Sophie"]
/// visibility = "private"
/// "#).unwrap();
///
/// let c = Canonicalizer::new(&ont);
/// // "Saphie" is too far for a match but close enough for a suggestion
/// let result = c.canonicalize("Saphie is here");
/// assert_eq!(result.unresolved.len(), 1);
/// assert!(result.unresolved[0].nearest.is_some());
/// ```
#[derive(Debug)]
pub struct Canonicalizer<'a> {
    ontology: &'a Ontology,
    fuzzy_threshold: f64,
}

impl<'a> Canonicalizer<'a> {
    /// Create a new canonicalizer with the default fuzzy threshold (0.85).
    pub fn new(ontology: &'a Ontology) -> Self {
        Self {
            ontology,
            fuzzy_threshold: DEFAULT_FUZZY_THRESHOLD,
        }
    }

    /// Set a custom fuzzy matching threshold (0.0–1.0).
    pub fn with_fuzzy_threshold(mut self, threshold: f64) -> Self {
        self.fuzzy_threshold = threshold;
        self
    }

    /// Resolve all mentions in a text against the ontology.
    pub fn canonicalize(&self, text: &str) -> CanonicalizeResult {
        if text.is_empty() {
            return CanonicalizeResult {
                matched: vec![],
                unresolved: vec![],
            };
        }

        let aliases = self.build_alias_index();
        let words = extract_words(text);

        if words.is_empty() {
            return CanonicalizeResult {
                matched: vec![],
                unresolved: vec![],
            };
        }

        let max_alias_words = aliases.iter().map(|a| a.word_count).max().unwrap_or(1);
        let mut consumed = vec![false; words.len()];
        let mut matched = Vec::new();
        let mut unresolved = Vec::new();

        // Phase 1: exact matches, longest first
        for window_size in (1..=max_alias_words.min(words.len())).rev() {
            for start in 0..=words.len().saturating_sub(window_size) {
                if consumed[start..start + window_size].iter().any(|c| *c) {
                    continue;
                }
                let span_start = words[start].0;
                let span_end = words[start + window_size - 1].1;
                let candidate = &text[span_start..span_end];
                let candidate_lower = candidate.to_lowercase();

                if let Some(entry) = aliases.iter().find(|a| a.alias_lower == candidate_lower) {
                    let entity_id = self.ontology.entity_id_for(entry.entity);
                    matched.push(MatchedMention {
                        mention: candidate.to_string(),
                        span: (span_start, span_end),
                        entity_id,
                        entity_type: entry.entity.entity_type.clone(),
                        canonical: entry.entity.canonical.clone(),
                        match_kind: MatchKind::Exact,
                    });
                    for c in consumed.iter_mut().skip(start).take(window_size) {
                        *c = true;
                    }
                }
            }
        }

        // Phase 2: fuzzy matching on unconsumed words
        for (i, &(start, end)) in words.iter().enumerate() {
            if consumed[i] {
                continue;
            }
            let word = &text[start..end];
            if word.len() < MIN_WORD_LEN {
                continue;
            }

            let (best_score, best_entity) = self.best_fuzzy_match(word);

            if best_score >= self.fuzzy_threshold {
                let entity = best_entity.unwrap();
                let entity_id = self.ontology.entity_id_for(entity);
                matched.push(MatchedMention {
                    mention: word.to_string(),
                    span: (start, end),
                    entity_id,
                    entity_type: entity.entity_type.clone(),
                    canonical: entity.canonical.clone(),
                    match_kind: MatchKind::Fuzzy { score: best_score },
                });
            } else if best_score >= NEAREST_THRESHOLD {
                let entity = best_entity.unwrap();
                unresolved.push(UnresolvedMention {
                    mention: word.to_string(),
                    span: (start, end),
                    nearest: Some(NearestMatch {
                        canonical: entity.canonical.clone(),
                        entity_type: entity.entity_type.clone(),
                        score: best_score,
                    }),
                });
            }
            // Words with best_score < 0.5 are silently ignored (common words)
        }

        CanonicalizeResult {
            matched,
            unresolved,
        }
    }

    /// Resolve a single mention without span tracking.
    /// Returns None if no match meets the threshold.
    pub fn resolve_one(&self, mention: &str) -> Option<MatchedMention> {
        let mention_lower = mention.to_lowercase();

        // Try exact match first
        for entity in &self.ontology.entities {
            let all_aliases = std::iter::once(&entity.canonical).chain(entity.aliases.iter());
            for alias in all_aliases {
                if alias.to_lowercase() == mention_lower {
                    let entity_id = self.ontology.entity_id_for(entity);
                    return Some(MatchedMention {
                        mention: mention.to_string(),
                        span: (0, mention.len()),
                        entity_id,
                        entity_type: entity.entity_type.clone(),
                        canonical: entity.canonical.clone(),
                        match_kind: MatchKind::Exact,
                    });
                }
            }
        }

        // Try fuzzy match
        let (best_score, best_entity) = self.best_fuzzy_match(mention);
        if best_score >= self.fuzzy_threshold {
            let entity = best_entity.unwrap();
            let entity_id = self.ontology.entity_id_for(entity);
            return Some(MatchedMention {
                mention: mention.to_string(),
                span: (0, mention.len()),
                entity_id,
                entity_type: entity.entity_type.clone(),
                canonical: entity.canonical.clone(),
                match_kind: MatchKind::Fuzzy { score: best_score },
            });
        }

        None
    }

    /// Find the nearest match for a mention (for error reporting).
    pub fn find_nearest(&self, mention: &str) -> Option<NearestMatch> {
        let (best_score, best_entity) = self.best_fuzzy_match(mention);
        if best_score >= NEAREST_THRESHOLD {
            let entity = best_entity.unwrap();
            Some(NearestMatch {
                canonical: entity.canonical.clone(),
                entity_type: entity.entity_type.clone(),
                score: best_score,
            })
        } else {
            None
        }
    }

    fn best_fuzzy_match(&self, mention: &str) -> (f64, Option<&'a OntologyEntity>) {
        let mention_lower = mention.to_lowercase();
        let mut best_score = 0.0f64;
        let mut best_entity: Option<&'a OntologyEntity> = None;

        for entity in &self.ontology.entities {
            let all_aliases = std::iter::once(&entity.canonical).chain(entity.aliases.iter());
            for alias in all_aliases {
                let score =
                    strsim::normalized_damerau_levenshtein(&mention_lower, &alias.to_lowercase());
                if score > best_score {
                    best_score = score;
                    best_entity = Some(entity);
                }
            }
        }

        (best_score, best_entity)
    }

    fn build_alias_index(&self) -> Vec<AliasEntry<'a>> {
        let mut entries = Vec::new();
        for entity in &self.ontology.entities {
            // Canonical is implicitly an alias
            let canonical_lower = entity.canonical.to_lowercase();
            let word_count = canonical_lower.split_whitespace().count();
            entries.push(AliasEntry {
                alias_lower: canonical_lower,
                word_count,
                entity,
            });
            for alias in &entity.aliases {
                let alias_lower = alias.to_lowercase();
                let word_count = alias_lower.split_whitespace().count();
                entries.push(AliasEntry {
                    alias_lower,
                    word_count,
                    entity,
                });
            }
        }
        entries
    }
}

struct AliasEntry<'a> {
    alias_lower: String,
    word_count: usize,
    entity: &'a OntologyEntity,
}

/// Extract word boundaries from text. Returns (byte_start, byte_end) pairs.
fn extract_words(text: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '-' || c == '\'' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            words.push((s, i));
        }
    }
    if let Some(s) = start {
        words.push((s, text.len()));
    }
    words
}
