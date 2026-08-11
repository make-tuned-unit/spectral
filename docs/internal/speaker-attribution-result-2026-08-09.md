# R23 — speaker attribution · NULL, and the gate could not have passed (2026-08-09)

**$0. Retrieval-only oracle, LoCoMo, 250 questions, `topk_fts`, k=40, R19 turn
labels. No model calls, no paid runs.** Preregistered at `6c2e32a` before any
of it was written: `speaker-attribution-prereg-2026-08-09.md`.

## Verdict: NULL by the prespecified rule

| arm | ev-micro | Δ | turns | zero-ev | multi-session | tokens |
|---|---:|---:|---:|---:|---:|---:|
| **A0'** re-ingested baseline | 64.89% | — | 231/356 | 53 | 44.70% | 1,989 |
| **B** speaker-prefixed content | **66.57%** | **+1.69pp** | **237/356** | **51** | **46.97%** | 2,058 |

Paired McNemar on the full-evidence indicator: **p = 0.2500**, discordant
**0/3**.

The gate required p < 0.05 **and** ≥ +2.0pp. It fails **both** clauses.
**This is a NULL and is recorded as one.**

**A0' precondition passed exactly** — a complete re-ingest reproduced
231/356, macro 72.75%, 53 zero-evidence, the identical multi-session slice, and
0 discordant pairs against R22's A0. Ingest is deterministic across a fresh
brain build, so arm B is interpretable.

## The gate was structurally incapable of passing — my error in the prereg

With 3 discordant pairs, the smallest attainable two-sided exact McNemar
p-value is `2 × 0.5³ = 0.25`. **p < 0.05 requires at least 6 discordant pairs
all pointing one way.** No outcome of this size could have cleared the
significance clause.

That is not a property of the intervention. It is a property of the statistic I
preregistered: the **all-or-nothing "all evidence turns retrieved" indicator**
discards most of the signal. Arm B gained **+6 evidence turns**, but only 3
questions crossed the full-evidence threshold — the rest gained *some* evidence
without gaining *all* of it.

**The prereg was underpowered for the effect it was designed to detect, and I
did not check that before running.** The result stands as a NULL because that
is what was preregistered, and re-scoring it under a statistic chosen after
seeing the data would be exactly the forking path the prereg exists to prevent.

**The fix belongs to the next prereg, not this one:** a paired test on
per-question evidence-turn *counts* (Wilcoxon signed-rank), with the power
computed in advance. That statistic is **not** applied here.

## The intervention did what it was designed to do

This is the first arm in the entire retrieval series to move the right way on
**every** metric at once, and the only one with **zero regressions**.

**Prespecified mechanism check — did the name inversion close?**

| | missed evidence containing the name | retrieved top-40 containing it | inversion |
|---|---:|---:|---:|
| A0' | 4.3% (3/70) | **36.4%** (772/2120) | **8.5×** |
| B | 3.2% (2/62) | **19.2%** (391/2040) | **5.9×** |

**Arm B stopped spending half its name-matching budget on turns that merely
_mention_ the person** (36.4% → 19.2%), and the pool of missed evidence shrank
70 → 62. The mechanism moved in the predicted direction and **partially**
closed. It did not close fully, which is consistent with a +6-turn effect
rather than the +40 the diagnostic's 62.9% would allow at full capture.

**The dilution risk did not materialise.** The prereg warned that prefixing
every turn with its speaker's name makes a query naming Caroline match half the
corpus, turning a high-IDF term into a near-stopword — the same
reach-exceeds-precision failure that killed RRF. Measured instead:

- rank of first evidence turn **promoted in 65 questions, demoted in 30** (2.2:1
  favourable, against RRF's 71/76 churn)
- **discordant 0/3 — not one question lost full-evidence status**
- zero-evidence **improved** 53 → 51
- **multi-session improved +2.27pp** (44.70% → 46.97%) — the slice every RRF arm
  made worse, and the one the failure analysis identified as carrying +37.5pp

Token cost rose 3.5% (1,989 → 2,058), which is the prefix itself.

## What this means

The direction is right and the magnitude is small. Both matter:

- **The coreference diagnosis is supported.** Attaching speaker identity moves
  retrieval the way the mechanism predicted, and moves the hardest slice most.
  Nothing else in this series has done that.
- **It captures a small fraction of the available 62.9%.** Prefixing makes the
  name *present* in the right turns, but it also makes it present in every
  other turn by that speaker. The signal is admitted, not made discriminative.

That gap is exactly what **arm C** — speaker as a separate indexed field rather
than inline in content — was preregistered to test, and it is now the more
interesting arm, not less.

## Arm C is deferred, not dropped

The prereg fixed three arms. **Two were run.** Arm C requires a small library
change rather than a dataset transform, and is cheaper than first assessed:
`memories_fts` **already indexes `key, content, description` as separate
columns**, so no schema change is needed — but `RememberOpts` does not expose
`description`, so the write path has to be plumbed through.

Deferring it is a scope reduction and is recorded as one.

## Honest limits

- **One corpus, one path, retrieval only.** No end-to-end actor arm; **no paid
  runs**. Retrieval moved +6 turns, which does not license an accuracy claim.
- **`recall_cascade` untested**, and it is the only path Permagent calls. This
  licenses no cascade change.
- **LoCoMo is two-speaker**, the worst possible dilution ratio, so arm B is a
  pessimistic estimate of the prefix approach. It is also the *harder* case than
  production, where speaker identity is already metadata.
- **A NULL is a NULL.** The favourable secondaries are consistent with a real
  small effect and equally consistent with noise. The correct next step is a
  properly powered preregistered replication, not a claim.

## Reproducing

```bash
python3 scripts/build_speaker_dataset.py \
  --labelled ~/spectral-local-bench/locomo_full_answerable_labelled.json \
  --raw      ~/spectral-local-bench/locomo10.json \
  --out      ~/spectral-local-bench/locomo_speaker_prefixed.json \
  --max-questions 250          # 149,456/149,456 turns matched, 0 unmatched
bash scripts/run_speaker_arms.sh
python3 scripts/analyze_rrf_arms.py --arms a0p=a0p.jsonl b=b.jsonl --baseline a0p
```

**Refs:** `speaker-attribution-prereg-2026-08-09.md`,
`speaker-attribution-diagnostic-2026-08-09.md` (the 8.5× inversion this
targets), `rrf-composition-result-2026-08-09.md` (R22).
