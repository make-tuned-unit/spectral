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
//!
//! # Zero-inference guarantee (claim C3)
//!
//! With **default features** this crate has no network stack and no ML
//! runtime: `reqwest` is gated behind the off-by-default `paraphrase-gen`
//! feature (used only by the `paraphrase_gen` dev binary that mints a
//! pay-once paraphrase fixture) and `fastembed`/ONNX behind the
//! off-by-default `neural-baseline` feature (out-of-band embedding
//! baseline). Every recognition path — enroll, recognize, forget, stream —
//! is pure local computation (SHA-256 fingerprints + SQLite/in-memory
//! lookups). CI compiles default features, so the dependency gate is
//! enforced on every build, and `cargo tree -p spectral-recognition`
//! (default features) is the audit: no reqwest, no fastembed, no ort.

pub mod eval;
mod extract;
pub mod minhash;
mod score;
mod store;
pub mod stream;

pub use extract::{
    extract_landmarks, extract_landmarks_with, fingerprint_stimulus, fingerprint_stimulus_with,
    normalized_tokens, Landmark, MapIdf, StimulusPrints, TermIdf,
};
pub use minhash::MinHashConfig;
pub use score::{score_candidates, MinHashMatch, ScoreConfig};
pub use store::{InMemoryRecognitionStore, RecognitionStore, SqliteRecognitionStore};
pub use stream::{
    centroid_of, make_cue, segment_stream, Centroid, CentroidConfig, CentroidTracker, Cue, Segment,
    StreamConfig, StreamEvent, StreamTracker,
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
    /// MinHash lexical-similarity channel (widely-accepted near-duplicate
    /// sketch). Set `minhash.weight = 0.0` to disable.
    pub minhash: MinHashConfig,
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
            minhash: MinHashConfig::default(),
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
    /// Optional corpus rarity source for landmark selection (R9 seam).
    /// `None` — the default — is byte-identical to the length-proxy ranking.
    term_idf: Option<Box<dyn TermIdf + Send + Sync>>,
}

impl<S: RecognitionStore> RecognitionEngine<S> {
    pub fn new(store: S, config: RecognitionConfig) -> Self {
        Self {
            store,
            config,
            term_idf: None,
        }
    }

    /// Supply (or clear) a corpus rarity source. Affects landmark selection at
    /// both enroll and recognize time; enrolled fingerprints are NOT
    /// re-derived, so set this before enrolling for consistent extraction.
    pub fn set_term_idf(&mut self, idf: Option<Box<dyn TermIdf + Send + Sync>>) {
        self.term_idf = idf;
    }

    fn term_idf(&self) -> Option<&dyn TermIdf> {
        self.term_idf.as_deref().map(|i| i as &dyn TermIdf)
    }

    /// Mutable access to the backing store, for maintenance that is not
    /// enrolment itself — recording and clearing enrolment retries.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Enroll a memory: extract landmarks, index pair + gram fingerprints and
    /// the shingle-set (MinHash) channel, update document-frequency counts.
    /// Idempotent per memory_id.
    pub fn enroll(&mut self, memory_id: &str, content: &str) -> Result<()> {
        if self.store.is_enrolled(memory_id)? {
            return Ok(());
        }
        let prints = fingerprint_stimulus_with(content, &self.config, self.term_idf());
        self.store.index_memory(memory_id, &prints)?;
        // Shingle-set channel (best-effort — a store without MinHash support
        // or an older read-only index must not break enrollment). Inverted
        // shingle index: store the shingle SET (for containment scoring) keyed
        // by each of its shingles (blocking). A probe sharing ANY shingle
        // becomes a candidate — maximal recall, which matters for heavily
        // degraded re-encounters. (MinHash-LSH banding remains available in
        // `minhash` for larger-scale deployments.)
        if self.config.minhash.weight > 0.0 {
            let set = minhash::shingle_set(content, self.config.minhash.shingle);
            let _ = self.store.index_minhash(memory_id, &set, &set);
        }
        Ok(())
    }

    /// Enroll a memory from several separately-fingerprinted parts under ONE
    /// id — the union of each part's pair/gram fingerprints and shingles.
    ///
    /// Why this exists (R37/R39): enrolling `content + "\n" + description` as
    /// one text lets description tokens displace content peaks under
    /// `max_peaks`, which measurably hurts re-encounters of the content;
    /// enrolling the description as a second trace makes it its own memory's
    /// runner-up and trips the lead-margin rule. Fingerprinting each part on
    /// its own and indexing the union keeps the content trace intact and adds
    /// the description's features to the same identity. Idempotent per
    /// memory_id; an empty part list enrols nothing.
    pub fn enroll_parts(&mut self, memory_id: &str, parts: &[&str]) -> Result<()> {
        if self.store.is_enrolled(memory_id)? {
            return Ok(());
        }
        let mut merged = extract::StimulusPrints {
            peaks: Vec::new(),
            pair_hashes: Vec::new(),
            gram_hashes: Vec::new(),
            token_count: 0,
        };
        // One decision, read twice. Written as two separate comparisons this
        // was untestable: at zero weight each site alone is unobservable — the
        // first computes shingles nothing indexes, the second indexes an empty
        // set — while together they are observable. A single binding makes the
        // gate one thing that can be got wrong, and therefore one thing a test
        // can pin.
        let minhash_on = self.config.minhash.weight > 0.0;
        let mut shingles: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut any = false;
        for part in parts {
            if part.trim().is_empty() {
                continue;
            }
            any = true;
            let p = fingerprint_stimulus_with(part, &self.config, self.term_idf());
            merged.peaks.extend(p.peaks);
            merged.pair_hashes.extend(p.pair_hashes);
            merged.gram_hashes.extend(p.gram_hashes);
            merged.token_count += p.token_count;
            if minhash_on {
                shingles.extend(minhash::shingle_set(part, self.config.minhash.shingle));
            }
        }
        if !any {
            return Ok(());
        }
        // Dedup shared hashes across parts so a feature present in both is not
        // counted twice for the same memory.
        merged.pair_hashes.sort_by_key(|(h, _)| *h);
        merged.pair_hashes.dedup_by_key(|(h, _)| *h);
        merged.gram_hashes.sort_by_key(|(h, _)| *h);
        merged.gram_hashes.dedup_by_key(|(h, _)| *h);
        self.store.index_memory(memory_id, &merged)?;
        if minhash_on {
            let set: Vec<u64> = shingles.into_iter().collect();
            let _ = self.store.index_minhash(memory_id, &set, &set);
        }
        Ok(())
    }

    /// Forget a memory: remove all of its pair/gram fingerprints and its
    /// enrolled marker. After this, `recognize()` no longer surfaces the
    /// memory. Returns `true` if it was enrolled. This is the recognition
    /// half of hard delete / right-to-be-forgotten.
    pub fn forget(&mut self, memory_id: &str) -> Result<bool> {
        self.store.forget_memory(memory_id)
    }

    /// Recognize a stimulus against everything enrolled.
    pub fn recognize(&self, stimulus: &str) -> Result<RecognitionResult> {
        let prints = fingerprint_stimulus_with(stimulus, &self.config, self.term_idf());
        let pair_matches = self.store.lookup_pairs(&prints.pair_hashes)?;
        let gram_matches = self.store.lookup_grams(&prints.gram_hashes)?;
        let enrolled = self.store.enrolled_count()?;

        // MinHash channel: sketch the stimulus, find LSH band candidates, and
        // score each by CONTAINMENT (fraction of the probe's shingles present
        // in the candidate) — the re-encounter-appropriate similarity, high
        // even when the probe is a degraded fragment. Best-effort: a lookup
        // failure (e.g. an older index without MinHash tables) degrades to
        // pair+gram only.
        let minhash_matches = if self.config.minhash.weight > 0.0 {
            let probe_set = minhash::shingle_set(stimulus, self.config.minhash.shingle);
            match self.store.lookup_minhash(&probe_set) {
                Ok(cands) => cands
                    .into_iter()
                    .map(|(memory_id, cand_set)| MinHashMatch {
                        similarity: minhash::containment(&probe_set, &cand_set),
                        memory_id,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Ok(score_candidates(
            &prints,
            &pair_matches,
            &gram_matches,
            &minhash_matches,
            enrolled,
            &self.config.score,
            self.config.minhash.weight,
            self.config.minhash.min_similarity,
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

    // The POSITIVE case for `discriminative_margin` is tested at the
    // `score_candidates` level (see `score::tests`), not here: in a small
    // corpus rarity weighting alone already separates two template-sharing
    // memories, so an end-to-end test cannot honestly establish the
    // precondition it needs (a sub-margin tie). Constructing the tie requires
    // controlling document frequencies, which the scoring entry point exposes
    // and the engine does not. The SAFETY property below is testable
    // end-to-end and is the one that must hold whatever the corpus.

    /// R42 safety property: two memories with IDENTICAL content have no
    /// exclusive evidence, so the rule must never invent an identity for them
    /// — `Familiar` is the only honest verdict. This is what keeps the
    /// promotion from being a licence to guess.
    #[test]
    fn discriminative_margin_never_promotes_byte_identical_memories() {
        let content = "Started working in project Grocery Savers 019ea358b6d67671";
        let mut cfg = RecognitionConfig::default();
        cfg.score.discriminative_margin = true;
        let mut e = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
        e.enroll("twin_1", content).unwrap();
        e.enroll("twin_2", content).unwrap();
        let r = e.recognize(content).unwrap();
        assert!(
            matches!(r.verdict, Verdict::Familiar),
            "identical twins must stay Familiar, got {:?}",
            r.verdict
        );
    }

    /// `enroll_parts` indexes the UNION of separately-fingerprinted parts under
    /// one id: a probe shaped like either part resolves to that id, the store
    /// holds one enrolled memory (not two), and re-enrolling is a no-op.
    #[test]
    fn enroll_parts_indexes_union_under_one_id() {
        let mut e = engine();
        for (id, content) in CORPUS {
            e.enroll(id, content).unwrap();
        }
        let content = "The deploy of service atlas failed with exit code 137 after the memory limit was hit at 03:14 UTC";
        let desc = "atlas deploy failure: exit 137, memory limit, 03:14 UTC. Related terms: atlas, exit 137, OOM. Categories: deploys.";
        e.enroll_parts("m_parts", &[content, desc]).unwrap();
        assert_eq!(
            e.store().enrolled_count().unwrap(),
            CORPUS.len() + 1,
            "one memory, however many parts"
        );
        // content-shaped probe
        let r = e.recognize(content).unwrap();
        assert_eq!(
            r.traces.first().map(|t| t.memory_id.as_str()),
            Some("m_parts")
        );
        // description-shaped probe (a paraphrase re-encounter)
        let r = e
            .recognize("atlas deploy failure exit 137 memory limit")
            .unwrap();
        assert_eq!(
            r.traces.first().map(|t| t.memory_id.as_str()),
            Some("m_parts")
        );
        // idempotent
        e.enroll_parts("m_parts", &["something else entirely"])
            .unwrap();
        assert_eq!(e.store().enrolled_count().unwrap(), CORPUS.len() + 1);
        // empty parts enrol nothing
        e.enroll_parts("m_empty", &["", "   "]).unwrap();
        assert!(!e.store().is_enrolled("m_empty").unwrap());
    }

    /// The description part must add features WITHOUT displacing the content's:
    /// a content-shaped probe scores the memory at least as well under
    /// `enroll_parts` must honour the MinHash weight gate: the shingle channel
    /// is indexed when the channel is ON and left alone when it is OFF.
    ///
    /// Both directions are asserted because each catches a different way of
    /// breaking the comparison — a gate that fires at zero weight indexes a
    /// channel the caller disabled, and a gate that never fires silently drops
    /// the containment signal that carries degraded re-encounters. Neither is
    /// visible through a verdict, because pair and gram fingerprints alone are
    /// usually enough to name the memory.
    #[test]
    fn enroll_parts_indexes_the_shingle_channel_only_when_it_is_enabled() {
        let content = "The deploy of service atlas failed with exit code 137 at 03:14 UTC";
        let desc = "atlas deploy failure: exit 137, memory limit. Categories: deploys.";
        let probe = minhash::shingle_set(content, RecognitionConfig::default().minhash.shingle);

        // ON (the default weight): the shingle set is stored.
        let mut on = engine();
        on.enroll_parts("m", &[content, desc]).unwrap();
        assert!(
            on.store()
                .lookup_minhash(&probe)
                .unwrap()
                .iter()
                .any(|(id, _)| id == "m"),
            "with the channel enabled, enroll_parts must index the shingles"
        );

        // OFF (weight 0.0): nothing is stored for it.
        let mut cfg = RecognitionConfig::default();
        cfg.minhash.weight = 0.0;
        let mut off = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
        off.enroll_parts("m", &[content, desc]).unwrap();
        assert!(
            off.store().lookup_minhash(&probe).unwrap().is_empty(),
            "with the channel disabled, enroll_parts must not index shingles"
        );
    }

    /// `enroll_parts(content, desc)` as under `enroll(content)`.
    #[test]
    fn enroll_parts_does_not_lose_content_evidence() {
        let content = "The deploy of service atlas failed with exit code 137 after the memory limit was hit at 03:14 UTC";
        let desc = "atlas deploy failure: exit 137, memory limit, 03:14 UTC. Related terms: atlas, exit 137, OOM. Categories: deploys.";
        let probe = "deploy of service atlas failed with exit code 137 after the memory limit";
        let mut a = engine();
        a.enroll("m", content).unwrap();
        let mut b = engine();
        b.enroll_parts("m", &[content, desc]).unwrap();
        let sa = a
            .recognize(probe)
            .unwrap()
            .traces
            .first()
            .map(|t| t.pair_hits)
            .unwrap_or(0);
        let sb = b
            .recognize(probe)
            .unwrap()
            .traces
            .first()
            .map(|t| t.pair_hits)
            .unwrap_or(0);
        assert!(
            sb >= sa,
            "pair hits fell from {sa} to {sb} when a description part was added"
        );
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
            .recognize(
                "Provisioned a brand new GPU node group for the training cluster in Frankfurt",
            )
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
            r.traces
                .iter()
                .filter(|t| t.memory_id == "m-deploy")
                .count(),
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
