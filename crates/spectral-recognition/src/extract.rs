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
    "a", "an", "the", "and", "or", "but", "if", "then", "else", "when", "at", "by", "for",
    "with", "about", "against", "between", "into", "through", "during", "before", "after",
    "above", "below", "to", "from", "up", "down", "in", "out", "on", "off", "over", "under",
    "again", "further", "once", "here", "there", "all", "any", "both", "each", "few", "more",
    "most", "other", "some", "such", "no", "nor", "not", "only", "own", "same", "so", "than",
    "too", "very", "can", "will", "just", "should", "now", "i", "me", "my", "we", "our", "you",
    "your", "he", "him", "his", "she", "her", "it", "its", "they", "them", "their", "what",
    "which", "who", "this", "that", "these", "those", "am", "is", "are", "was", "were", "be",
    "been", "being", "have", "has", "had", "having", "do", "does", "did", "doing", "of", "as",
    "because", "until", "while", "how", "why", "where",
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
    let all_caps = raw.len() >= 2 && raw.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
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

fn hash64(input: &str) -> u64 {
    let digest = Sha256::digest(input.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
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
    // Rank: anchors first, then longer keys, then earlier position.
    let mut ranked = candidates.clone();
    ranked.sort_by(|a, b| {
        b.anchor
            .cmp(&a.anchor)
            .then(b.key.len().cmp(&a.key.len()))
            .then(a.position.cmp(&b.position))
    });
    ranked.truncate(config.max_peaks);
    // Restore document order for pairing (gap buckets need positions).
    ranked.sort_by_key(|l| l.position);
    ranked
}

/// Generate all fingerprints for a stimulus: landmark pair hashes and
/// winnowed k-gram hashes.
pub fn fingerprint_stimulus(content: &str, config: &RecognitionConfig) -> StimulusPrints {
    let tokens = tokenize(content);
    let peaks = extract_landmarks(content, config);

    // Pair fingerprints: each peak pairs with subsequent peaks inside a
    // one-sided token window (the Shazam "target zone"). Robustness by
    // construction: token dropout only SHRINKS distances, so a surviving
    // pair always stays in-window — there is no bucket boundary to flip.
    // Hashing is ORDER-INSENSITIVE (canonical key order) so paraphrase
    // reordering still collides — Panako's lesson: coarse geometry survives
    // distortion; exact geometry does not.
    let mut pair_hashes = Vec::new();
    for (i, a) in peaks.iter().enumerate() {
        let mut taken = 0usize;
        for b in peaks.iter().skip(i + 1) {
            if b.position.saturating_sub(a.position) > config.pair_window {
                break;
            }
            if taken >= config.fan_out {
                break;
            }
            taken += 1;
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
        assert!(keys.contains(&"OOMKilled"), "mixed-case id preserved: {keys:?}");
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
        let shared = "the staging deploy failed with exit code 137 because the pod was OOMKilled today";
        let a = fingerprint_stimulus(&format!("prefix words here {shared}"), &cfg);
        let b = fingerprint_stimulus(&format!("{shared} and totally different tail content"), &cfg);
        let set_a: std::collections::HashSet<u64> = a.gram_hashes.iter().map(|(h, _)| *h).collect();
        assert!(
            b.gram_hashes.iter().any(|(h, _)| set_a.contains(h)),
            "shared verbatim run must produce a shared winnowed hash"
        );
    }
}
