# Spectrogram/Constellation audit — Shazam-first analysis

**Date:** 2026-07-02
**Question:** the spectrogram is core to Spectral's vision (inspiration: Shazam).
Is it useless, or just applied wrong?
**Verdict:** applied wrong, in a specific and fixable way. The subsystem is a
**recognition engine being asked to do recall**. Pointed at the right task and
fed content instead of metadata, the Shazam architecture is exactly right for
the "recognition" memory type — and no competitor has it.

## Measurements (real data, $0)

### Production brain (`~/.permagent/brain/memory.db`, read-only; 1,717 memories, 48 days of use)

| Metric | Value | Meaning |
|---|---|---|
| constellation_fingerprints rows | 390,523 | 227 rows per memory (quadratic peer-pairing) |
| **distinct fingerprint_hash values** | **442** | hash space is hall×hall×wing×bucket ≈ 1,900 possible |
| top single hash | 169,235 rows (43%) | a "match" is a category filter, not identification |
| constellation table size | 60 MB of 144 MB DB (42%) | most of the store spent on ~9 bits of information |
| memory_spectrogram rows | 0 | flag off in production too |

Shazam's discriminative power comes from **millions of distinct content-derived
hashes**, each matching a handful of tracks. 442 near-constant hashes cannot
identify anything.

### Dimension quality (bench brain, 497 real conversational memories, spectrograms on)

| Dimension | Distribution | Information |
|---|---|---|
| novelty | **1.0 for 497/497** | **broken** — `brain.rs:984` passes `AnalysisContext::default()`, wing corpus always empty |
| decision_polarity | gated on action=decision: 3/497 memories | ~none |
| causal_depth | 58% exactly 0.0, mean 0.10 | low |
| temporal_specificity | mean 0.08 | low |
| action_type | observation+advice = 86% | ~1 bit |
| entity_density | mean 0.44, real spread | some |
| emotional_valence | \|mean\| 0.43, real spread | some |

Effectively ~2.5 informative dimensions of 7. `find_resonant` requires
action_type equality + ≥3 of 6 dims within 0.3 tolerance — with zero-skewed
distributions, most same-action-type pairs trivially "resonate". Resonance
matching as shipped returns large undiscriminating sets. This is the measured
reason the Tier-0 oracle showed 0/500 retrieval effect.

## Shazam, element by element

| Shazam | What it's for | Spectral today | Gap |
|---|---|---|---|
| Spectrogram: time × frequency energy | robust signal representation | 7 scalar dims per memory | no time axis, no locality; a point, not a spectrogram; 4/7 dims degenerate |
| Peak picking → constellation map | sparse noise-robust landmarks **from the signal** | none — "constellation" fingerprints never touch content | the actual peaks (rare terms, entities, numbers) are never extracted |
| Pair hashing (peak₁, peak₂, Δt) | combinatorial specificity, millions of hashes | SHA-256(hall\|hall\|wing\|bucket), 442 distinct | hashes metadata categories; quadratic row blowup |
| Exact lookup + offset-histogram voting | pick THE track among millions | COUNT(*) of hash matches | no coherence voting; counting near-constant hashes ≈ popularity |
| Query = noisy fragment of the SAME signal | **recognition** | applied to **recall** (paraphrase = different signal) | category error — Shazam never solved semantic search |

The last row is the key insight. Shazam answers *"have I heard this exact
thing before, even degraded and partial?"* That is recognition memory — the
new memory type on the roadmap — not recall. FTS+ranking already beats the
constellation at recall (Tier-1 fingerprint search is starved and
outperformed); no tuning of the current design changes that, because the
architecture is answering a different question.

## The correct application: a content-derived recognition engine

Repoint the machinery, feeding it content instead of categories:

1. **Peaks** = salient content features: rare/IDF-heavy tokens, entities,
   numbers, error codes — scored against the brain's own corpus (deterministic,
   no LLM). This is what "local maxima above the noise floor" means for text.
2. **Pairs** = co-occurring peaks within a memory (and optionally across an
   episode window with a Δ-bucket) → hash(peakA, peakB, Δ). Thousands of
   distinct hashes per brain, ~5–20 rows per memory — replaces the quadratic
   390k-row/60MB table with a linear, higher-entropy one.
3. **Voting** = exact hash lookup, count aligned pairs per stored memory;
   episode-order coherence as the offset-histogram analog. Score = familiarity.
   Partial/noisy re-encounter works exactly as in Shazam: only a fraction of
   pairs must survive.
4. **Novelty = 1 − familiarity.** One mechanism serves both signals; fixes the
   broken dimension by construction. (Also fix `brain.rs:984` regardless —
   one-line: pass the wing corpus.)
5. **Dims that earned their keep** (entity_density, emotional_valence,
   action_type) can remain as cheap band features / tie-breakers; retire the
   degenerate four or fix and re-measure.

What this enables, natively and embedding-free: "have I seen this error
before?", "we already tried this approach", "this is the same person in a new
context", dedup with provenance ("matched because these exact features
aligned" — auditable in a way cosine similarity never is). Permagent is
already instrumented for it: its `recognition_events` table (170 rows, with
focus_wing and outcome linkage) and Spectral's `RecognitionContext` show the
product converging on this need from both sides.

Relationship to backlog T3 ("peak-pair fingerprinting", est. 1–2 weeks): this
is T3, with the target corrected — content peaks, recognition semantics, and
the constellation table replaced rather than augmented.

## Validation path (consistent with the Tier-0 discipline)

- Mechanism tests: synthetic only for adversarial cases (hard negatives:
  similar-but-novel must score low).
- Benefit: replay corpora — inject re-encounters (exact, paraphrased, partial)
  into LongMemEval sessions and the real Permagent workload (736 recall
  events, 48 days) — familiar-vs-novel AUC, plus cost/latency scaling curves.
  This doubles as the seed of the public recognition benchmark.
