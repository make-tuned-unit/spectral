//! Deterministic recognition memory for Spectral.
//!
//! Recall answers "what do I know about X?". Recognition answers **"have I
//! encountered this before — and what happened last time?"**. This crate
//! implements the query mode of the recognition engine (design:
//! `docs/internal/RECOGNITION_ENGINE_DESIGN.md`):
//!
//! 1. **Landmarks** — a stimulus's statistically salient features (rare
//!    stems, numbers, identifiers, entities), scored by IDF against the
//!    brain's own corpus. The text analog of spectral peaks above the noise
//!    floor.
//! 2. **Pair fingerprints** — Shazam-style combinatorial hashes of
//!    co-occurring landmarks with coarse gap buckets (Panako's lesson:
//!    coarse geometry survives rewording the way coarse time survives
//!    tempo shift).
//! 3. **Winnowed k-grams** — a second channel with the Schleimer/MOSS
//!    guarantee: any shared verbatim run of at least `w + k − 1` tokens is
//!    detected. Catches copy-paste re-encounters.
//! 4. **Scoring** — matched features are weighted by log-inverse corpus
//!    frequency (REM: rare matches are strong evidence of "old"), summed
//!    into per-trace odds; MINERVA 2's cubed echo aggregates vote shares
//!    into a corpus-level familiarity scalar even when no single trace
//!    dominates. Novelty = 1 − familiarity.
//!
//! No embeddings, no models, no LLM. Every verdict carries the exact
//! features that produced it.

mod extract;
mod score;
mod store;
pub mod stream;

pub use extract::{extract_landmarks, fingerprint_stimulus, Landmark, StimulusPrints};
pub use score::{score_candidates, ScoreConfig};
pub use store::{InMemoryRecognitionStore, RecognitionStore, SqliteRecognitionStore};
pub use stream::{
    centroid_of, make_cue, segment_stream, Centroid, CentroidConfig, CentroidTracker, Cue,
    Segment, StreamConfig, StreamEvent, StreamTracker,
};

use anyhow::Result;

/// Tunable parameters for the engine. Defaults follow the design doc.
#[derive(Debug, Clone)]
pub struct RecognitionConfig {
    /// Maximum landmarks (peaks) per stimulus/memory.
    pub max_peaks: usize,
    /// Pair fan-out: each peak pairs with at most F subsequent peaks.
    pub fan_out: usize,
    /// Target zone: peaks pair only within this token distance. One-sided —
    /// dropout shrinks distances, so surviving pairs never fall out.
    pub pair_window: usize,
    /// Winnowing k-gram size in tokens.
    pub kgram: usize,
    /// Winnowing window size. Guarantee: shared runs >= window + kgram - 1
    /// tokens are always detected.
    pub window: usize,
    /// Verdict thresholds and evidence weighting.
    pub score: ScoreConfig,
}

impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            max_peaks: 32,
            fan_out: 8,
            pair_window: 16,
            kgram: 5,
            window: 8,
            score: ScoreConfig::default(),
        }
    }
}

/// The verdict of a recognition query.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Verdict {
    /// A specific stored trace was recognized.
    Recognized { memory_id: String },
    /// The stimulus is familiar in aggregate but no single trace dominates
    /// (the dual-process "familiarity without recollection" signal).
    Familiar,
    /// Nothing like this has been seen before.
    Novel,
}

/// One piece of matched evidence — the audit trail of a verdict.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    /// The matched feature, human-readable (e.g. "pair: clerk~auth/near"
    /// or "run: 'the deploy failed with exit 137'").
    pub feature: String,
    /// Which stored memory it matched.
    pub memory_id: String,
    /// Evidence weight (log-inverse corpus frequency of the feature).
    pub weight: f64,
}

/// A candidate trace with its accumulated evidence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceMatch {
    pub memory_id: String,
    /// Rarity-weighted evidence sum (log-odds contribution).
    pub score: f64,
    /// Matched pair count.
    pub pair_hits: usize,
    /// Matched winnowed-gram count (verbatim-run signal).
    pub gram_hits: usize,
    /// Fraction of the stimulus's fingerprints this trace matched.
    pub coverage: f64,
}

/// Result of `recognize()`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecognitionResult {
    pub verdict: Verdict,
    /// Corpus-level familiarity in [0, 1] (MINERVA-style cubed echo over
    /// candidate vote shares, normalized).
    pub familiarity: f64,
    /// Log-odds that the stimulus is "old" for the best trace (REM-style).
    pub odds_of_old: f64,
    /// Novelty = 1 − familiarity. Replaces the spectrogram novelty dim.
    pub novelty: f64,
    /// Top candidate traces, strongest first.
    pub traces: Vec<TraceMatch>,
    /// The exact matched features behind the verdict (capped, strongest first).
    pub evidence: Vec<Evidence>,
    /// Stimulus stats for observability.
    pub stimulus_peaks: usize,
    pub stimulus_pairs: usize,
}

/// The recognition engine: extraction + index + scoring over a store.
pub struct RecognitionEngine<S: RecognitionStore> {
    store: S,
    config: RecognitionConfig,
}

impl<S: RecognitionStore> RecognitionEngine<S> {
    pub fn new(store: S, config: RecognitionConfig) -> Self {
        Self { store, config }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Enroll a memory: extract landmarks, index pair + gram fingerprints,
    /// update document-frequency counts. Idempotent per memory_id.
    pub fn enroll(&mut self, memory_id: &str, content: &str) -> Result<()> {
        if self.store.is_enrolled(memory_id)? {
            return Ok(());
        }
        let prints = fingerprint_stimulus(content, &self.config);
        self.store.index_memory(memory_id, &prints)?;
        Ok(())
    }

    /// Recognize a stimulus against everything enrolled.
    pub fn recognize(&self, stimulus: &str) -> Result<RecognitionResult> {
        let prints = fingerprint_stimulus(stimulus, &self.config);
        let pair_matches = self.store.lookup_pairs(&prints.pair_hashes)?;
        let gram_matches = self.store.lookup_grams(&prints.gram_hashes)?;
        let enrolled = self.store.enrolled_count()?;
        Ok(score_candidates(
            &prints,
            &pair_matches,
            &gram_matches,
            enrolled,
            &self.config.score,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> RecognitionEngine<InMemoryRecognitionStore> {
        RecognitionEngine::new(
            InMemoryRecognitionStore::default(),
            RecognitionConfig::default(),
        )
    }

    const CORPUS: &[(&str, &str)] = &[
        (
            "m-deploy",
            "The staging deploy failed with exit code 137 because the pod was OOMKilled during the migration step",
        ),
        (
            "m-auth",
            "Decided to use Clerk for authentication instead of rolling our own session management",
        ),
        (
            "m-grocery",
            "Planned the weekly grocery run: Costco for bulk items, saved about forty dollars splitting with neighbors",
        ),
        (
            "m-report",
            "Started the weekly status report for the Wealthie project covering bond structure progress",
        ),
    ];

    fn enrolled_engine() -> RecognitionEngine<InMemoryRecognitionStore> {
        let mut e = engine();
        for (id, content) in CORPUS {
            e.enroll(id, content).unwrap();
        }
        e
    }

    #[test]
    fn exact_reencounter_is_recognized() {
        let e = enrolled_engine();
        let r = e.recognize(CORPUS[0].1).unwrap();
        assert_eq!(
            r.verdict,
            Verdict::Recognized {
                memory_id: "m-deploy".into()
            },
            "exact re-encounter must be recognized; got {:?} familiarity={}",
            r.verdict,
            r.familiarity
        );
        assert!(!r.evidence.is_empty(), "verdict must carry evidence");
    }

    #[test]
    fn degraded_reencounter_is_recognized() {
        // The Shazam property: a partial, degraded fragment of the same
        // signal still locks. Drop ~40% of the content and reorder nothing.
        let e = enrolled_engine();
        let r = e
            .recognize("deploy failed exit code 137 pod OOMKilled")
            .unwrap();
        assert_eq!(
            r.verdict,
            Verdict::Recognized {
                memory_id: "m-deploy".into()
            },
            "degraded fragment must still lock; got {:?}",
            r.verdict
        );
    }

    #[test]
    fn paraphrase_shares_landmarks() {
        // Paraphrase keeps salient anchors (137, OOMKilled) even when
        // function words change. Should be at least Familiar.
        let e = enrolled_engine();
        let r = e
            .recognize("our pods got OOMKilled again — exit 137 on the deploy")
            .unwrap();
        assert_ne!(
            r.verdict,
            Verdict::Novel,
            "paraphrase sharing rare anchors must not read as novel; familiarity={}",
            r.familiarity
        );
    }

    #[test]
    fn hard_negative_is_novel() {
        // Same broad topic (kubernetes-ish ops) but a genuinely new event.
        let e = enrolled_engine();
        let r = e
            .recognize("Provisioned a brand new GPU node group for the training cluster in Frankfurt")
            .unwrap();
        assert_eq!(
            r.verdict,
            Verdict::Novel,
            "similar-but-new must be novel; got {:?} familiarity={}",
            r.verdict,
            r.familiarity
        );
        assert!(r.novelty > 0.8, "novelty should be high, got {}", r.novelty);
    }

    #[test]
    fn empty_store_is_novel() {
        let e = engine();
        let r = e.recognize("anything at all").unwrap();
        assert_eq!(r.verdict, Verdict::Novel);
        assert_eq!(r.familiarity, 0.0);
        assert_eq!(r.novelty, 1.0);
    }

    #[test]
    fn enroll_is_idempotent() {
        let mut e = enrolled_engine();
        e.enroll("m-deploy", CORPUS[0].1).unwrap();
        e.enroll("m-deploy", CORPUS[0].1).unwrap();
        let r = e.recognize(CORPUS[0].1).unwrap();
        // Double enrollment must not inflate evidence.
        assert_eq!(
            r.traces.iter().filter(|t| t.memory_id == "m-deploy").count(),
            1
        );
    }

    #[test]
    fn evidence_is_auditable() {
        let e = enrolled_engine();
        let r = e.recognize("exit code 137 OOMKilled migration").unwrap();
        // Every evidence row names a concrete feature and a real memory.
        for ev in &r.evidence {
            assert!(!ev.feature.is_empty());
            assert!(CORPUS.iter().any(|(id, _)| *id == ev.memory_id));
            assert!(ev.weight > 0.0);
        }
    }

    #[test]
    fn verbatim_run_detected_via_winnowing() {
        // A long verbatim quote inside otherwise-new text must register
        // gram hits (the MOSS guarantee).
        let e = enrolled_engine();
        let r = e
            .recognize(
                "Unrelated preamble text here. The staging deploy failed with exit code 137 because the pod was OOMKilled during the migration step. And some new trailing thoughts.",
            )
            .unwrap();
        let deploy = r
            .traces
            .iter()
            .find(|t| t.memory_id == "m-deploy")
            .expect("deploy trace present");
        assert!(
            deploy.gram_hits > 0,
            "verbatim run must produce winnowed-gram hits"
        );
    }
}
