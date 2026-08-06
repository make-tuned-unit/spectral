# Result — R14: frozen expansion on the eval path (2026-08-06)

Register row R14 (READY → DONE). Found by R11's identity gate:
`r11-render-ab-stage1-void-2026-08-05.md`.

## The defect, restated

Eval-path query expansion (shape-gated to Counting questions, ON by
default) is an LLM call sampled per run: two same-day runs of identical
code on byte-identical brains retrieved different SETS on 3/120 questions.
A second, quieter source: a transient expansion API failure silently fell
back to the unexpanded query (eprintln only). Both make "deterministic
retrieval" false across paid runs, and both had been present for every
prior paid comparison that left expansion on.

## What shipped

- `EvalConfig::expansion_cache` + `run --expansion-cache <json>` — replays
  a frozen `{question_id: expanded_query}` map. Loaded and validated
  before any brain is built or paid call made.
- **Fail-loud contract:** a cache miss fails the question; combining
  `--expansion-cache` with `--no-expand-queries` is rejected as
  contradictory. No silent fallback in cache mode — the silent paths are
  what R14 is.
- Enters `config_fingerprint` via serde, so arms/checkpoints cannot mix
  cached and live expansion.
- Pinned by `frozen_expansion_cache_replays_and_fails_loud_on_miss`
  (Counting-shaped fixture asserted, hit + miss branches).

## Frozen caches generated (key retired after today)

`~/spectral-local-bench/r11-2026-08-05/expansion-cache-{heldout,
validation,stage2}.json` — 120/120 each, Haiku, ~$0.075 total. LME-S
already had `expansion-cache.json` (oracle era). All four datasets with
paid history are now replayable with expansion ON at $0 marginal.

## Verification (live, ~$0.16)

`locomo_5_46` — Counting-shaped, one of the three questions that diverged
in R11's void run — run twice through the full eval path with
`--expansion-cache`: **retrieved keys byte-identical across runs** (60/60).
The exact failure that voided a $3 run is now structurally impossible in
cache mode.

## Standing guidance

Paired paid comparisons use `--expansion-cache` (expansion ON, frozen) or
`--no-expand-queries` (expansion off both arms) — never live expansion in
two arms that will be diffed. The oracle already had this discipline;
the eval path now matches it.
