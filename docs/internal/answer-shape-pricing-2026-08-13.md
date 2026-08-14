# R33 — answer-shape matching, priced for $0 · NOT BUILT (2026-08-13)

> **Renumbering note:** registered under the number "R30"/"R31" on a
> parallel branch; renumbered R32/R33 on rebase after main's same-numbered
> rows became visible. Content otherwise unchanged.


**$0, offline re-read of the R32 baseline arm (`a0.jsonl`, full N, `topk_fts`)
plus the dataset's own labels. No implementation, no run.**
`scripts/price_answer_shape.py`.

R22 queued "answer-shape matching" — *'how many' preferring quantity-bearing
turns* — as the last query-conditioned $0 idea. Before writing a boost into
`ranking.rs`, this prices the signal it would use.

## The signal is real…

| class | questions | shape on evidence | on distractors | lift | on MISSED evidence |
|---|---:|---:|---:|---:|---:|
| count ("how many/much/often/long") | 72 (5.0%) | 38.8% | 10.1% | **3.84×** | 36.4% |
| date-time ("when/what date/…") | 283 (19.7%) | 78.6% | 12.9% | **6.08×** | 70.7% |

A 6× lift is a genuinely discriminative, query-conditioned signal — unlike
declarative density, which was a static corpus prior. The R22 intuition was
right that this is a different kind of lever.

## …and the prize is too small anyway

The two classes cover **55 + 58 = 113 missed evidence turns — 13.2% of all
misses**, of which only the shape-bearing ones are addressable: **20 + 41 =
61 turns = +2.85pp absolute ceiling** on corpus evidence recall, before:

1. **pool truncation** — a rerank boost only touches the fetched pool
   (k × fetch_mult); any addressable turn deeper than that is unreachable
   regardless of boost (the R22 arithmetic);
2. **distractor competition** — ~13% of every haystack carries the shape,
   so a boost also promotes dozens of non-evidence turns per question (the
   R32 dilution mechanism, measured 4/23 there).

The prespecified house gate for this series is **≥ +2.0pp corpus micro plus
significance**. A lever whose *perfect* ceiling is +2.85pp, eroded by two
mechanisms both measured to bite on this corpus, cannot honestly be built
against that gate. **Where the addressable population is small, so is the
prize — by construction** (the same arithmetic that closed the LongMemEval
adjacency run).

## What would change the verdict

- A **per-class goal**: within date-time questions alone the addressable
  ceiling is 41/313 = **+13.1pp class recall**. If the temporal-reasoning
  slice ever becomes the target (it is the slice R11 moved end-to-end), this
  pricing says the signal is strong there and a class-scoped prereg would be
  legitimate — with a class-scoped gate declared in advance, not a corpus
  gate quietly narrowed after the fact.
- A **second retrieval modality** that changes what enters the pool.
  Boosting can only reorder what admission provides; the 38% of misses
  unreachable by both owned channels (adjacency mechanism diagnostic) still
  argue that admission, not ranking, is the frontier.

## Verdict

**NOT BUILT, by its own pricing.** This closes R22's residue: both queued
$0 ideas are now measured (R32: +1.70pp held-out, FAIL) or priced under the
gate (this). The $0 lexical/structural family on this corpus is exhausted;
the frontier is a second modality.

**Refs:** `rrf-composition-result-2026-08-09.md` (R22, which queued it),
`query-aliases-result-2026-08-13.md` (R32),
`adjacency-mechanism-diagnostic-2026-08-11.md` (the frontier argument).
