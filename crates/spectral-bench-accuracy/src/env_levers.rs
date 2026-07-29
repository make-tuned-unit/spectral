//! Bench-harness env → typed-config bridge.
//!
//! The `SPECTRAL_*` tuning levers used to be read inside library code
//! (spectral-graph / spectral-ingest), which made the tuning surface
//! undiscoverable and raced under parallel tests. They are now typed
//! [`BrainConfig`] fields; this module is the ONE place where the historical
//! env-var workflow is preserved, so every existing
//! `SPECTRAL_X=1 cargo run …` bench invocation keeps working identically.
//! The bench fingerprint already captures the environment, so A/B integrity
//! is unchanged.

use spectral_graph::brain::BrainConfig;

/// Apply the historical `SPECTRAL_*` env levers onto a typed [`BrainConfig`].
///
/// Mapping (env var → field, unset ⇒ field left as constructed):
///
/// | env var                          | field                   |
/// |----------------------------------|-------------------------|
/// | `SPECTRAL_READ_POOL_SIZE`        | `read_pool_size`        |
/// | `SPECTRAL_RECURRENCE_FEEDBACK`   | `recurrence_feedback`   |
/// | `SPECTRAL_FTS_FUSION`            | `fts_fusion`            |
/// | `SPECTRAL_FTS_TOKENIZER`         | `fts_tokenizer` (only if `None`) |
/// | `SPECTRAL_FTS_STOPWORDS`         | `fts_stopwords`         |
/// | `SPECTRAL_ANTICIPATORY_RECALL`   | `anticipatory_recall`   |
/// | `SPECTRAL_NUMBER_NORMALIZE`      | `number_normalize`      |
/// | `SPECTRAL_QUERY_ALIASES`         | `query_aliases_path`    |
/// | `SPECTRAL_MAX_FINGERPRINT_PEERS` | `max_fingerprint_peers` (`0` ⇒ unbounded) |
pub fn apply_env_levers(config: &mut BrainConfig) {
    apply_levers_from(config, |k| std::env::var(k).ok())
}

/// Inner mapping with an injectable env source, so the mapping itself is
/// testable without mutating process-global env (the race this migration
/// removes).
fn apply_levers_from(config: &mut BrainConfig, get: impl Fn(&str) -> Option<String>) {
    // Boolean levers: historically `v == "1" || v eq_ignore_ascii_case "true"`,
    // any other present value meant false.
    let flag = |v: String| v == "1" || v.eq_ignore_ascii_case("true");
    if let Some(v) = get("SPECTRAL_RECURRENCE_FEEDBACK") {
        config.recurrence_feedback = flag(v);
    }
    if let Some(v) = get("SPECTRAL_FTS_FUSION") {
        config.fts_fusion = flag(v);
    }
    if let Some(v) = get("SPECTRAL_FTS_STOPWORDS") {
        config.fts_stopwords = flag(v);
    }
    if let Some(v) = get("SPECTRAL_ANTICIPATORY_RECALL") {
        config.anticipatory_recall = flag(v);
    }
    if let Some(v) = get("SPECTRAL_NUMBER_NORMALIZE") {
        config.number_normalize = flag(v);
    }
    // Pool size: unparseable values fall back to the default (None).
    if let Some(v) = get("SPECTRAL_READ_POOL_SIZE") {
        config.read_pool_size = v.trim().parse::<usize>().ok();
    }
    // Tokenizer: env was historically a FALLBACK — explicit config won.
    if config.fts_tokenizer.is_none() {
        config.fts_tokenizer = get("SPECTRAL_FTS_TOKENIZER");
    }
    if let Some(p) = get("SPECTRAL_QUERY_ALIASES") {
        config.query_aliases_path = Some(std::path::PathBuf::from(p));
    }
    // Fingerprint peer cap: `0` restores unbounded; unparseable restores the
    // default cap — the historical mapping exactly.
    if let Some(v) = get("SPECTRAL_MAX_FINGERPRINT_PEERS") {
        config.max_fingerprint_peers = match v.trim().parse::<usize>() {
            Ok(0) => None,
            Ok(n) => Some(n),
            Err(_) => Some(spectral_ingest::ingest::DEFAULT_MAX_FINGERPRINT_PEERS),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(map: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            map.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn maps_every_env_lever_onto_its_typed_field() {
        let mut config = BrainConfig::default();
        apply_levers_from(
            &mut config,
            env(&[
                ("SPECTRAL_READ_POOL_SIZE", "7"),
                ("SPECTRAL_RECURRENCE_FEEDBACK", "1"),
                ("SPECTRAL_FTS_FUSION", "true"),
                ("SPECTRAL_FTS_TOKENIZER", "unicode61"),
                ("SPECTRAL_FTS_STOPWORDS", "1"),
                ("SPECTRAL_ANTICIPATORY_RECALL", "TRUE"),
                ("SPECTRAL_NUMBER_NORMALIZE", "1"),
                ("SPECTRAL_QUERY_ALIASES", "/tmp/aliases.json"),
                ("SPECTRAL_MAX_FINGERPRINT_PEERS", "128"),
            ]),
        );
        assert_eq!(config.read_pool_size, Some(7));
        assert!(config.recurrence_feedback);
        assert!(config.fts_fusion);
        assert_eq!(config.fts_tokenizer.as_deref(), Some("unicode61"));
        assert!(config.fts_stopwords);
        assert!(config.anticipatory_recall);
        assert!(config.number_normalize);
        assert_eq!(
            config.query_aliases_path.as_deref(),
            Some(std::path::Path::new("/tmp/aliases.json"))
        );
        assert_eq!(config.max_fingerprint_peers, Some(128));
    }

    #[test]
    fn unset_env_leaves_defaults_and_explicit_tokenizer_wins() {
        let mut config = BrainConfig {
            fts_tokenizer: Some("porter unicode61".into()),
            ..Default::default()
        };
        apply_levers_from(&mut config, env(&[("SPECTRAL_FTS_TOKENIZER", "unicode61")]));
        // Explicit config beat the env fallback, historically and now.
        assert_eq!(config.fts_tokenizer.as_deref(), Some("porter unicode61"));
        assert!(!config.fts_fusion);
        assert_eq!(config.read_pool_size, None);
        assert_eq!(
            config.max_fingerprint_peers,
            Some(spectral_ingest::ingest::DEFAULT_MAX_FINGERPRINT_PEERS)
        );
    }

    #[test]
    fn zero_peer_cap_means_unbounded() {
        let mut config = BrainConfig::default();
        apply_levers_from(&mut config, env(&[("SPECTRAL_MAX_FINGERPRINT_PEERS", "0")]));
        assert_eq!(config.max_fingerprint_peers, None);
    }

    #[test]
    fn public_fn_reads_process_env() {
        // One benign lever through the real env to cover the public seam.
        std::env::set_var("SPECTRAL_READ_POOL_SIZE", "3");
        let mut config = BrainConfig::default();
        apply_env_levers(&mut config);
        std::env::remove_var("SPECTRAL_READ_POOL_SIZE");
        assert_eq!(config.read_pool_size, Some(3));
    }
}
