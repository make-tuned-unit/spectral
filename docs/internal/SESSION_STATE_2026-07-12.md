# Session state / resume point — 2026-07-12

Save point for the recall-hardening + recency + actor-counting arc. Read this
first to pick up.

## Branch
`feat/recall-recency-hardening-and-levers` — 7 commits, pushed to origin.
PR: https://github.com/make-tuned-unit/spectral/pull/new/feat/recall-recency-hardening-and-levers
Working tree clean. Not yet merged to `main`.

## What shipped (default-path, proven correct)
- **Query separator tokenization** — split on separators (`@ / . % $`) instead of
  deleting them; `alice@acme.io` etc. no longer merge into non-matching blobs.
  Subsumes the possessive fix. `brain.rs::fts_query_words_opts`.
- **Recency correctness + safety** — parser now dual-format (SQLite + RFC3339, so
  `RememberOpts.created_at` imports get recency), and recency changed from a
  score-annihilating multiplier to a bounded additive tiebreaker
  (`RECENCY_BOOST_WEIGHT=0.1`). All created_at parsers unified on
  `ranking::parse_created_at`.
- **Bench harness fixes** — actor/judge `extract_text()` handles thinking-model
  blocks; judge `max_tokens` 512→2048 (was truncating verdict JSON). These made
  the eval accurate on sonnet-5.

## Opt-in levers (default-OFF — measured null/negative on REAL data, kept off)
See `docs/internal/recall-lever-measurement-plan-2026-07-11.md` for the real
oracle results. Baseline recall@40 is already 98–100% → no headroom.
- `SPECTRAL_FTS_FUSION` — null lift on real multi-session; second index = pure cost.
- `SPECTRAL_NUMBER_NORMALIZE` — REGRESSED temporal −0.7pp (pool dilution).
- `SPECTRAL_QUERY_ALIASES` (empty no-op), `SPECTRAL_FTS_STOPWORDS`,
  `SPECTRAL_ANTICIPATORY_RECALL` — untested on real data; opt-in.

## Actor counting intervention (the one verified accuracy gain)
`docs/internal/actor-counting-intervention-2026-07-12.md`. Identity-keying +
inclusion-strictness in `counting_enumerate.md` / `counting_current_state.md`.
- Tuning set (20): 85% → 94.4%. Held-out (25): 72% → **80% (+8pp, 0 regressions)**.
- **Next:** port these patterns into Permagent's actor (needs Permagent repo);
  confirm on the full 133-question multi-session set (more budget).

## Local assets on this machine (NOT in git)
- Dataset: `~/spectral-local-bench/longmemeval/longmemeval_s.json` (500q, 278MB,
  from HF `xiaowu0162/longmemeval` → `longmemeval_s`, public/MIT).
  Held-out subset: `~/spectral-local-bench/heldout_ms.json` (25 multi-session).
- Ingested brains (reusable, skip re-ingest): work-dirs under
  `~/spectral-local-bench/`: `eval-multi`, `eval-heldout`, `oracle-hard`,
  `oracle-temporal`, `oracle-fusion`.
- A/B binaries: `~/spectral-local-bench/sba-baseline` (pre-intervention prompts),
  `sba-intervention` (current prompts).

## Toolchain notes
- Release build required for feasible LongMemEval ingest:
  `cargo build --release -p spectral-bench-accuracy` (no `ort` dep — builds fine,
  unlike the embedding crates that block `--workspace`).
- Graph tests: run `--test-threads=1` (Kuzu mmap contention under parallel).
- Oracle (retrieval, $0 no LLM): `spectral-bench-accuracy oracle --dataset … --work-dir … --categories … --max-questions … --fresh-brains`.
- Actor eval ($): `… run … --use-cascade --no-expand-queries --actor-model claude-sonnet-5 --judge-model claude-sonnet-5`. Reuse brains by keeping the work-dir and deleting only `checkpoint.json`.

## Resume checklist for next session
1. **API key**: the scratchpad `.apikey` does NOT persist across sessions — a
   fresh `ANTHROPIC_API_KEY` is needed for any actor run. Retrieval oracle is $0.
2. Highest-value next work (in order): (a) port counting patterns to Permagent's
   actor; (b) confirm counting intervention on full 133 multi-session; (c) the
   sum-question retrieval-completeness gap (total-$ questions miss price turns —
   a retrieval issue, not actor); (d) decide whether to merge this branch to main.
3. The recall levers are settled (measured, kept off) — do not re-litigate
   without new real-corpus evidence.
