# The measured record — what we tried, what held, what we rejected

Every retrieval or accuracy claim in this repo traces to a measured experiment,
and the negative results are load-bearing: they are why the shipped stack looks
the way it does. One line per experiment; the linked doc carries the full
method, numbers, and caveats. Nothing below is asserted without a run behind it.

## Held (shipped or validated)

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **Dated session-grouped context (R11)** | **PASS, SHIPPED** — two-stage held-out LoCoMo A/B with byte-identical retrieval (identity-gated; first attempt voided itself, exposing R14 expansion nondeterminism): `session_grouped` vs the undated shipped block, dev **+19.2pp** (p=1.6e-6), disjoint validation **+14.2pp** (B-fixed 20/broke 3, McNemar p=4.9e-4). Entire effect is temporal-reasoning (20.0%→62.5% validation); other categories bit-flat — undated context starves temporal questions of dates. Facade recall surfaces now publish `session_grouped` as `context_block` (BREAKING for old-block parsers; pinned by test). First prereg-validated accuracy lever in the project's history. | 2026-08-06 | [prereg](internal/r11-render-ab-prereg-2026-08-05.md), [void](internal/r11-render-ab-stage1-void-2026-08-05.md), [stage 1](internal/r11-render-ab-stage1-result-2026-08-05.md), [stage 2](internal/r11-render-ab-stage2-result-2026-08-06.md) |
| **Porter stemming (FTS5 tokenizer)** | **PASS** — zero-evidence questions 8→4, session recall 96.9%→97.6% at $0 (Tier 0); porter-only 46/60 vs LLM-expansion-only 43/60 in the paid head-to-head — a fully deterministic pipeline matches the LLM-assisted config, so porter replaces expansion at $0. Shipped as the default tokenizer. | 2026-07-02 | [ORACLE_TIER0](internal/ORACLE_TIER0.md), [TIER1_RESULTS](internal/TIER1_RESULTS.md) |
| **Identity-keyed counting prompt** | **PASS** — +8.0pp on held-out multi-session counting (18/25 → 20/25), zero regressions; the patterns (identity-keyed dedup, strict inclusion) generalize beyond the tuning set. | 2026-07-12 | [actor-counting-intervention](internal/actor-counting-intervention-2026-07-12.md) |
| **porter + fetch-pool widening (mult=3)** | **NEUTRAL, kept for cost** — accuracy-neutral vs baseline (23/39 both arms); of the 16 remaining failures, 15 had the answer retrieved — the actor, not retrieval, is the ceiling. | 2026-07-03 | [TIER1_PORTER_WIDEN](internal/TIER1_PORTER_WIDEN.md) |

| **Constellation fingerprint retirement** | **COST CONFIRMED, CONCLUSION WITHDRAWN** — the 7x/14.7x figures stand, but tier 1 fires on only 3.2% of questions *because its wing classifier is demo data*. Retiring a feature starved of its input is amputation; the real experiment is to build the taxonomy and re-measure. Default stays `true`. Measured cost: ~39% of write, ~57% of store bytes, 7.0–7.8x ingest and 14.7x storage, byte-identical retrieval over 361 questions. Exposed as `IngestConfig::fingerprints` for consumers who do not use wings, and as the control arm for the taxonomy experiment. | 2026-08-03 | [record](internal/fingerprint-retirement-2026-08-03.md) |
| **Tier-1 ungating (fire on wing alone)** | **REJECTED as preregistered** — reachability 0.0% → 12.0%, against a ≥30% gate; latency +15.4%/+21.9% across runs, straddling the +20% line. The mechanism works (once a wing is detected the index is never empty and the tier fires 26/27), but **the binding constraint is wing detection on the query: 12.4%**, not the hall gate. My ≥30% prediction came from a loose token proxy that overestimated the real classifier by ~3.7x. Default stays gated. | 2026-08-03 | [prereg](internal/tier1-ungating-prereg-2026-08-03.md), [result](internal/tier1-ungating-result-2026-08-03.md) |
| **Wings are a general retrieval accelerator** | **REFRAMED** — a wing fires when the query *names the project*, which real agent queries do 12.4% of the time ("Give me a tour of the app"). Wings are a **scoping mechanism for when scope is stated**, not a primary route. The other 87.6% belongs to ambient context (`RecognitionContext::focus_wing`), which is unexercised. | 2026-08-03 | [result](internal/tier1-ungating-result-2026-08-03.md) |
| **TACT tier-1 gate requires hall on the QUERY** | **DESIGN DEFECT — confirmed, and it is real but not sufficient.** On 217 real Permagent queries against the real taxonomy: wing fires **46.5%**, hall fires **5.5%**, both **0.9%**. Hall is a property of the *memory*, not the question — real queries ("Give me a tour of the app") don't announce what kind of memory answers them. Ungating tier 1 from hall would take reachability 0.9% → 46.5%. The existing 0-wins/2-losses/9-ties verdict was measured on 11 cases from a biased 3% slice and is **not** evidence about the design. | 2026-08-03 | [record](internal/wing-taxonomy-2026-08-03.md) |
| **Wings are auto-derivable from corpus statistics** | **REFUTED** — salient-term anchors trade coverage against discrimination with no workable point: permissive bands yield `just`/`want`/`could` (59.8% coverage, zero topicality), strict bands yield real topics that appear in almost no questions (11.0%). Wings are deployment knowledge; the real brain shows they work when a consumer supplies them (46.5% of real queries name one). | 2026-08-03 | [record](internal/wing-taxonomy-2026-08-03.md) |
| **Removing the demo wing fixtures** | **SIDE-OBSERVATION, not a claim** — held-out LoCoMo after removal: session-recall 92.9%→93.1%, zero-recall 4→3, key-recall 13.8%→13.7%, tokens flat. 39/120 contexts changed because the fixtures were matching `alice`/`acme` keywords on 8.3% of that corpus. Not preregistered and n=120, so directional only — but the fixtures were demonstrably not helping. | 2026-08-03 | [record](internal/wing-taxonomy-2026-08-03.md) |
| **Default wing rules are demo fixtures** | **DEFECT — REMOVED 2026-08-03** — `default_wing_rule_pairs()` ships `alice\|coffee\|noah\|carol-doe`, `acme\|widget\|bob\|recipe`, `apollo\|polymarket` as the library default. Worse, they capture live content in the **real** Permagent brain — `apollo` 46, `alice` 18, `acme` 17, `polaris` 16 memories filed into fictional topic areas by keyword collision, alongside the consumer's genuine wings. Not fixed — needs a migration plan. | 2026-08-03 | [record](internal/fingerprint-retirement-2026-08-03.md) |

## Rejected or refuted

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **K=60→80 admission widening** | **REJECTED** — +36.5% context tokens buys zero new answer sessions and zero case unblocks; pure redundant-evidence spend. | 2026-07-20 | [k-admission-test](internal/k-admission-test-2026-07-20.md) |
| **RERANK associative spreading (weak actor)** | **REFUTED** — powered A/B on the full knowledge-update category: 18/78 vs 20/78, McNemar p=0.81; the encouraging n=30 pilot (+10pp) was regression to the mean (held-out split net −1). Retrieval-lever family closed with a measured null. | 2026-07-27 | [local-actor-ab-result](internal/local-actor-ab-result-2026-07-27.md) |
| **Assistant-turn cap (0.36)** | **REJECTED** — −53% context tokens but accuracy 85.0% → 70.0% (−15pp) on the SSA-weighted sample; all 7 regressions were single-session-assistant, exactly where the evidence lives in truncated turns. | 2026-07-02 | [TIER1_RESULTS](internal/TIER1_RESULTS.md) |
| **Spectrogram as a recall path** | **RETIRED** — enabling write-time spectrograms changed 0/500 retrieval contexts; the crate is repointed to the recognition lineage's history, not recall. | 2026-07-02 | [ORACLE_TIER0](internal/ORACLE_TIER0.md), [SPECTROGRAM_AUDIT](internal/SPECTROGRAM_AUDIT.md) |
| **Recognition accuracy claim** | **WITHDRAWN** — the peak-pair engine is the most accurate method in no regime (MinHash 0.998 vs engine 0.941 on lexical re-encounter; embeddings 0.703 vs 0.543 on semantic). Its measured differentiator is auditable verdicts, not accuracy. | 2026-07-03 | [RECOGNITION_BASELINE](internal/RECOGNITION_BASELINE.md) |
| **Ingest gap decomposition** | **Durability is NOT the bottleneck** — batched SQLite+FTS5 runs **60,489 ev/s, 2.7x FASTER than MinHash+BM25**. Spectral's remaining 7.2x gap is 5% insert floor, **21% per-event transaction commit**, **73% Spectral's own per-event work** (classify/score/hash/episode), which has never been profiled at that granularity. Batching buys 7.2x → ~5.7x. | 2026-08-03 | [decomposition](internal/ingest-gap-decomposition-2026-08-03.md) |
| **Phase 0 vs MinHash+BM25 — RE-RUN 2026-08-03** | **Verdict moved.** The recorded 43 ev/s was stale (the `max_fingerprint_peers` cap landed after it and was never re-measured): the unmodified config now runs **428 ev/s**, and with fingerprints retired **3,148 ev/s at 5.07 KB/event**. Gap to MinHash+BM25 goes from **~500x/40–70x** to **7.2x/2.4x**, determinism tied at 1.0. The storage comparison is RAM-only index vs durable signed store. Cost-moat conclusion unchanged and still negative. | 2026-08-03 | [re-run](internal/phase0-rerun-2026-08-03.md) |
| **Phase 0 vs MinHash+BM25 (original, SUPERSEDED)** | **LOSES the systems axes** — MinHash+BM25 is also $0/offline/deterministic and is ~500× faster to ingest and ~40–70× lighter; the cost moat vs an embedding stack is ~$0.04/month at real volume, below the pre-registered $5 kill line. Spectral's remaining edges: auditability, verified deletion, graph, federation. | 2026-07-03 | [PHASE0_RESULTS](internal/PHASE0_RESULTS.md) |
| **Two-stage actor synthesis (per-session extract + aggregate)** | **STOPPED** — neither target case flipped; the extraction step itself is non-deterministic (same prompt, same model, different output), so net lift is not reproducible. | 2026-05-14 | [candidate-c-aggregation-v2](internal/candidate-c-aggregation-v2.md) |
| **Descriptions in the actor context** | **NO LIFT** — 9/10 vs 9/10, +7% tokens/+11% cost; a gloss is redundant when the memory is retrieved and useless when it isn't. Descriptions stay retrieval-side. | 2026-07-11 | [descriptions-in-actor-context](internal/descriptions-in-actor-context-2026-07-11.md) |
| **ACR retrieval lift → accuracy** | **DOES NOT CONVERT** — +18–40pp answer-key recall on all six memory types, but weak-actor accuracy went 57/74 → 55/74 (net −2): the extra evidence distracts more than it fixes. | 2026-07-15 | [acr-lift-all-memory-types](internal/acr-lift-all-memory-types-2026-07-15.md) |
| **Novelty folded into signal score** | **REJECTED** — measured all-downside; novelty and durability are orthogonal axes, keep them separate. | 2026-07-14 | [novelty-signal-lever](internal/novelty-signal-lever-measured-2026-07-14.md) |
| **Query-conditioned answerability rerank** | **REFUTED over 3 runs and 3 mechanisms** — reorder-only −0.1 rank1 (sign test p=0.059); membership-changing +0.1pp; both, on a fixed foundation, **+0.3pp session-recall / +0.3pp key-recall** against a preregistered +1.0pp gate. Effect size tracked the foundation fixes exactly (+0.1 → +0.3pp) so the lever was fully active, not broken. Closes the *query-conditioned* family — the one kind never previously tested, and the deterministic analogue of the best-measured lever (cross-encoder). | 2026-08-02 | [prereg](internal/answerability-prereg-2026-08-02.md), [run 1](internal/answerability-result-run1-2026-08-02.md), [run 2](internal/answerability-result-run2-2026-08-02.md), [run 3](internal/answerability-result-run3-2026-08-02.md) |
| **TACT fingerprint tier as recall** | **NO HEADROOM** — content recognition never beats FTS at recall (0 wins, 2 losses, 9 ties); both are lexical and BM25 is already strong. | 2026-07-15 | [tact-unlock-synthesis](internal/tact-unlock-synthesis-2026-07-15.md) |

| **Read-time supersession suppression (regex over free text)** | **REJECTED on precision** — conservative `my <attr> is …` extraction fires on 1.02% of 246,930 turns and makes only 8.2% of questions suppressible, but the matches are dominated by **assistant boilerplate** ("My knowledge is derived from…", "As an AI language model…"), not user facts. Supersession belongs in the typed graph layer, where Spectral already implements it correctly. | 2026-08-03 | [prereg](internal/supersession-prereg-2026-08-03.md), [result](internal/supersession-result-2026-08-03.md) |
| **Wall-clock decay breaks reproducibility** | **CLAIM REFUTED (my own)** — recall ordering is time-invariant on both paths: `apply_recency_weight` is multiplicative so a clock shift scales all scores by a common factor, and `recall_at` never re-sorts after decaying. Measured at anchors +200d/+700d/+5y/+30y: zero ordering changes. Decayed *score values* do move. Latent defect found: `recall_at` computes a decayed score it never uses for ordering. | 2026-08-03 | [record](internal/decay-time-invariance-2026-08-03.md) |
| **Policy V2Fixed (classifier defect repair)** | **INCONCLUSIVE (n=1)** — gates passed on paper (preference session-recall 93.3%→96.7%, control moved 0.0pp) but the entire effect is **one question**; the +2.0pp gate was miscalibrated for n=30, where one question *is* 3.3pp. Default stays V1. Two preregistered predictions confirmed. | 2026-08-02 | [prereg](internal/policy-v2-prereg-2026-08-02.md), [result](internal/policy-v2-result-2026-08-02.md) |
| **`*CurrentState` sub-shapes** | **DEAD WEIGHT — confirmed** — `FactualCurrentState`/`CountingCurrentState` share a cascade profile and route with their base shape, so reclassification changes a label and nothing else (2 questions reclassified, zero retrieval effect). Current-state handling is missing from the per-shape **profile table**, not the classifier. | 2026-08-02 | [result](internal/policy-v2-result-2026-08-02.md) |
| **"Fix preference routing to fix the weakest category"** | **REFUTED** — `single-session-preference` is 56.0% end-to-end but its **session-recall is already 93.3%**. The evidence is retrieved for 28/30 questions and the actor still misses 44%. ~37pp of the gap is actor-side; total retrieval headroom is 6.7pp. | 2026-08-02 | [result](internal/policy-v2-result-2026-08-02.md) |

## Measured, not shipped (gated on a value decision)

| Experiment | Verdict | Date | Record |
|---|---|---|---|
| **Cross-encoder rerank** | Best retrieval-precision lever measured to date (+1.6pp session recall on the hard set, zero session losses) — but unproven to end-to-end accuracy and requires a neural model, colliding with the no-embedding stance. Filed, not shipped. | 2026-07-21 | [cross-encoder-dense-rerank](internal/cross-encoder-dense-rerank-2026-07-21.md) |
| **Dense (bi-encoder) hybrid** | Near-null fused over the BM25-won lexical pool, but the only measured lever that reaches the vocabulary-mismatch floor (lifts a pos-302 operand to rank 19). Shipping it is a value decision against the no-embedding stance, not a benchmark verdict. | 2026-07-21 | [cross-encoder-dense-rerank](internal/cross-encoder-dense-rerank-2026-07-21.md) |
| **Cascade fetch-pool widening (`fetch_mult`)** | Capability shipped, default OFF — Pareto-safe on the retrieval metric but a proven accuracy no-op; do not re-default without a powered actor A/B. | 2026-07-14 | [cascade-fetch-mult-lever](internal/cascade-fetch-mult-lever-2026-07-14.md) |

## External evidence: the benchmark leader does no ranking

Mastra's Observational Memory holds the LongMemEval state of the art (**84.23%**
gpt-4o, **94.87%** gpt-5-mini) using **static retrieval** — a compressed
observation log in the prompt prefix, with *no query-based retrieval and no
per-turn reranking at all*. It buys accuracy with two background LLM agents at
write time (Observer, Reflector), the mechanism this project's thesis rejects.

This is independent confirmation of the pattern below: LongMemEval accuracy is
not in retrieval ranking. See
[source-survey-2026-08-03](internal/source-survey-2026-08-03.md).

## Structural findings (why some levers could not have worked)

Two defects found while instrumenting the answerability lever, both verified in
code. They constrain how any *future* retrieval experiment must be built.

| Finding | Consequence |
|---|---|
| **Session-grouped rendering discards rank order.** `format_hits_grouped_capped_dated` groups by `episode_id`, sorts within a group by `key`, and orders groups by date. Rank is never consulted. | On the cascade route — the default for every non-Temporal shape, 70% of the held-out set — a **rerank-only lever cannot change what the actor sees at all**. Only admission (set membership) survives rendering. |
| **Harness pool widening is a no-op on cascade.** `run_cascade_pipeline_scoped` ends with `results.truncate(config.k)` (`cascade_layers.rs:441`), so `merged_hits.take(k * widen)` returns the same `k` items. | `ACTR_POOL_WIDEN` is affected identically, so **ACT-R's pool widening is inert on the cascade route**. ACT-R there is doubly inert: it cannot change membership, and its reordering never reaches the actor. ACT-R is an off-by-default env lever and no published number depends on it, but its recorded behaviour should not be trusted without a re-run. |

Together these give a partial *mechanistic* explanation for the pattern below,
and narrow it: on the default route, rerank-shaped levers were never able to
convert to accuracy, because their effect was erased before the actor saw
anything. Admission levers (K widening, fetch-mult, spreading) were measured on
their merits and rejected on their merits.

**Both are now fixed**, defaulting to the previous behaviour and verified
byte-identical with all levers off (oracle, 0/120 diffs):

- widening reaches `pipeline_config.k` before the pipeline runs, so an
  admission lever changes membership on the cascade route — measured to take a
  lever from acting on 36/120 questions to 114/120;
- `render::SessionOrder::ByRank` orders sessions by best contained rank, so a
  rerank reaches the actor — measured to change the rendered context on 84/84
  cascade questions while changing the retrieved set on 0.

Note the second is **unmeasurable on the $0 oracle**: its metrics derive from
`retrieved_keys`, so rendered session order is invisible to them by
construction. It removes a structural blocker; it is not itself evidence of
benefit.

The pattern across the record: retrieval recall is near ceiling and retrieval
levers stopped converting to accuracy long ago — the actor/synthesis stage is
the ceiling ([TIER1_PORTER_WIDEN](internal/TIER1_PORTER_WIDEN.md),
[local-actor-ab-result](internal/local-actor-ab-result-2026-07-27.md)). The
levers that held are the cheap deterministic ones (porter) and the actor-side
prompt fixes (identity-keyed counting).
