# Preregistration — query-conditioned answerability rerank — 2026-08-02

**Written before the measurement.** Decision rules below are binding.

## Hypothesis

Spectral's reranker (`spectral_graph::ranking::apply_reranking_pipeline`) takes
**no query parameter**. Signal score, recency, entity clustering and context
dedup are all properties of the candidate, not of its fit to the question. Past
the FTS match, the question does not influence ranking.

Every rejected retrieval lever in `docs/MEASURED_RECORD.md` — K widening,
associative spreading, the fingerprint tier, ACR, cascade fetch-mult — is
likewise query-independent: they change *which* candidates are in the pool,
never *how well each one answers this question*. The single best-measured lever
(cross-encoder rerank: +1.6pp session recall, zero session losses) **is**
query-conditioned, and was shelved only for requiring a neural model.

**H1:** a deterministic query-conditioned rerank lowers the rank of the first
answer key without changing which candidates are retrieved.

## What is measured, and what cannot be

The rerank is **size-preserving** — it reorders the admitted set and never adds
or drops a candidate. Therefore, by construction:

| oracle metric | can this lever move it? |
|---|---|
| session-recall | **No** — set membership, invariant |
| key-recall | **No** — set membership, invariant |
| zero-recall | **No** — set membership, invariant |
| **rank1** (mean rank of first answer key) | **Yes — the target metric** |
| context tokens | Only via shape-dependent capping; expected ~flat |

If session-recall, key-recall or zero-recall move at all, that is a **bug**, not
a result: it means the rerank is not size-preserving. This is an integrity
check, not a hypothesis.

## The honesty constraint on rank1

**A rank improvement is not an accuracy improvement, and will not be reported
as one.** The record already contains the exact failure this guards against:

> **ACR retrieval lift → accuracy DOES NOT CONVERT** — +18–40pp answer-key
> recall on all six memory types, but weak-actor accuracy went 57/74 → 55/74
> (net −2): the extra evidence distracts more than it fixes.

rank1 is a proxy for the "lost in the middle" effect and nothing more. Whether
a better rank converts to a better answer is an **actor-side** question that
this $0 oracle cannot answer. Converting requires a powered paid A/B, which is
a separate, budgeted decision.

## Decision rules (binding)

1. **Integrity gate.** session-recall, key-recall and zero-recall must be
   **identical** to baseline on all 120 held-out LoCoMo questions. Any
   difference = the implementation is not size-preserving → fix or withdraw.
   No result is reportable until this passes.
2. **Effect gate.** rank1 must improve (decrease) by **≥ 0.3 positions overall**
   for the lever to stay in the tree as a candidate. Smaller than that is noise
   at n=120 and the lever is recorded as a null.
3. **No-harm gate.** No individual category's rank1 may regress by **> 0.2**.
   A lever that trades temporal against multi-session is not a win.
4. **Default stays OFF regardless of outcome.** Passing gates 1–3 makes this a
   candidate for a paid actor A/B, not a default. `AnswerabilityConfig::enabled`
   is `false` and only a paid, powered accuracy result may change that.
5. **No weight tuning after seeing the result.** The weights below are fixed
   now. If the result is a null at these weights, it is recorded as a null.
   A sweep may be run afterwards but any sweep result is explicitly
   **exploratory** and requires fresh preregistration to be claimed.

## Fixed configuration

Weights, frozen before the run (`AnswerabilityConfig::default`):

| parameter | value |
|---|---|
| `answer_type_weight` | 0.12 |
| `coverage_weight` | 0.10 |
| `ack_penalty` | 0.15 |
| `topic_only_penalty` | 0.08 |
| `rank_step` | 0.03 |

Features, all deterministic and $0:

- **answer-type match** — the token type the question shape needs
  (Counting→number, Temporal→date, Factual→proper noun,
  GeneralPreference→preference cue). A bonus when present.
- **coverage** — fraction of *distinct* query content words present. Orthogonal
  to BM25, which is IDF-weighted and length-normalised and so can rank a
  document highly on one rare term repeated.
- **acknowledgement penalty** — phatic turns ("Sure!", "Got it"). This is the
  scored form of the render layer's `< 40` char hard drop; a penalty demotes
  without destroying evidence, so a short real answer ("Yes, 42") can still
  surface.
- **topic-only penalty** — matched query topic words but carries no token of
  the needed answer type: "matches the subject, answers nothing".

`rank_step` is a **uniform** per-position prior, deliberately not `1/(1+rank)`.
The reciprocal form puts a 0.5 gap between ranks 0 and 1 but only 0.17 between
1 and 2, making the top slot immovable while letting the tail shuffle freely —
influence that varies by position rather than by evidence. (Found by a unit
test, not by inspection.)

## Method

- Dataset: `locomo_heldout.json`, 120 questions, **held out** — the retrieval
  was never tuned on LoCoMo.
- `$0` oracle, zero LLM calls, `--fresh-brains --no-keep-brains`.
- Baseline arm reproduces the published held-out figures exactly
  (92.9% / 13.8% / 4 zero-recall), which also confirms the Phase 1 changes
  (regex cache, render migration) perturbed retrieval by zero.
- Single env lever `SPECTRAL_ANSWERABILITY=1`; both arms otherwise identical.

## Prior probability, stated up front

Low-to-moderate. The measured record's summary line is that "retrieval recall
is near ceiling and retrieval levers stopped converting to accuracy long ago —
the actor/synthesis stage is the ceiling." This lever is different in kind from
the ones already refuted (query-conditioned, not admission-widening), which is
the reason to run it. That is a reason for a $0 test, not a reason to expect a
win.
