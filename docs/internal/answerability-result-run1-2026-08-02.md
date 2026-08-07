# Answerability rerank — run 1 — NULL (failed gates) + a structural finding

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

Prereg: `answerability-prereg-2026-08-02.md`. Held-out LoCoMo, 120 questions,
$0 oracle, zero LLM calls.

## Verdict against the preregistered gates

| gate | rule | result |
|---|---|---|
| 1. Integrity (size-preserving) | set metrics identical | **PASS** — `retrieved_keys` set differed on 0/120; session-recall 92.9%, key-recall 13.8%, zero-recall 4 all identical |
| 2. Effect | rank1 improves ≥ 0.3 | **FAIL** — 3.7 → 3.6 (−0.1) |
| 3. No-harm | no category regresses > 0.2 | **FAIL** — single-session-user 1.6 → 1.9 (+0.3) |

**The lever is recorded as a NULL. Default stays OFF.** Per prereg rule 5, the
weights are not tuned and re-run to rescue this; any later sweep is exploratory
and needs fresh preregistration.

| category | baseline rank1 | answerability rank1 | Δ |
|---|---:|---:|---:|
| multi-session | 5.8 | 5.6 | −0.2 |
| single-session-user | 1.6 | 1.9 | **+0.3** |
| temporal-reasoning | 3.9 | 3.5 | −0.4 |
| **TOTAL** | **3.7** | **3.6** | **−0.1** |

Per-question: 36 improved, 23 regressed, 61 unchanged. Sign test on the 59
changes: one-sided **p = 0.059**. Not significant.

## The structural finding — this is the important part

Breaking the run down by retrieval route:

| route | n | rerank changed hit order | **rendered context changed** |
|---|---:|---:|---:|
| Cascade (default for every non-Temporal shape) | 84 | 84 | **0** |
| TopkFts (Temporal only) | 36 | 36 | 36 |

**On the default retrieval route, reranking cannot change what the actor sees.**

Two independent causes, both verified in code:

1. **The renderer discards rank order.**
   `format_hits_grouped_capped_dated` groups hits by `episode_id` and sorts
   within each group by `key` (turn order), then orders groups by date. Rank is
   never consulted. Whatever order retrieval produced is overwritten.

2. **The set is frozen before reranking.**
   `retrieve_cascade` does `result.merged_hits.into_iter().take(pool_size)`
   with `pool_size == pipeline_config.k` unless ACT-R is on
   (`retrieval.rs:781-786`). Membership is fixed at that `take`, so a
   downstream reorder has nothing left to influence.

Consequence: on 84 of 120 held-out questions this lever was **structurally
incapable of changing the output**, and the run above measured a reordering the
actor would never observe. That is a defect in my instrumentation, not a
property of the hypothesis — and the failed gates above stand regardless.

### Why this matters beyond this lever

The measured record's summary line is:

> retrieval recall is near ceiling and retrieval levers stopped converting to
> accuracy long ago — the actor/synthesis stage is the ceiling.

This finding gives a **partial mechanistic explanation** for part of that, and
narrows it: on the cascade route, *rerank-shaped* levers cannot convert to
accuracy because their effect is erased before the actor sees anything. Only
**admission** changes — which memories enter the set — survive rendering.

This does **not** overturn the rejected levers. K widening, cascade fetch-mult
and associative spreading are all admission levers and were measured on their
merits; ACT-R was implemented correctly (widen pool → rerank → truncate, so it
does change membership). The claim is narrower and specific: any future
rerank-only experiment on the cascade route must change membership or it is
measuring nothing.

### The one route where rank does reach the actor

TopkFts renders with `flat_hit` in rank order, so ordering survives — and it is
also where this lever did best (temporal-reasoning −0.4 rank1, 13 improved vs 6
regressed). That is an observation, not a result: it did not clear the
preregistered effect gate, and n=36 on one category is not a finding.

## What happens next

A corrected experiment (widen the candidate pool, rerank, then truncate — the
pattern ACT-R already uses) is preregistered separately in
`answerability-prereg-run2-2026-08-02.md`. It is a different experiment with a
different mechanism of action, not a retry of this one, and it gets its own
gates.

## Reproduce

```bash
./target/release/spectral-bench-accuracy oracle \
  --dataset locomo_heldout.json --output baseline.jsonl --label baseline \
  --fresh-brains --no-keep-brains

SPECTRAL_ANSWERABILITY=1 ./target/release/spectral-bench-accuracy oracle \
  --dataset locomo_heldout.json --output answ.jsonl --label answerability \
  --fresh-brains --no-keep-brains
```

The baseline arm reproduces the published held-out figures exactly (92.9% /
13.8% / 4), which also confirms the Phase 1 changes (policy regex cache, render
migration) perturbed retrieval by zero.
