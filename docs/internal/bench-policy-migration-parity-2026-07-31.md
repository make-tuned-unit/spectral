# Bench → library retrieval-policy migration: parity evidence — 2026-07-31

The question-shape classifier, per-shape cascade profiles, and route selection
moved from `spectral-bench-accuracy` into the library as
`spectral::policy` (`RetrievalPolicyVersion::V1`).

**Why:** the bench owned its own retrieval policy and did not depend on the
`spectral` facade crate at all — it wired the sub-crates directly. Every
published accuracy number therefore described a *harness* configuration that no
consumer could execute. That is the in-sample→product-configuration credibility
gap named in `docs/MEASURED_RECORD.md`.

**Discipline:** the logic moved **verbatim**. This migration is required to be
behaviour-preserving, and the $0 retrieval oracle is the gate. Stated in
advance: *any mismatch not fully explained kills the migration.*

## Gate: $0 retrieval oracle, LongMemEval-S, all 500 questions

Cached brains reused (identical corpus), same machine, back-to-back.

| aggregate | baseline | migrated |
|---|---|---|
| session-recall | 97.9% | **97.9%** |
| key-recall | 54.9% | **54.9%** |
| zero-retrieval | 3 | **3** |
| mean rank-1 | 2.3 | **2.3** |
| context tokens (mean / p95) | 14440 / 23564 | **14440 / 23564** |

Per-category rows were identical for all six memory types.

### Per-question field comparison (the real gate)

| field | identical |
|---|---|
| `context_hash` | **500 / 500** |
| `shape` | 500 / 500 |
| `retrieval_path` | 500 / 500 |
| `answer_keys_retrieved` | 500 / 500 |
| `rank_first_answer_key` | 500 / 500 |
| `n_retrieved` | 500 / 500 |
| `context_tokens_est` | 500 / 500 |
| `retrieved_keys` | 496 / 500 |
| `retrieval_wall_ms` | 104 / 500 (wall clock — expected to vary) |

## The 4 `retrieved_keys` mismatches are pre-existing, not migration-caused

All four are `shape=Counting`, `path=Cascade`. For each: the key **set** is
identical, the length is identical (60), and the **`context_hash` is identical**
— only the order of the emitted key list differs.

Decisive check: **running the same binary twice reproduces exactly the same
4-row instability**, on the same shape, with `context_hash` stable 500/500. So
the ordering variation exists without any code change and is not attributable to
the migration.

| comparison | `retrieved_keys` order differs | `context_hash` differs |
|---|---|---|
| baseline vs migrated | 4 / 500 (all Counting) | 0 / 500 |
| migrated vs migrated (same binary) | 4 / 500 (all Counting) | 0 / 500 |

**Verdict: T1 parity PASSES**, with the mismatch fully explained.

## Incidental finding — worth its own ticket, not fixed here

The Counting cascade path has **unstable tie ordering**: repeated identical runs
emit the same 60 keys in a different order. It does not affect delivered output
(the rendered context is byte-identical, hence the stable hash), so no published
number is affected. But determinism is a stated Spectral property — Phase 0
recorded determinism 1.0, "byte-identical rankings on repeat" — and this is a
narrow exception to it at the key-list level. Counting uses `k=60` with
`max_per_episode: 3`, so the episode-diversity interleave over score ties is the
likely source.

## What moved, and what deliberately did not

**Moved to `spectral::policy`:** `QuestionShape` (8 variants) with `classify`,
`cascade_profile`, `retrieval_route`; `RetrievalRoute`; `RetrievalPolicyVersion`.

**Stayed in the harness:** actor prompt templates (`prompt_template` /
`prompt_content`), context formatting, dated-context rendering, assistant caps,
and the `SPECTRAL_*` env levers. These describe how to talk to a model or how to
render a benchmark context — not how memory retrieves.

The bench keeps the historical names via re-export
(`QuestionShape as QuestionType`, `RetrievalRoute as RetrievalPath`), so ~130
call sites are unchanged and the diff stays reviewable. The bench's existing
classifier unit tests now exercise the library implementation through that
re-export.

## What this does and does not earn

**Does:** a published number can now cite `RetrievalPolicyVersion::V1` and name
an executable configuration that ships in the library.

**Does not:** it does not make the LongMemEval-S result held-out. It is still
in-sample — the policy was tuned against this dataset. Generalization still
requires the held-out LoCoMo evaluation (`BENCHMARKING.md` §4). This migration
removes the "the benchmark runs code the product doesn't have" objection, not
the "tuned on the same questions" objection.
