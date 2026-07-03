# Recognition engine — baselines and validation

**Date:** 2026-07-02
**Cost:** $0 (the neural baseline runs a local model; no API calls)

All numbers below are on the **live Permagent brain** (`~/.permagent/brain/memory.db`,
1616 memories ≥ 60 chars), under one shared harness (`src/eval.rs`) so every
method scores the identical split, positives, negatives, and label-noise mask.
The only variable across rows is the familiarity scalar.

## Protocol (honest by construction)

- Deterministic 90/10 split by `hash_id(id) % 10` → 1462 enrolled, 154 held-out.
- **Positives:** enrolled memories with ~30% deterministic token dropout (the
  Shazam noisy-fragment condition) — 1392 after the ≥5-token gate.
- **Negatives:** the 154 held-out memories — true novels from the *same*
  distribution (far harder than off-topic negatives).
- **Label noise:** 44 of 154 held-out items are genuine near-duplicates of an
  enrolled memory (token Jaccard ≥ 0.5) — the "novel" label is wrong, so they
  are excluded from the clean AUC. 110 clean negatives remain.
- Score = rank-statistic AUC (ties at half credit).

## Head-to-head: deterministic peak-pairs vs neural embeddings

| method | clean AUC | latency/query | model |
|---|---|---|---|
| **peak-pair engine (this crate)** | **0.9493** | ~1.5 ms | none |
| BGE-small-en-v1.5, max cosine | 0.8658 | ~495 ms | 130 MB ONNX |

The deterministic engine **beats** a standard dense-embedding retrieval by
**+8.4 points of clean AUC at ~330× lower latency**, model-free and fully
auditable (every verdict carries the exact matched features).

**Why embeddings lose here.** The task is "have I encountered *this*?", not
"what is topically near this?". Degraded positives shed distinctive tokens but
keep their rare anchors (identifiers, numbers, error codes); same-corpus
held-out negatives are topically close and so score high cosine. Dense
embeddings reward topical proximity — exactly the wrong axis. Peak-pairs key on
rare-anchor co-occurrence, which survives degradation and is absent from
topical near-misses. This is the recurrence thesis measured against the
strongest cheap alternative.

Reproduce:

```bash
# deterministic
cargo run --release -p spectral-recognition --bin replay -- \
  --db ~/.permagent/brain/memory.db
# neural baseline (first run downloads BGE-small)
cargo run --release -p spectral-recognition --example embedding_baseline -- \
  --db ~/.permagent/brain/memory.db
```

## Paraphrase family — the hard case, resolved at the verdict level

Positives are Haiku paraphrases (same facts, different words) of enrolled
memories; negatives as above.

| metric | value |
|---|---|
| AUC(familiarity scalar) | 0.5515 |
| paraphrases judged **Novel** | **1.1%** (2 / 179) |
| Recognized (exact trace) | 14.5%, of which **100%** correct trace |
| Familiar | 84.4% |

The familiarity *scalar* does not separate paraphrases (they share few
surface features with their source) — and a blend of absolute evidence into
the scalar was tried and **rejected** (it lifted topical negatives more than
paraphrase positives; degraded-AUC fell 0.95 → 0.83 for +0.02 paraphrase).
Recognition of paraphrases instead lives at the **verdict** level, where it
works: 98.9% read as non-novel. Consumer guidance (encoded at `score.rs`):
**branch on `verdict`, do not threshold the familiarity scalar across
families.** This closes the "paraphrase scalar separation" item — not by
raising the scalar, but by confirming the verdict path already handles it and
that raising the scalar costs more than it buys.

## Permagent real-query replay — what the production snapshot can and can't say

`permagent_replay` enrolls the live brain and runs the real
`recognition_events` queries (111 substantive queries carrying a real
`rc_focus_wing`) through the engine, scored against two production labels.

| metric | value | reading |
|---|---|---|
| recognised as non-novel | 99.1% | high sensitivity, but no negatives to contrast |
| wing precision (top trace in query's wing) | 0.9% | — |
| cascade agreement (top trace ∈ production retrieved set) | 10.9% | — |

**This is a category finding, not a failure.** `recognition_events` are
*recall* queries — questions like "Henry, what's a field where…" — with
`strategy=cascade`. Query-mode recognition answers "have I seen this
*content* before?", so a short question spuriously locks onto whatever memory
shares its rare tokens (usually the agent name "Henry", which lives in
general/permagent memories), almost never the project wing. Contrast the
stream-mode wing-precision of ~67% when fed actual ambient *content*. The low
numbers here are the recognition≠recall boundary showing up in real data:
recognition is for content re-encounter (the ambient stream), not for
answering user questions (that is the cascade's job).

**Data gap (measured):** all 149 outcomes in this snapshot are `Positive`;
there are no negative outcomes and no per-query familiar/novel label, so a
discrimination AUC is not computable from `recognition_events`. The real
recognition ground truth needs one of: (a) Permagent emitting negative
outcomes, or (b) replaying the ~225 raw ambient-stream memories (which *are*
content stimuli) with their wing labels through stream mode. Until then, the
synthetic degrade/paraphrase replays plus the neural baseline above remain the
load-bearing evidence.

Reproduce:

```bash
cargo run --release -p spectral-recognition --bin permagent_replay -- \
  --brain ~/.permagent/brain/memory.db --events ~/.permagent/spectral/permagent.db
```

## Positive lock rate (degraded re-encounters) — a calibrated operating point

On degraded positives the engine reads 99.9% as non-novel; the **exact-trace
lock** rate is 65.0%, and when it locks it is right **99.9%** of the time. The
other 35% are `Familiar` (correctly "old", not pinned). Gate-failure breakdown
of the 487 non-locking positives: coverage < 0.35 in 288, the anti-flap margin
(< 1.5× runner-up) in 242 (196 of them margin-*only*), min-score in just 2.

So two forces cap the lock rate, and they are different in kind:

- **Margin (196 margin-only):** a near-duplicate trace competes within 1.5×.
  This is the ACR anti-flap rule working as designed — the brain is ~29%
  near-duplicates, and pinning one of two indistinguishable traces would be a
  coin-flip. Correctly held as `Familiar`. Not safe to relax.
- **Coverage (288):** degradation split the surviving fingerprints below the
  0.35 lock threshold. This *is* tunable. Sweep (`SPECTRAL_REC_COVERAGE`, margin
  fixed):

  | coverage | lock rate | correct-trace | clean-negative false-locks |
  |---|---|---|---|
  | **0.35 (shipped)** | 65.0% | 99.9% | 4 / 110 (3.6%) |
  | 0.30 | 71.1% | 99.9% | 8 (7.3%) |
  | 0.25 | 77.4% | 99.9% | 9 (8.2%) |
  | 0.20 | 81.1% | 99.9% | 12 (10.9%) |
  | 0.15 | 82.3% | 99.9% | 17 (15.5%) |

  The marginal trade is favorable in raw counts (0.35→0.30: +85 correct pins for
  +4 false pins) but the clean-negative false-lock *rate* roughly doubles by
  0.25. For "have I seen this?", a false lock (claimed familiarity with a genuine
  novel) is the costlier error, so the shipped default stays at the
  precision-calibrated 0.35.

**Decision:** the operating point is now exposed (`SPECTRAL_REC_COVERAGE`,
`SPECTRAL_REC_MARGIN`) and characterized, not hard-coded-and-opaque. The real
arbiter of where to sit is the Permagent `recognition_events` outcome replay —
synthetic held-out negatives can't say whether a false lock actually costs the
user anything. Keep 0.35 until real outcomes say otherwise.
