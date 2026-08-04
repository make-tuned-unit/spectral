//! Landmark extraction and fingerprint generation.
//!
//! Deterministic end to end: same input + same config → identical
//! fingerprints, on every platform. Hashes are the first 8 bytes of
//! SHA-256, matching the convention in `spectral-ingest::fingerprint`.

use crate::RecognitionConfig;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A salient feature of a stimulus.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    /// Normalized form used for hashing (stemmed token, or verbatim for
    /// numbers/identifiers/entities).
    pub key: String,
    /// Token position in the normalized token stream.
    pub position: usize,
    /// True if preserved verbatim (number, error code, identifier, entity).
    pub anchor: bool,
}

/// All fingerprints derived from one stimulus.
#[derive(Debug, Clone, Default)]
pub struct StimulusPrints {
    pub peaks: Vec<Landmark>,
    /// Pair fingerprint hashes with human-readable labels for evidence.
    pub pair_hashes: Vec<(u64, String)>,
    /// Winnowed k-gram hashes with the covered token text.
    pub gram_hashes: Vec<(u64, String)>,
    /// Total normalized tokens (for stats).
    pub token_count: usize,
}

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "if", "then", "else", "when", "at", "by", "for", "with",
    "about", "against", "between", "into", "through", "during", "before", "after", "above",
    "below", "to", "from", "up", "down", "in", "out", "on", "off", "over", "under", "again",
    "further", "once", "here", "there", "all", "any", "both", "each", "few", "more", "most",
    "other", "some", "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too",
    "very", "can", "will", "just", "should", "now", "i", "me", "my", "we", "our", "you", "your",
    "he", "him", "his", "she", "her", "it", "its", "they", "them", "their", "what", "which", "who",
    "this", "that", "these", "those", "am", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "having", "do", "does", "did", "doing", "of", "as", "because", "until",
    "while", "how", "why", "where",
];

fn is_stopword(t: &str) -> bool {
    STOPWORDS.contains(&t)
}

/// True for tokens preserved verbatim: numbers, error codes, mixed
/// alphanumerics, snake/kebab identifiers, ALL-CAPS terms, and mixed-case
/// identifiers (OOMKilled, iPhone — uppercase anywhere past position 0;
/// ordinary sentence-case words have it only at position 0).
fn is_anchor(raw: &str) -> bool {
    let has_digit = raw.chars().any(|c| c.is_ascii_digit());
    let has_sep = raw.contains('_') || raw.contains('-') || raw.contains('.');
    let all_caps = raw.len() >= 2
        && raw
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let inner_upper = raw.chars().skip(1).any(|c| c.is_ascii_uppercase());
    has_digit || (has_sep && raw.len() >= 4) || all_caps || inner_upper
}

/// Compact deterministic suffix stemmer (porter-style steps 1a/1b + common
/// derivational suffixes). Consistency matters more than linguistic
/// perfection: both enrollment and query pass through the same stemmer, so
/// under/over-stemming cancels out.
fn stem(t: &str) -> String {
    let mut s = t.to_string();
    // Step 1a: plurals
    if let Some(b) = s.strip_suffix("sses") {
        s = format!("{b}ss");
    } else if let Some(b) = s.strip_suffix("ies") {
        s = format!("{b}i");
    } else if s.len() > 4
        && (s.ends_with("ches") || s.ends_with("shes") || s.ends_with("xes") || s.ends_with("zes"))
    {
        s.truncate(s.len() - 2);
    } else if s.ends_with('s') && !s.ends_with("ss") && s.len() > 3 {
        s.pop();
    }
    // Step 1b: -ed / -ing (require a remaining vowel)
    let has_vowel = |x: &str| x.chars().any(|c| "aeiou".contains(c));
    for suf in ["ing", "ed"] {
        if let Some(b) = s.strip_suffix(suf) {
            if b.len() >= 3 && has_vowel(b) {
                s = b.to_string();
                break;
            }
        }
    }
    // Common derivational endings
    for (suf, rep) in [
        ("ization", "ize"),
        ("ational", "ate"),
        ("fulness", "ful"),
        ("ousness", "ous"),
        ("iveness", "ive"),
        ("tional", "tion"),
        ("alism", "al"),
        ("ment", ""),
        ("ness", ""),
    ] {
        if let Some(b) = s.strip_suffix(suf) {
            if b.len() >= 3 {
                s = format!("{b}{rep}");
                break;
            }
        }
    }
    s
}

/// Tokenize into normalized landmark candidates: (key, anchor, stopword)
/// per token, positions implicit by index. Stopword status is decided on
/// the raw lowercase form BEFORE stemming ("during" must not survive as
/// "dur"). Stopwords are excluded from landmark selection but retained in
/// the gram stream (verbatim runs include them).
fn tokenize(content: &str) -> Vec<(String, bool, bool)> {
    content
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .filter(|t| !t.is_empty())
        .map(|raw| {
            let trimmed = raw.trim_matches(|c: char| c == '.' || c == '-' || c == '_');
            if trimmed.is_empty() {
                (String::new(), false, false)
            } else if is_anchor(trimmed) {
                (trimmed.to_string(), true, false)
            } else {
                let lower = trimmed.to_lowercase();
                let stop = is_stopword(&lower);
                (stem(&lower), false, stop)
            }
        })
        .filter(|(k, _, _)| !k.is_empty())
        .collect()
}

/// The normalized token stream of a text — the exact keys the engine
/// fingerprints over (anchors verbatim, other tokens lowercased + stemmed,
/// stopwords retained for the gram channel).
///
/// Public as the **auditability seam (claim C2)**: every `Evidence` row a
/// verdict carries cites landmark keys ("pair: a~b") or a token run
/// ("run: '...'"); a third party can machine-verify the citation by checking
/// that those keys literally occur in `normalized_tokens(probe)` AND
/// `normalized_tokens(enrolled_content)`. Without this function the evidence
/// trail was human-readable but not independently checkable.
pub fn normalized_tokens(content: &str) -> Vec<String> {
    tokenize(content).into_iter().map(|(k, _, _)| k).collect()
}

fn hash64(input: &str) -> u64 {
    let digest = Sha256::digest(input.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// A source of corpus term rarity.
///
/// Landmark salience currently ranks by token **length** as a proxy for IDF.
/// Measured on a real 1,981-document brain: Spearman(length, IDF) = **0.275**,
/// and length-ranked top-8 landmarks overlap IDF-ranked ones only **51%** of
/// the time. For an engine that selects "statistically salient features", that
/// is the wrong half.
///
/// True per-term document frequency is available at zero extra storage from
/// SQLite's `fts5vocab` module over `memories_fts`. This trait is the seam.
pub trait TermIdf {
    /// Documents containing `term`, or `None` if unknown to the corpus.
    fn doc_frequency(&self, term: &str) -> Option<u32>;
    /// Total documents in the corpus.
    fn total_docs(&self) -> u32;

    /// Inverse document frequency; `None` for unknown terms so callers can
    /// fall back rather than treat unseen as maximally rare.
    fn idf(&self, term: &str) -> Option<f64> {
        let df = self.doc_frequency(term)?.max(1) as f64;
        Some((self.total_docs().max(1) as f64 / df).ln())
    }
}

/// A `TermIdf` backed by an in-memory map — for tests, benchmarks, and callers
/// that already hold corpus statistics.
#[derive(Debug, Clone, Default)]
pub struct MapIdf {
    df: HashMap<String, u32>,
    total: u32,
}

impl MapIdf {
    pub fn new(df: HashMap<String, u32>, total: u32) -> Self {
        Self { df, total }
    }

    /// Build document frequencies by tokenizing a corpus the same way
    /// extraction does, so the statistics match the tokens being ranked.
    pub fn from_corpus<'a>(docs: impl IntoIterator<Item = &'a str>) -> Self {
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut total = 0u32;
        for d in docs {
            total += 1;
            let mut seen = std::collections::HashSet::new();
            for k in normalized_tokens(d) {
                if seen.insert(k.clone()) {
                    *df.entry(k).or_insert(0) += 1;
                }
            }
        }
        Self { df, total }
    }
}

impl TermIdf for MapIdf {
    fn doc_frequency(&self, term: &str) -> Option<u32> {
        self.df.get(term).copied()
    }
    fn total_docs(&self) -> u32 {
        self.total
    }
}

/// Select up to `max_peaks` landmarks by salience.
///
/// Salience is corpus-free at extraction time: anchors first (numbers,
/// identifiers, entities are always salient), then rarer-looking tokens by
/// length (a cheap monotone proxy for IDF that keeps extraction free of
/// store reads; corpus rarity enters at SCORING time via document
/// frequencies, where it belongs — REM weights evidence, not extraction).
/// Deterministic tie-break: position order.
pub fn extract_landmarks(content: &str, config: &RecognitionConfig) -> Vec<Landmark> {
    extract_landmarks_with(content, config, None)
}

/// [`extract_landmarks`] with an optional corpus rarity source.
///
/// `None` reproduces the length-proxy ranking exactly. With an IDF source,
/// anchors still rank first (numbers and identifiers are salient regardless of
/// corpus), then genuinely rare terms, then length for terms the corpus has
/// never seen.
pub fn extract_landmarks_with(
    content: &str,
    config: &RecognitionConfig,
    idf: Option<&dyn TermIdf>,
) -> Vec<Landmark> {
    let tokens = tokenize(content);
    let mut seen = HashMap::new();
    let mut candidates: Vec<Landmark> = Vec::new();
    for (pos, (key, anchor, stop)) in tokens.iter().enumerate() {
        if !anchor && (*stop || key.len() < 3) {
            continue;
        }
        // First occurrence only — repeated tokens add no pairing value.
        if seen.insert(key.clone(), ()).is_some() {
            continue;
        }
        candidates.push(Landmark {
            key: key.clone(),
            position: pos,
            anchor: *anchor,
        });
    }
    // Rank: anchors first, then rarity, then earlier position.
    let mut ranked = candidates.clone();
    match idf {
        // True corpus rarity. Unknown terms fall back to the length proxy so a
        // term the corpus has not seen is not silently treated as common.
        Some(src) => ranked.sort_by(|a, b| {
            let key = |l: &Landmark| src.idf(&l.key).unwrap_or(f64::INFINITY).min(f64::MAX);
            let (ka, kb) = (key(a), key(b));
            b.anchor
                .cmp(&a.anchor)
                .then(kb.partial_cmp(&ka).unwrap_or(std::cmp::Ordering::Equal))
                .then(b.key.len().cmp(&a.key.len()))
                .then(a.position.cmp(&b.position))
        }),
        None => ranked.sort_by(|a, b| {
            b.anchor
                .cmp(&a.anchor)
                .then(b.key.len().cmp(&a.key.len()))
                .then(a.position.cmp(&b.position))
        }),
    }
    ranked.truncate(config.max_peaks);
    // Restore document order for pairing (gap buckets need positions).
    ranked.sort_by_key(|l| l.position);
    ranked
}

/// Generate all fingerprints for a stimulus: landmark pair hashes and
/// winnowed k-gram hashes.
pub fn fingerprint_stimulus(content: &str, config: &RecognitionConfig) -> StimulusPrints {
    fingerprint_stimulus_with(content, config, None)
}

/// [`fingerprint_stimulus`] with an optional corpus rarity source for landmark
/// selection. `None` is byte-identical to [`fingerprint_stimulus`].
pub fn fingerprint_stimulus_with(
    content: &str,
    config: &RecognitionConfig,
    idf: Option<&dyn TermIdf>,
) -> StimulusPrints {
    let tokens = tokenize(content);
    let peaks = extract_landmarks_with(content, config, idf);

    // Pair fingerprints: each peak pairs with subsequent peaks inside a
    // one-sided token window (the Shazam "target zone"). Robustness by
    // construction: token dropout only SHRINKS distances, so a surviving
    // pair always stays in-window — there is no bucket boundary to flip.
    // Hashing is ORDER-INSENSITIVE (canonical key order) so paraphrase
    // reordering still collides — Panako's lesson: coarse geometry survives
    // distortion; exact geometry does not.
    let mut pair_hashes = Vec::new();
    for (i, a) in peaks.iter().enumerate() {
        // Pair with the next peaks within the target window, up to fan_out.
        // Peaks are position-ordered, so take_while stops at the window edge.
        let in_window = peaks
            .iter()
            .skip(i + 1)
            .take_while(|b| b.position.saturating_sub(a.position) <= config.pair_window)
            .take(config.fan_out);
        for b in in_window {
            let (lo, hi) = if a.key <= b.key {
                (a.key.as_str(), b.key.as_str())
            } else {
                (b.key.as_str(), a.key.as_str())
            };
            let label = format!("pair: {lo}~{hi}");
            let h = hash64(&format!("rp1|{lo}|{hi}"));
            pair_hashes.push((h, label));
        }
    }

    // Winnowing channel over the full token stream (stopwords included —
    // verbatim runs are verbatim). Schleimer guarantee: any shared run of
    // >= window + kgram - 1 tokens produces at least one selected hash.
    let keys: Vec<&str> = tokens.iter().map(|(k, _, _)| k.as_str()).collect();
    let mut gram_hashes: Vec<(u64, String)> = Vec::new();
    if keys.len() >= config.kgram {
        let grams: Vec<(u64, String)> = keys
            .windows(config.kgram)
            .map(|w| {
                let text = w.join(" ");
                (hash64(&format!("rg1|{text}")), format!("run: '{text}'"))
            })
            .collect();
        if grams.len() <= config.window {
            if let Some(min) = grams.iter().min_by_key(|(h, _)| *h) {
                gram_hashes.push(min.clone());
            }
        } else {
            let mut last_pushed: Option<u64> = None;
            for w in grams.windows(config.window) {
                let min = w.iter().min_by_key(|(h, _)| *h).expect("nonempty");
                if last_pushed != Some(min.0) {
                    gram_hashes.push(min.clone());
                    last_pushed = Some(min.0);
                }
            }
        }
    }

    StimulusPrints {
        peaks,
        pair_hashes,
        gram_hashes,
        token_count: tokens.len(),
    }
}

#[cfg(test)]
mod idf_tests {
    use super::*;
    use crate::RecognitionConfig;

    /// No IDF source must reproduce the length-proxy ranking byte for byte.
    #[test]
    fn none_reproduces_length_ranking() {
        let cfg = RecognitionConfig::default();
        let text = "the pod was OOMKilled at 512Mi during reindex characteristically";
        let a = extract_landmarks(text, &cfg);
        let b = extract_landmarks_with(text, &cfg, None);
        assert_eq!(
            a.iter().map(|l| &l.key).collect::<Vec<_>>(),
            b.iter().map(|l| &l.key).collect::<Vec<_>>()
        );
    }

    /// A long common word must lose to a shorter rare one once real corpus
    /// rarity is available — the exact case the length proxy gets wrong.
    #[test]
    fn true_rarity_beats_length() {
        let cfg = RecognitionConfig {
            max_peaks: 1,
            ..RecognitionConfig::default()
        };
        // "characteristically" is long and everywhere; "kafka" is short and rare.
        let corpus: Vec<&str> = std::iter::repeat_n("characteristically speaking", 50)
            .chain(std::iter::once("kafka"))
            .collect();
        let idf = MapIdf::from_corpus(corpus);

        let text = "characteristically kafka";
        let by_length = extract_landmarks_with(text, &cfg, None);
        assert_eq!(
            by_length[0].key, "characteristically",
            "length proxy baseline"
        );

        let by_idf = extract_landmarks_with(text, &cfg, Some(&idf));
        assert_eq!(by_idf[0].key, "kafka", "true rarity should win");
    }

    /// Terms the corpus has never seen fall back to length rather than being
    /// treated as maximally rare or maximally common.
    #[test]
    fn unknown_terms_fall_back_to_length() {
        let cfg = RecognitionConfig::default();
        let idf = MapIdf::from_corpus(vec!["completely unrelated corpus content"]);
        let text = "zzzzlongunknown zzshort";
        let out = extract_landmarks_with(text, &cfg, Some(&idf));
        assert_eq!(out.first().map(|l| l.key.as_str()), Some("zzzzlongunknown"));
    }

    #[test]
    fn idf_is_none_for_unknown_and_finite_for_known() {
        let idf = MapIdf::from_corpus(vec!["alpha beta", "alpha gamma"]);
        assert!(idf.idf("alpha").unwrap() < idf.idf("beta").unwrap());
        assert!(idf.idf("nevermentioned").is_none());
        assert_eq!(idf.total_docs(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RecognitionConfig {
        RecognitionConfig::default()
    }

    #[test]
    fn anchors_are_preserved_verbatim() {
        let peaks = extract_landmarks(
            "the deploy failed with exit code 137 and OOMKilled in us-east-1",
            &config(),
        );
        let keys: Vec<&str> = peaks.iter().map(|l| l.key.as_str()).collect();
        assert!(keys.contains(&"137"), "number preserved: {keys:?}");
        assert!(
            keys.contains(&"OOMKilled"),
            "mixed-case id preserved: {keys:?}"
        );
        assert!(keys.contains(&"us-east-1"), "kebab id preserved: {keys:?}");
    }

    #[test]
    fn stopwords_never_become_peaks() {
        let peaks = extract_landmarks("the and with about during", &config());
        assert!(peaks.is_empty());
    }

    #[test]
    fn stemming_bridges_plural_and_gerund() {
        assert_eq!(stem("doctors"), stem("doctor"));
        assert_eq!(stem("boxes"), stem("box"));
        assert_eq!(stem("wishes"), stem("wish"));
        assert_eq!(stem("churches"), stem("church"));
        assert_eq!(stem("deploying"), stem("deploy"));
        assert_eq!(stem("deployed"), stem("deploy"));
    }

    #[test]
    fn fingerprints_are_deterministic() {
        let a = fingerprint_stimulus("The deploy failed with exit 137", &config());
        let b = fingerprint_stimulus("The deploy failed with exit 137", &config());
        assert_eq!(a.pair_hashes, b.pair_hashes);
        assert_eq!(a.gram_hashes, b.gram_hashes);
    }

    #[test]
    fn pair_count_bounded_by_peaks_times_fanout() {
        let cfg = config();
        let p = fingerprint_stimulus(
            "alpha bravo charlie delta echo foxtrot golf hotel india juliett kilo lima mike november oscar papa quebec romeo sierra tango",
            &cfg,
        );
        assert!(p.pair_hashes.len() <= cfg.max_peaks * cfg.fan_out);
        assert!(!p.pair_hashes.is_empty());
    }

    #[test]
    fn winnowing_selects_shared_hash_for_shared_run() {
        // Two texts sharing a long verbatim run must share at least one
        // selected gram hash (the MOSS guarantee, window+kgram-1 = 12).
        let cfg = config();
        let shared =
            "the staging deploy failed with exit code 137 because the pod was OOMKilled today";
        let a = fingerprint_stimulus(&format!("prefix words here {shared}"), &cfg);
        let b = fingerprint_stimulus(
            &format!("{shared} and totally different tail content"),
            &cfg,
        );
        let set_a: std::collections::HashSet<u64> = a.gram_hashes.iter().map(|(h, _)| *h).collect();
        assert!(
            b.gram_hashes.iter().any(|(h, _)| set_a.contains(h)),
            "shared verbatim run must produce a shared winnowed hash"
        );
    }
}
