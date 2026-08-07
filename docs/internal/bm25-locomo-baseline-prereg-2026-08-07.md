# Prereg — the BM25-only LoCoMo baseline (2026-08-07)

**Status: BUDGET-GATED and BLOCKED. Not run.** Requires (a) a working
bench API key, (b) R15 + R16 landed, (c) budget sign-off (~$18).
Committed before any measurement.

## Why this run exists

No BM25-only end-to-end baseline has ever been published on LoCoMo. Not
by the vendors, and not by the original paper, which used DRAGON dense
retrieval. Every published LoCoMo number therefore compares memory
systems against each other with **no lexical floor** to say what the
machinery is worth.

This is a contribution the field is missing and that we can make for ~$18
because our read path needs no model. It is a **floor measurement, not a
competitive claim.**

## The claim under test

**C1.** What end-to-end LoCoMo accuracy does a memory layer with **zero
model inference at read or write time** achieve?

That is the whole claim. We are not claiming to beat anyone. The number
is interesting whether it is high (the machinery is worth less than
advertised) or low (lexical retrieval has a real ceiling) — and we commit
below to publishing it either way.

## Configuration — pinned before the run

- Retrieval: `--retrieval-path topk_fts` — forces every question onto the
  plain FTS5/BM25 path. **No cascade, no shape routing, no wings, no
  constellation tier, no recognition.**
- `--no-expand-queries` — no LLM query expansion. Combined with the above
  this means **zero model calls inside the memory layer**, at read or
  write time.
- Tokenizer: shipped default (`porter unicode61`). No env levers set.
- Ingest: `per_turn`. Fingerprints at their shipped default (they do not
  participate in the `topk_fts` path; recorded for completeness).
- Actor and judge: `claude-sonnet-4-6`. These are the **reader**, not the
  memory layer, and the distinction must be stated wherever the number is
  quoted.
- Dataset: the full LoCoMo answerable set (categories 1/2/4; adversarial
  and open-domain excluded by the converter, as documented in
  `scripts/locomo_to_oracle.py`). **No sampling** — using every answerable
  question removes any "why this subset" question.
- Commit: must be ≥ the merge that lands R15 and R16, recorded by SHA in
  the result doc.

## Ordering requirement (non-negotiable)

The run happens **after** R15 (true evidence-turn metric) and R16 (SQL
tiebreak) land. R16 shifts the retrieval baseline; publishing numbers
produced before it would mean publishing a figure we have already
invalidated. R15 is required so the retrieval-side companion metric is
the real one rather than the 12×-diluted proxy.

## What gets reported — all of it, unconditionally

1. Overall accuracy and per-category accuracy.
2. **Cluster-robust confidence intervals.** LoCoMo is **10 conversations**,
   not 1,438 independent questions. Vendors report ~±0.31pp (judge
   stochasticity on a fixed item set — the wrong variance). The honest
   interval accounting for clustering is ~±5.5pp. We report the clustered
   interval and say why.
3. The $0 retrieval-side companion: **session recall only.**

   **CORRECTION 2026-08-07, before any run:** an earlier draft of this
   prereg promised true evidence-turn recall here. That is **not
   computable on LoCoMo** — `scripts/locomo_to_oracle.py` marks evidence
   *sessions* with an `answer_` prefix and never emits per-turn
   `has_answer` labels, so the R15 metric reports `n/a` (undefined, NOT
   0%) on every converted LoCoMo set. Reporting a coverage number here
   would repeat exactly the 12x-diluted mistake R15 exists to correct.
   Session recall is the honest companion; evidence-turn recall stays a
   LongMemEval-only figure until the converter is taught `dia_id` →
   turn mapping (register R19, gated on a strip-and-diff byte-equality
   check so the held-out samples cannot silently move).
4. Cost and latency per question, and total tokens.
5. Every failure: transport, auth, judge-parse.

## Mandatory caveats that travel with the number

Any publication of this figure must carry, in the same document:
- **LoCoMo's answer key is ~6.4% wrong** (99 score-corrupting errors in
  1,540 questions, including temporal-reasoning errors), giving a
  practical ceiling near 93.6%.
- **The standard judge accepts ~62.8%** of deliberately-wrong but
  topically-adjacent answers. It rewards vagueness.
- **The same system has scored 58.44–84 on LoCoMo across harnesses** — a
  25.6-point spread wider than any published gap between systems. Our
  number is therefore **not comparable** to any vendor-reported figure,
  and we will say so rather than build a comparison table.
- Full-context baselines beat memory systems in several of those systems'
  own papers.

## Publication commitment

**We publish the number regardless of what it is**, including if it makes
Spectral look unnecessary. That commitment is the point: the field's
defect is selective reporting, and a preregistered floor is only worth
publishing if it was preregistered *before* the result was known.

No re-runs with different settings to find a better figure. If the run
reveals a config error, the corrected re-run is disclosed as such, with
both numbers.

## Cost — verified, not estimated

Dataset built and counted 2026-08-07 (`locomo_to_oracle.py --all`, staged
at `~/spectral-local-bench/locomo_full_answerable.json`):

```
total answerable: 1438
  single-session-user  841
  temporal-reasoning   317
  multi-session        280
```

At the measured $0.0127/question (actor+judge, derived from the R11
runs): **$18.26**. Single run, no arms, no re-rolls.

## Exact invocation (staged; runs when the key and the fixes land)

```
spectral-bench-accuracy run \
  --dataset ~/spectral-local-bench/locomo_full_answerable.json \
  --retrieval-path topk_fts \
  --no-expand-queries \
  --actor-model claude-sonnet-4-6 --judge-model claude-sonnet-4-6 \
  --work-dir <fresh> --output bm25-locomo-baseline.json --confirm-cost
```
