# K=60→80 admission test — the RESIDUAL_FLOOR deep-rank lever, measured

Date: 2026-07-20 | main tip `efdd960` | $0, zero LLM calls (oracle only).

## What this answers

`RESIDUAL_FLOOR.md` (2026-06-15) ended with "Free retrieval levers exhausted: NO",
licensed by a tight cluster of load-bearing operands at pos **61, 63, 68** — just past
the K=60 cascade cutoff — in the fused-expanded candidate ordering. The licensed
experiment (bump K to 80, see if they admit) was never run. This runs it.

## Config caveat (read first)

RESIDUAL_FLOOR's exact configuration is **unreproducible today**: the persisted Haiku
expansion sample (EXPANSION_DELTA Appendix A) and the `fused-expansion-replay` binary
lived on `bench/stable-fail-2x2` / `exp/cascade-admission`, both deleted without merge;
no funded API key is available to regenerate an expansion cache. This test therefore
runs **today's shipped default retrieval** (shape-routed cascade, expansion OFF) on
current main — the configuration the library actually ships. Positions here are not
directly comparable to RESIDUAL_FLOOR's; the lever verdict is for the shipped path.

## Method

Paired oracle A/B, `--question-id` per case over the 30-case stable-fail set (cached
brains in `~/spectral-local-bench/oracle-hard/`, reused in both arms) plus `ba358f49`
(fresh per-turn ingest). Arms: default routing (K=60 for these counting/multi-session
shapes) vs `SPECTRAL_CASCADE_K=80`. Rank probes via `SPECTRAL_CASCADE_K=500`.
Operand keys verified against brain content (not just key-string match).

## Results

### The motivating cluster no longer exists on main

| operand (RESIDUAL_FLOOR class) | pos June (exp+fused) | rank now (shipped, exp-OFF) |
|---|---|---|
| 6d550036 `_2:t2` solo Data Mining project | 68 | **30 — IN at K=60** |
| gpt4_15e38248 `_4:t3` wobbly-leg fix | 63 | **19 — IN at K=60** |
| gpt4_7fce9456 `_3:t8` pool condo | 61 | 197 |
| gpt4_15e38248 `_3:t1` mattress | 158 | 302 |
| ba358f49 `_2:t2` "I'm 32" (vocab-mismatch) | 187 | 147 |

Two of the three deep-rank operands are already inside K=60 under the shipped config
(the June positions were measured under a different admission ordering AND expanded
queries, so no clean attribution — but plausibly the July ranking fixes: bounded-additive
recency, undated-no-max-boost, rerank dedup-before-diversity). The rest sit at 147–302,
far beyond any bounded K reach. **There is no rank-61–80 band of load-bearing operands
left for K=80 to harvest.**

### K=80 effect on the 31-case set

- **Answer-session recall: ZERO gain.** Not one new answer session admitted across all
  31 cases (session recall is the accuracy-gating metric; K-extension is additive, so it
  could only gain — it gained nothing).
- **Case unblocks: ZERO.** No load-bearing operand crosses into the retrieved set.
  6d550036 is now retrieval-complete at K=60 (its only blocker is in); gpt4_15e38248 and
  gpt4_7fce9456 remain blocked by ranks 302/197; ba358f49 remains the vocab floor.
- **Answer-key recall: +8.5pp pooled** (47.6% → 56.1%, +102 keys) — the known bloated
  proxy: every extra key was redundant evidence inside already-retrieved sessions.
- **Cost: +36.5% context tokens** on the K=60-routed cases (530,955 → 724,640 est).

(3 cases shape-routed to K=30 profiles jumped 30→80 under the env override — included
in the pooled numbers above, direction identical.)

## Verdict

**K=60→80 is REJECTED as a lever: +37% tokens buys zero new answer sessions and zero
case unblocks — pure redundant-evidence spend.** The deep-rank cluster that licensed it
in RESIDUAL_FLOOR has dissolved on current main: partly *already recovered* (2 of 3
operands now rank 19/30, inside K=60) and partly *sunk far past bounded reach* (197/302).
RESIDUAL_FLOOR's "one more bounded admission/K experiment" is hereby run and closed.

Remaining un-run levers from the 2026-07-20 deep-research pass: local weak-actor RERANK
A/B (designated-decisive; still blocked — ollama not installed), cross-encoder rerank
over the existing pool, hybrid dense for semantic-regime production workloads (a value
decision vs the no-embedding stance, not a benchmark question).
