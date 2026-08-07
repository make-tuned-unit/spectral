# Read-path lever sweep — 2026-07-25

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

## Question

The write path had a known O(N) term (fixed separately). Holistically, what
else is costing real time — and is any of it removable without touching what
Spectral actually does?

Constraint from the outset: **no core subsystem gets removed.** TACT, the
cascade, recognition and the constellation all stay. The only admissible
changes are ones that make existing behaviour cheaper.

## What was measured

`spectral-bench-real --bin recall_path_cost` — the three public read paths plus
`Brain::open`, at corpus 100/200/400/800, median of 12 reps × 5 queries, page
cache warmed. Deterministic, $0, no LLM.

## Levers tested and REJECTED (recorded so they are not re-chased)

| lever | result | why rejected |
|---|---|---|
| Missing index on the recall query | `EXPLAIN QUERY PLAN` shows the `NOT IN` uses the `consolidation_edges` PK autoindex; join is by rowid | nothing to add |
| `prepare_cached` on the 48 raw `prepare()` sites | compile saving **0.0124 ms/call** = 0.22% of a 5.6 ms recall | 48-call-site churn for noise |
| Read path degrades with corpus size | cascade 1.3x, `recall_local` 1.3x over an 8x corpus | no scaling problem to fix |
| `async_writeback` by default | saves ~0.85 ms of 5.6 ms | real but a durability trade; not free, so not defaulted |
| Skipping TACT tiers in the cascade | multi-session session recall 100.0% → 99.0%, key recall 48.6% → **46.0%** | removing a core subsystem measurably hurts; reverted |

The TACT-bypass experiment is worth keeping on record for a second reason: on
*single-session-preference* it went the other way (session recall 90.0% → 95.0%,
zero-recall 2 → 1, same key recall). So the long-standing "route General shapes
to FTS" hypothesis is real but **category-dependent** — a routing question in
the frozen retrieval zone, not a removal. Not pursued.

## The lever that worked

`TactConfig` stores wing/hall rules as regex **pattern strings**, and
`classifier::detect_wing` / `detect_hall` called `Regex::new(pattern)` *inside
the match loop* — recompiling every rule on every call. TACT runs on every
cascade recall, and classification runs twice per retrieval, so this was paid
on every single read. Three more static patterns (`extract_query_terms`,
`extract_fts_words`, and two in `spectrogram::dimensions`, the latter on every
memory at ingest) were also compiled per call.

Fix: compile-once caches. A pattern-keyed cache in `classifier` for the
configurable rules, `OnceLock` for the static ones. A pattern that fails to
compile is still cached as `None` and skipped, preserving the old
`if let Ok(re)` semantics exactly.

| stage | before | after | |
|---|---:|---:|---|
| classify (wing+hall) | 0.2828 ms | 0.0059 ms | **48x** |
| TACT candidate gathering @100 | 4.45 ms | 0.62 ms | 86% |
| cascade recall @100 | 5.38 ms | 1.48 ms | **72% faster** |
| cascade recall @800 | 6.84 ms | 2.91 ms | 57% faster |
| `recall_local` @100 | 4.36 ms | 0.50 ms | 89% faster |

Confirmed across two consecutive runs. Nothing was removed; the same tiers run,
the same rules match.

Also bundled: `spectral_tact::retrieve_memories`, a sibling of `retrieve` that
skips building `context_block`. The cascade path consumed only `memories` and
discarded the formatted block. Measured on its own this was **null** (within
noise) — it is kept as elimination of provably wasted allocation, not because
it showed a number.

## Retrieval verification

Regex caching is semantically identical, and the oracle confirms it: 25-question
held-out multi-session set, **0 context-hash differences, 0 answer-key delta, 0
token delta**, and the summary reproduces the published baseline exactly
(100.0% session recall, 48.6% key recall, 17,943 tok-mean).

## Still open

- Recognition enrolment grows 3.0x over 800 writes (~180 inverted-index rows per
  memory via `index_minhash`). An LSH-banding path exists but trades against
  recognition recall — needs its own bench.
- `Brain::open` is 10–16 ms and grows mildly. Irrelevant for a long-lived
  process, potentially worth attention for short-lived CLI use.
- `bench-accuracy/src/retrieval.rs` has ~11 per-call `Regex::new` in shape
  routing. Bench-only, so it distorts harness speed rather than library speed;
  left alone to keep this change library-scoped.
