# The measured record — what we tried, what held, what we rejected

Every retrieval or accuracy claim in this repo traces to a measured experiment,
and the negative results are load-bearing: they are why the shipped stack looks
the way it does. One line per experiment; the linked doc carries the full
method, numbers, and caveats. Nothing below is asserted without a run behind it.

## Held (shipped or validated)

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **Porter stemming (FTS5 tokenizer)** | **PASS** — zero-evidence questions 8→4, session recall 96.9%→97.6% at $0 (Tier 0); porter-only 46/60 vs LLM-expansion-only 43/60 in the paid head-to-head — a fully deterministic pipeline matches the LLM-assisted config, so porter replaces expansion at $0. Shipped as the default tokenizer. | 2026-07-02 | [ORACLE_TIER0](internal/ORACLE_TIER0.md), [TIER1_RESULTS](internal/TIER1_RESULTS.md) |
| **Identity-keyed counting prompt** | **PASS** — +8.0pp on held-out multi-session counting (18/25 → 20/25), zero regressions; the patterns (identity-keyed dedup, strict inclusion) generalize beyond the tuning set. | 2026-07-12 | [actor-counting-intervention](internal/actor-counting-intervention-2026-07-12.md) |
| **porter + fetch-pool widening (mult=3)** | **NEUTRAL, kept for cost** — accuracy-neutral vs baseline (23/39 both arms); of the 16 remaining failures, 15 had the answer retrieved — the actor, not retrieval, is the ceiling. | 2026-07-03 | [TIER1_PORTER_WIDEN](internal/TIER1_PORTER_WIDEN.md) |

## Rejected or refuted

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **K=60→80 admission widening** | **REJECTED** — +36.5% context tokens buys zero new answer sessions and zero case unblocks; pure redundant-evidence spend. | 2026-07-20 | [k-admission-test](internal/k-admission-test-2026-07-20.md) |
| **RERANK associative spreading (weak actor)** | **REFUTED** — powered A/B on the full knowledge-update category: 18/78 vs 20/78, McNemar p=0.81; the encouraging n=30 pilot (+10pp) was regression to the mean (held-out split net −1). Retrieval-lever family closed with a measured null. | 2026-07-27 | [local-actor-ab-result](internal/local-actor-ab-result-2026-07-27.md) |
| **Assistant-turn cap (0.36)** | **REJECTED** — −53% context tokens but accuracy 85.0% → 70.0% (−15pp) on the SSA-weighted sample; all 7 regressions were single-session-assistant, exactly where the evidence lives in truncated turns. | 2026-07-02 | [TIER1_RESULTS](internal/TIER1_RESULTS.md) |
| **Spectrogram as a recall path** | **RETIRED** — enabling write-time spectrograms changed 0/500 retrieval contexts; the crate is repointed to the recognition lineage's history, not recall. | 2026-07-02 | [ORACLE_TIER0](internal/ORACLE_TIER0.md), [SPECTROGRAM_AUDIT](internal/SPECTROGRAM_AUDIT.md) |
| **Recognition accuracy claim** | **WITHDRAWN** — the peak-pair engine is the most accurate method in no regime (MinHash 0.998 vs engine 0.941 on lexical re-encounter; embeddings 0.703 vs 0.543 on semantic). Its measured differentiator is auditable verdicts, not accuracy. | 2026-07-03 | [RECOGNITION_BASELINE](internal/RECOGNITION_BASELINE.md) |
| **Phase 0 vs MinHash+BM25** | **LOSES the systems axes** — MinHash+BM25 is also $0/offline/deterministic and is ~500× faster to ingest and ~40–70× lighter; the cost moat vs an embedding stack is ~$0.04/month at real volume, below the pre-registered $5 kill line. Spectral's remaining edges: auditability, verified deletion, graph, federation. | 2026-07-03 | [PHASE0_RESULTS](internal/PHASE0_RESULTS.md) |
| **Two-stage actor synthesis (per-session extract + aggregate)** | **STOPPED** — neither target case flipped; the extraction step itself is non-deterministic (same prompt, same model, different output), so net lift is not reproducible. | 2026-05-14 | [candidate-c-aggregation-v2](internal/candidate-c-aggregation-v2.md) |
| **Descriptions in the actor context** | **NO LIFT** — 9/10 vs 9/10, +7% tokens/+11% cost; a gloss is redundant when the memory is retrieved and useless when it isn't. Descriptions stay retrieval-side. | 2026-07-11 | [descriptions-in-actor-context](internal/descriptions-in-actor-context-2026-07-11.md) |
| **ACR retrieval lift → accuracy** | **DOES NOT CONVERT** — +18–40pp answer-key recall on all six memory types, but weak-actor accuracy went 57/74 → 55/74 (net −2): the extra evidence distracts more than it fixes. | 2026-07-15 | [acr-lift-all-memory-types](internal/acr-lift-all-memory-types-2026-07-15.md) |
| **Novelty folded into signal score** | **REJECTED** — measured all-downside; novelty and durability are orthogonal axes, keep them separate. | 2026-07-14 | [novelty-signal-lever](internal/novelty-signal-lever-measured-2026-07-14.md) |
| **TACT fingerprint tier as recall** | **NO HEADROOM** — content recognition never beats FTS at recall (0 wins, 2 losses, 9 ties); both are lexical and BM25 is already strong. | 2026-07-15 | [tact-unlock-synthesis](internal/tact-unlock-synthesis-2026-07-15.md) |

## Measured, not shipped (gated on a value decision)

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **Cross-encoder rerank** | Best retrieval-precision lever measured to date (+1.6pp session recall on the hard set, zero session losses) — but unproven to end-to-end accuracy and requires a neural model, colliding with the no-embedding stance. Filed, not shipped. | 2026-07-21 | [cross-encoder-dense-rerank](internal/cross-encoder-dense-rerank-2026-07-21.md) |
| **Dense (bi-encoder) hybrid** | Near-null fused over the BM25-won lexical pool, but the only measured lever that reaches the vocabulary-mismatch floor (lifts a pos-302 operand to rank 19). Shipping it is a value decision against the no-embedding stance, not a benchmark verdict. | 2026-07-21 | [cross-encoder-dense-rerank](internal/cross-encoder-dense-rerank-2026-07-21.md) |
| **Cascade fetch-pool widening (`fetch_mult`)** | Capability shipped, default OFF — Pareto-safe on the retrieval metric but a proven accuracy no-op; do not re-default without a powered actor A/B. | 2026-07-14 | [cascade-fetch-mult-lever](internal/cascade-fetch-mult-lever-2026-07-14.md) |

The pattern across the record: retrieval recall is near ceiling and retrieval
levers stopped converting to accuracy long ago — the actor/synthesis stage is
the ceiling ([TIER1_PORTER_WIDEN](internal/TIER1_PORTER_WIDEN.md),
[local-actor-ab-result](internal/local-actor-ab-result-2026-07-27.md)). The
levers that held are the cheap deterministic ones (porter) and the actor-side
prompt fixes (identity-keyed counting).
