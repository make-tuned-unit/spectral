# R24 — speaker as a separate indexed field · **PASS** (2026-08-09)

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
topk_fts`, k=40, R19 turn labels. No model calls, no paid runs, model-free.**
Preregistered at `aaba5a9` before implementation:
`speaker-field-prereg-2026-08-09.md`.

**The first PASS in this retrieval series.** Capability is bench-scoped and
**default off**; see "What does NOT follow" before acting on it.

## Result

| | A0″ baseline | **C** speaker-field | Δ |
|---|---:|---:|---:|
| evidence-turn **micro** | 59.86% (1281/2140) | **62.62% (1340/2140)** | **+2.76pp** |
| macro | 68.63% | 70.54% | +1.91pp |
| zero-evidence questions | 357 | **329** | **−28** |
| full-evidence questions | 898 | 929 | +31 |
| **multi-session micro** | 40.91% | **45.49%** | **+4.58pp** |
| context tokens | 1,974 | 1,980 | **+0.3%** |

**Primary statistic (preregistered):** Wilcoxon signed-rank on per-question
evidence-turn counts. **72 nonzero pairs [+64 / −8]**, W = 315.5,
**two-sided p < 0.0001**.

**PASS on both prespecified clauses** — p < 0.05 and ≥ +2.0pp — with the power
floor (≥15 nonzero pairs) cleared by nearly 5×. Secondary McNemar agrees
(discordant 4/35, p < 0.0001).

**+59 evidence turns for +0.3% tokens.** This is close to free.

## Both preconditions passed

- A0″'s first 250 rows reproduce the R22/R23 baseline exactly: 231/356, 53
  zero-evidence, 0 discordant.
- A0″ at full N reproduces **R19's published corpus figures** exactly: 59.86%
  micro, 68.63% macro, 357 zero-evidence (24.86%). This is the stronger check —
  it validates against the published record rather than another arm.

## The mechanism moved, and it is the one that was predicted

Prespecified check — the share of turns containing the queried person's name:

| | missed evidence | retrieved top-40 | inversion |
|---|---:|---:|---:|
| A0″ | 8.1% | **38.4%** | **4.8×** |
| C | 11.0% | **20.8%** | **1.9×** |

The coreference diagnostic measured that BM25 spends its budget on turns that
*mention* a person while the evidence is that person's own turn. Arm C nearly
halved the share of retrieved turns matching on a mention (38.4% → 20.8%) and
cut the inversion from 4.8× to **under 2×**. The missed-evidence pool shrank
533 → 462.

**Multi-session gained most (+4.58pp)** — the weakest slice, the one the failure
analysis identified as carrying +37.5pp, and the one **every RRF arm made
worse**. That is what the mechanism predicts: cross-session questions are
exactly where a name is the strongest available query term and where matching it
to mentions rather than utterances costs the most.

First-evidence-turn rank moved up in 374 questions against 145 down (2.6:1),
versus RRF's 71/76 churn.

## Why this worked where R23 arm B did not

R23 prefixed the speaker inline into content and returned +1.69pp under a test
that could not pass. R24 puts the same information in the separate indexed
`description` column. The prereg predicted the difference in advance: prefixing
makes the name present in the right turns *and in every other turn by that
speaker*, diluting the content channel; a separate column lets a query naming a
person match turns that person **spoke** without competing with content
matching.

The B′ arm (R23 replicated at full N) is **still running** and will settle
whether that account is right. It is reported separately and does not affect
this verdict.

## What does NOT follow

- **No accuracy claim.** Retrieval only. `P(correct | full evidence) = 88.31%`
  makes +59 evidence turns *suggestive*, but no end-to-end arm was run and none
  is budgeted. **A retrieval PASS is not an accuracy PASS.**
- **No cascade change.** `recall_cascade` is the only path Permagent calls and
  was not measured. Defaults stay off on both paths.
- **Bench-scoped implementation.** Arm C writes the speaker via
  `set_description` in the bench ingest. Production would need speaker identity
  plumbed as first-class metadata — which Permagent already holds, making this
  easier there than here, not harder.
- **Corpus-shaped.** LoCoMo is two-speaker with strictly alternating turns
  (272/272 sessions). A many-speaker corpus dilutes the signal differently, and
  a document corpus has no speaker at all. This result does not transfer
  unexamined.
- **Restored metadata, not inference.** Speaker comes from raw LoCoMo
  (`speaker_a`/`speaker_b`, per-turn `speaker`), 865,369/865,369 turns matched.
  Never a question, answer, or evidence label.

## The N lesson, recorded because it changed the outcome

R23 used 250 questions and returned an uninterpretable null. R24 used 1,438 —
the same lever, 5.75× the data — and returned a decisive PASS with 72 nonzero
pairs where 250 questions had produced 6.

The 250-question subset was inherited from G4 and never questioned. It is also
**~5pp easier than the corpus** (64.89% vs 59.86%), so every absolute number
in this series quoted from it is optimistic. N was never disk-bound:
`oracle.rs` deletes each brain inside the question loop, so peak disk is one
brain (~20MB).

**Run full N unless there is a stated reason not to.**

## Reproducing

```bash
python3 scripts/build_speaker_dataset.py --mode field \
  --labelled ~/spectral-local-bench/locomo_full_answerable_labelled.json \
  --raw      ~/spectral-local-bench/locomo10.json \
  --out      ~/spectral-local-bench/locomo_speaker_field.json --max-questions 1438
bash scripts/run_speaker_field_arms.sh
python3 scripts/score_r24.py --baseline a0pp.jsonl --arm c.jsonl --label C
```

**Refs:** `speaker-field-prereg-2026-08-09.md`,
`speaker-attribution-diagnostic-2026-08-09.md` (the 8.5× inversion this
targeted), `speaker-attribution-result-2026-08-09.md` (R23, underpowered),
`rrf-composition-result-2026-08-09.md` (R22).
