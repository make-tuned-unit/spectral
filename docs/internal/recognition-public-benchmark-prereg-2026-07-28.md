# PRE-REGISTRATION — public recognition benchmark (2026-07-28)

**This document is committed BEFORE the harness is built or any number is
measured.** Git history is the proof of ordering. Nothing below may be edited
after results exist; deviations get a dated addendum, never a rewrite.

## Why this exists

The recognition engine's private-brain baseline matrix
(`RECOGNITION_BASELINE.md`) is honest but not externally verifiable — it runs
on the live Permagent database. A skeptic cannot reproduce it. This benchmark
re-derives the trade-off surface on **public data** with **pre-registered
predictions**, including predictions of the engine's own failure modes.

## Claims under test (the only claims we make)

- **C1 — Determinism.** Same content → byte-identical verdict and scalar,
  regardless of run count, platform, or store insertion order.
- **C2 — Auditability.** Every non-Novel verdict carries machine-checkable
  evidence: the matched landmarks exist verbatim in both probe and enrolled
  content. No verdict without sufficient evidence. (Structural invariant,
  proven by tests, not benchmarks.)
- **C3 — Zero inference.** No embeddings, no LLM, no network on any
  recognition path. Proven at build level (dependency audit) and run level
  (token-cost assertion).
- **C4 — A published trade-off surface**, per-regime, against the baselines
  that beat us, on public data anyone can re-run. We claim competitiveness
  and auditability at $0 — we explicitly do NOT claim best-in-regime accuracy
  (withdrawn 2026-07-03, `RECOGNITION_BASELINE.md`).

## Datasets (public, fixed before measurement)

- **R1 — Lexical re-encounter:** LongMemEval-S user turns ≥ 60 chars;
  enrolled = deterministic 90/10 split by `hash_id % 10` (same protocol as the
  private benchmark); positives = enrolled content with 30% deterministic
  token dropout; negatives = held-out turns. Public dataset, deterministic
  derivation — fully reproducible from `longmemeval_s.json`.
- **R2 — Semantic re-encounter (paraphrase):** MRPC test split, label=1 pairs
  (genuine paraphrases): enroll sentence A, probe with sentence B.
  Negatives: label=1 B-sides probed against a store WITHOUT their A-side.
- **R3 — Adversarial near-miss (the hard one):** PAWS-Wiki labeled test split.
  Label=0 pairs (high lexical overlap, DIFFERENT meaning): enroll A, probe B —
  these SHOULD read as Novel; a familiar verdict is a false positive.
  Label=1 pairs as the paired positives. PAWS is constructed specifically to
  defeat lexical-overlap methods — this is the regime designed to break us.

## Systems (identical splits, identical scoring)

1. Peak-pair engine (this crate, query mode)
2. MinHash-128 (in-crate classical baseline — the lexical champion)
3. BGE-small-en-v1.5 max-cosine (embedding baseline, local ONNX/fastembed —
   the semantic champion)

Metric: clean ROC-AUC (Mann–Whitney, ties half credit), same `eval.rs`
implementation for all systems. Per-regime, no averaging across regimes
(a blended number would hide exactly what honesty requires showing).

## Pre-registered predictions

Derived from the private-brain matrix; deviations beyond the stated bands are
findings, not embarrassments — they get reported either way.

| regime | engine | MinHash-128 | BGE-small | prediction basis |
|---|---|---|---|---|
| R1 lexical | 0.90–0.97 | **0.98–1.00** | 0.80–0.90 | private: 0.941 / 0.998 / 0.866 |
| R2 semantic | 0.50–0.62 | 0.40–0.52 | **0.65–0.78** | private: 0.543 / 0.453 / 0.703 |
| R3 PAWS adversarial | **0.45–0.65** | **0.40–0.60** | 0.55–0.75 | no prior measurement — see below |

**Pre-registered failure-mode prediction (R3):** PAWS label-0 pairs share high
lexical overlap by construction, so BOTH lexical systems (engine AND MinHash)
are predicted to false-positive heavily — AUC near or modestly above chance.
The engine's rare-anchor weighting may buy a few points over raw shingles; it
will NOT reach embedding performance. **We are predicting our own weak regime
quantitatively before measuring it.** If the engine exceeds 0.75 on R3, that
is a surprising positive requiring adversarial re-verification before belief
(same discipline as the RERANK n=30 lesson).

**Verdict-level prediction (R2/R3):** at the verdict layer (not the scalar),
≤ 5% of R2 true paraphrases read as Novel (private measurement: 1.1%), at the
cost of a false-familiar rate on R3 label-0 that we will report but do not
predict (no prior).

## Decision rules (fixed now)

- Results within bands → publish the table as-is in a public
  `docs/RECOGNITION_RESULTS.md`, losses included, linked from README.
- Engine below band in R1 (< 0.90) → investigate before publishing; if real,
  publish WITH the finding. Nothing is quietly shelved.
- Any system above band → re-verify (leakage check: split hygiene, near-dup
  contamination between enroll/probe sets) before publication.
- The claims-drift CI gate (C4 enforcement) lands with the results: every
  number cited in public docs must match the committed results JSON or the
  build fails.

## Out of scope (recorded so scope can't silently grow)

Production verdict value (Permagent's 170 labeled recognition events) is
Tier C — blocked on negative-outcome emission; separately pre-registered when
unblocked. Stream mode is not under test here (wing-label ground truth is
saturated; same blocker).

---

## ADDENDUM — outcomes vs. pre-registered bands (2026-07-29, post-measurement)

Per the rules above, this is an addendum; nothing above this line was edited.

| regime | system | band | measured | outcome |
|---|---|---|---|---|
| R1 | engine | 0.90–0.97 | 0.9946 | **above band** |
| R1 | MinHash | 0.98–1.00 | 0.9988 | in band |
| R1 | BGE | 0.80–0.90 | 0.8497 | in band |
| R2 | engine | 0.50–0.62 | 0.9788 | **above band** |
| R2 | MinHash | 0.40–0.52 | 0.9668 | **above band** |
| R2 | BGE | 0.65–0.78 | 0.9812 | **above band** |
| R3 | engine | 0.45–0.65 | 0.4875 | in band ✓ (predicted failure confirmed) |
| R3 | MinHash | 0.40–0.60 | 0.4917 | in band ✓ |
| R3 | BGE | 0.55–0.75 | 0.4853 | **below band — prediction MISS** |

Re-verification findings (required by the decision rules for out-of-band
results, run before publication):

- **R2 (all systems above band):** MRPC label=1 pairs have median content-word
  Jaccard **0.55** — the bands were calibrated on vocabulary-disjoint Haiku
  paraphrases; MRPC is a substantially lexical test. Dataset character, not
  leakage. Published as such; R2 is demoted to an easy-paraphrase ceiling
  check in the results doc.
- **R1 (engine slightly above band):** LongMemEval turns are longer than the
  private-brain memories the band came from; 30% dropout on longer texts
  preserves more landmarks per probe. Direction and ordering vs MinHash match
  the private measurement exactly (MinHash > engine > BGE).
- **R3 (BGE below band): the substantive prediction miss.** We predicted
  embeddings would beat the lexical methods on adversarial paraphrases; BGE
  is at chance with them (PAWS's matched-overlap construction defeats
  off-the-shelf cosine similarity — consistent with PAWS's published purpose).
  Reported as a miss. Consequence: R3 is nobody's regime at $0 or otherwise
  without task-tuned models; the engine's conservative false-familiar verdict
  behavior (0 missed re-encounters, 100% false-familiar on adversarial pairs)
  is the operative fact for consumers.

Verdict-level prediction check: ≤5% of true paraphrases read Novel —
measured **0%** in both R2 and R3 (pos_novel=0 everywhere). Confirmed.
