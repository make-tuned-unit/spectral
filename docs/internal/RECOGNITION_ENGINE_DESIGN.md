# Spectral Recognition Engine — design

**Date:** 2026-07-02
**Goal:** the third memory operation. Recall answers "what do I know about X?";
the graph answers "how does X relate to Y?"; **recognition answers "have I
encountered this before — and what happened last time?"** Both on-demand
(query mode, Shazam-style) and continuously over the ambient work stream
(stream mode, smart-TV-ACR-style), so Permagent can chime in unprompted.
Strictly deterministic, no embeddings, every match auditable.

Research base: Wang 2003 (Shazam), Six & Leman 2014 (Panako), Schleimer et
al. 2003 (winnowing/MOSS), Manku et al. 2007 (SimHash at Google), Hintzman
1984 (MINERVA 2), Shiffrin & Steyvers 1997 (REM), and the production ACR
patent family (Inscape/Vizio path pursuit, Sorenson scene matching) — all
claims 3-vote verified; full ACR notes below. Prior work is raw material:
no existing agent-memory product (Mem0, Zep, Letta) ships a familiarity/
novelty primitive or pattern-triggered proactive assistance.

## Architecture: one store, two matchers

```
                    ┌──────────────────────────────────┐
   remember() ────► │  Landmark extractor (peaks)      │
                    │  IDF vs own corpus, deterministic │
                    └──────┬───────────────┬───────────┘
                           │               │
              pair hashes  │               │  cue vectors (per ambient tick)
                           ▼               ▼
                 ┌─────────────────┐  ┌──────────────────┐
                 │ landmark_pairs  │  │ routine_cues      │
                 │ (inverted index)│  │ + lsh buckets     │
                 └────────┬────────┘  └────────┬─────────┘
                          │                    │
  recognize(stimulus) ────┘                    └──── observe(tick) [stream]
   → votes → odds-of-old                        → path-pursuit belief state
   → RecognitionResult{                         → edge-triggered events:
       familiarity, verdict,                      LOCK_ACQUIRED/LOST/TRANSFERRED
       evidence[], best_traces[]}                 → probe() → Permagent chime-in
```

## Query mode — "have I seen this?"

### Landmarks (the peaks)

A memory's landmarks are its statistically salient features, scored against
the brain's own corpus — the text analog of spectral peaks above the noise
floor:

- Tokens: lowercase, porter-stemmed (reuses the validated tokenizer work),
  stopwords dropped. Salience = IDF from a `df_counts` table maintained
  incrementally at write (`df[token] += 1` per distinct memory).
- Preserved verbatim (never stemmed): numbers, error codes, identifiers,
  Capitalized entity n-grams.
- Peak selection: top-P by IDF per memory with a density cap (P ≈ 12–24,
  like Shazam's peak-density control). Deterministic tie-break: lexicographic.

### Pair fingerprints

Shazam's combinatorial trick, applied to content: for each peak, pair with
the next F peaks in document order (F ≈ 5, Wang's fan-out): hash64(stemA ‖
stemB ‖ gap_bucket) where gap_bucket ∈ {adjacent, near, far} quantizes token
distance. ~P×F ≈ 60–120 pairs per memory, each high-entropy. Replaces the
current constellation table (390k rows → 442 distinct hashes in production;
this design yields distinct-hash counts near row counts, linear growth).

Panako's lesson for paraphrase robustness: store pair geometry coarsely
(gap *bucket*, not exact offset) so reworded spans still collide — the
text analog of tempo-shift tolerance.

Winnowing (MOSS) as a second, cheaper channel for verbatim/quotation
detection: k-gram hashes (k≈5 tokens) window-min selected (w≈8) with the
Schleimer guarantee — any shared run ≥ w+k−1 tokens is detected. This
catches copy-paste re-encounters that peak-pairs might under-weight.

### Scoring — familiarity as explicit odds (REM) + global echo (MINERVA 2)

1. Candidate votes: inverted-index lookup of the stimulus's pair hashes;
   count aligned pairs per stored memory. Order-coherence bonus when matched
   pairs appear in the same relative order (the offset-histogram analog).
2. Rarity weighting (REM's insight): a matched pair's evidence weight is
   its log-inverse corpus frequency — matching a rare pair is strong
   evidence of "old", matching a common one is weak. Sum = log-odds-of-old
   for the best trace. Auditable by construction: the evidence list IS the
   explanation.
3. Global familiarity (MINERVA 2 echo intensity): Σ over candidate traces of
   (normalized vote share)³ — cubing suppresses incidental matches and
   amplifies strong ones. This yields a corpus-level familiarity scalar even
   when no single trace locks ("this smells familiar but nothing specific"),
   which is precisely the dual-process familiarity-without-recollection
   signal from cognitive science.
4. Verdict thresholds (tuned on replay corpora, pre-registered):
   `Recognized(trace)` / `Familiar(no specific trace)` / `Novel`.
   Novelty = 1 − normalized familiarity — retires the broken spectrogram
   novelty dimension by construction.

## Stream mode — "the user is doing X again"

Production ACR's architecture, translated (see ACR research notes):

- **Weak cues at fixed cadence:** each ambient item (Permagent already feeds
  raw-compaction-tier memories) → a small fixed-schema integer vector:
  quantized hour-bucket, wing id, top peak-stem buckets, entity buckets,
  size/kind buckets. Deliberately low-information alone (Vizio's fingerprint
  is 13 averaged pixel patches); identity emerges from sequences, and FP
  rate decays geometrically with required sequence length.
- **Path pursuit:** belief state over (routine, offset) hypotheses in a
  `routine_suspects` table. Per cue: LSH-bucket candidate lookup, log-prob
  update (+bonus on match at expected offset, −small miss penalty — tolerate
  7-of-10), λ-decay with uniform re-injection (regime-change escape), floor
  pruning. Declare LOCK when top hypothesis clears θ AND leads runner-up by
  margin δ.
- **Edge-triggered events only:** LOCK_ACQUIRED / LOCK_LOST / LOCK_TRANSFERRED.
  A continuing match fires nothing — re-alert suppression is structural, not
  a filter. Lock-loss localizes the breaking event (which cue diverged) —
  itself chime-in material.
- **Segment-then-match:** consecutive-cue correlation below threshold marks a
  work "scene boundary" (context switch); segments summarized by centroid.
  Proactive suggestions fire at boundaries, never mid-scene.
- **Common-segment suppression:** cue subsequences occurring in ≥ k routines
  ("reading email", "running tests") are marked common; while the top
  hypothesis is common, stay multi-modal and suppress proactive action until
  flanking context disambiguates. This is the guard against the annoying-
  assistant failure mode.
- **Reference tiers:** rolling recent head (re-encounters of recent
  incidents, stricter thresholds) vs promoted routine catalog (a pattern
  graduates after n confirmed recurrences).
- **Consent surface (ACR's scar tissue — Vizio FTC settlement):** per-wing
  opt-in scopes, visible capture state, local-only fingerprints, and the
  audit trail the deterministic design provides natively.

## What no existing system does

Deterministic, auditable familiarity/novelty as a first-class memory
operation, plus ambient routine lock with edge-triggered proactive events.
Letta's "sleep-time compute" is background reorganization, not recognition;
Mem0/Zep/Graphiti are retrieval-only. The recognition benchmark (separate
doc, pre-registered, with an embedding-cosine baseline run honestly) is
downstream of this engine.

## Build phases

1. **Core (this branch):** `spectral-recognition` crate — df_counts, landmark
   extraction, pair fingerprints, winnowing channel, inverted index, REM
   odds + MINERVA echo scoring → `Brain::recognize(stimulus)` returning
   `RecognitionResult { verdict, familiarity, odds_of_old, evidence, traces }`.
   Storage in the existing memory.db. Spectrogram novelty dimension re-derived
   as 1 − familiarity (fixes the always-1.0 bug by replacement).
2. **Stream:** cue schema, path-pursuit tables, lock events; wire to
   `probe()`/`probe_recent()`; Permagent consumes via recognition_events.
3. **Validation (per the agreed pyramid, $0 first):** mechanism tests +
   adversarial hard negatives; degraded re-encounter replay (deterministic
   corruption of real Permagent/LongMemEval memories) → familiar-vs-novel
   AUC vs an honest cosine baseline; oracle recall-coupling check;
   Permagent outcome replay (170 labeled recognition_events).

## Validation gates (pre-registered targets)

- Degraded re-encounter (30% token dropout): AUC ≥ 0.95 (Shazam-analog task,
  should be near-perfect).
- Paraphrase re-encounter: AUC ≥ 0.80 (hard family; embeddings may win —
  report honestly, we compete on determinism/cost/auditability).
- Hard negatives (same-topic novel): FPR ≤ 5% at the Recognized threshold.
- Query latency: sub-millisecond at 10k memories; linear index growth.
