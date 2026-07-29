# Recognition — public benchmark results

Pre-registered before measurement
([prereg](internal/recognition-public-benchmark-prereg-2026-07-28.md), committed
`deb410e` — predictions, datasets, and decision rules are in git history ahead
of the harness and every number below). Reproduce:
`cargo run -p spectral-recognition --bin public_bench` against the public
datasets named in the prereg; canonical numbers live in
[`recognition-benchmark-results.json`](recognition-benchmark-results.json),
which CI cross-checks against this document.

## The trade-off surface (clean ROC-AUC, identical splits for all systems)

| regime | peak-pair engine | MinHash-128 | BGE-small (embeddings) |
|---|---|---|---|
| R1 — lexical re-encounter (LongMemEval-S, 30% dropout) | 0.9946 | **0.9988** | 0.8497 |
| R2 — paraphrase (MRPC) | 0.9788 | 0.9668 | **0.9812** |
| R3 — adversarial paraphrase (PAWS) | 0.4875 | 0.4917 | 0.4853 |

## Honest reading

- **Lexical re-encounter (R1): MinHash still wins** (0.9988 vs 0.9946), as
  measured privately and pre-registered. We do not claim best-in-regime
  accuracy. The engine's edge over embeddings here is large (+0.14 AUC) at
  ~36× lower query latency (13.8ms vs ~495ms) with no model.
- **R2 came in above every system's pre-registered band.** Our verification
  pass explains it rather than celebrates it: MRPC "paraphrases" share a
  median 0.55 content-word Jaccard — it is a substantially *lexical* test.
  R2 does not demonstrate semantic understanding for any system; treat it as
  an easy-paraphrase ceiling check.
- **Adversarial paraphrase (R3): every method is at chance — including
  embeddings.** PAWS pairs share identical lexical-overlap distributions for
  matched and mismatched meaning (median Jaccard 0.889 both), so only real
  semantic structure can score above 0.5. We pre-registered our own failure
  here (band 0.45–0.65; measured 0.4875) — and predicted embeddings would do
  better (band 0.55–0.75); they did not (0.4853). **The prediction miss is
  reported, not hidden: off-the-shelf embedding similarity fails this regime
  too.**
- **Verdict layer: zero missed re-encounters in any regime** (`pos_novel = 0`
  across R1–R3) — the engine's conservative direction is false-familiar, never
  false-novel. The public benchmark exposed a scale defect in the original
  thresholds (calibrated at 1.6k memories, near-total false-familiar at 9k);
  the scale-robust recalibration (min-features + similarity-floor,
  pre-registered with hard never-miss constraints) cut cross-document
  false-familiars from 81% to **31.3%** (R2) with zero missed re-encounters.
  R1's residual high rate is negative-set construction (same-conversation
  turns, median 0.27 Jaccard vs enrolled), and R3's 100% is the semantic
  regime nothing distinguishes. "Familiar" remains a lead, not a proof —
  which is precisely why every verdict carries an auditable evidence trail.

## What the claims are (and are not)

Claimed, and enforced by tests in `crates/spectral-recognition/tests/invariants.rs`:
1. **Deterministic** — byte-identical verdicts across runs and insertion orders.
2. **Auditable** — every non-Novel verdict cites landmarks machine-verified to
   exist in both probe and enrolled content; no evidence ⇒ Novel.
3. **Zero inference** — the default build has no network stack, no model, no
   LLM; recognition costs ~µs–ms of CPU.
4. **This published trade-off surface** — including both regimes where we
   lose and the regime where nothing works.

Not claimed: best-in-regime accuracy (withdrawn 2026-07-03 —
[the measured record](MEASURED_RECORD.md)).
