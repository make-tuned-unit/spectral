# Spectral — Benchmark Results (LongMemEval-S, n=500)

**Headline: 81.5% accuracy (401/492)** on LongMemEval-S, with a memory-layer
retrieval overhead of **~169 tokens/query** and **~17 ms median retrieval
latency** — no embedding model, no vector database, no LLM call in the recall
path.

This is the citable summary. Full method, integrity checks, and limitations
are in [`internal/BENCH_METHODOLOGY.md`](internal/BENCH_METHODOLOGY.md).

---

## Accuracy

| | Value |
|---|---|
| **Overall accuracy** | **81.5%** (±2pp) |
| Correct / clean denominator | 401 / 492 |
| Dataset | LongMemEval-S (500 questions, 6 categories, 246,930 turns) |
| Actor / Judge | Claude Sonnet 4.6 / Claude Sonnet 4.6 |
| Configuration | cascade retrieval, expansion-ON (Haiku 4.5), `--max-results 40` |
| Run | main `cdd793e` (#172), 2026-06-15→16 |

**Denominator note (n=492).** 8 of 500 questions hit transport (network)
failures and are quarantined from accuracy — they neither pass nor fail.
Accuracy = correct / (500 − 8) = 401 / 492. This is the pre-registered
"clean denominator" rule (no silent inflation).

**Integrity-checked count.** The on-disk run scored 398/492 (80.9%) as-judged.
A blast-radius audit ([`internal/pr-173-blast-radius-verification.md`](internal/pr-173-blast-radius-verification.md))
found the actor occasionally appended a fabricated prompt-continuation that
the judge mis-graded. Applying the shipped sanitizer and **re-judging only
(no actor re-run)** corrected 3 true-negative→correct flips with **0 false
positives** → **401/492 = 81.5%**. Every one of the 22 artifact-carrying
"correct" cases was re-verified.

### Per-category

`n` is the clean (evaluated) denominator per category.

| Category | n | Correct | Accuracy |
|----------|---|---------|----------|
| single-session-assistant | 55 | 51 | 92.7% |
| knowledge-update | 78 | 68 | 87.2% |
| single-session-user | 70 | 60 | 85.7% |
| temporal-reasoning | 133 | 110 | 82.7% |
| multi-session | 131 | 98 | 74.8% |
| single-session-preference | 25 | 14 | 56.0% |
| **Overall** | **492** | **401** | **81.5%** |

---

## Cost & latency (memory-layer)

The differentiator is not just accuracy but what it costs to *remember*.
Spectral's recall path is deterministic (SQLite FTS5 + TACT fingerprint +
re-ranking) — **zero LLM calls, zero tokens**. The only memory-layer LLM cost
is the optional pre-retrieval query-expansion call (Haiku).

| Metric | Mean | p50 | p95 |
|--------|------|-----|-----|
| **Memory-layer overhead** (tokens/query) | **169** | 166 | 198 |
| Retrieval latency (ms) | 18.1 | **17** | 42 |
| system_tokens_per_query (context to actor) | 16,554 | 15,476 | 25,476 |

- **Memory-layer overhead ≈ $0.25 / 1,000 queries** (Haiku expansion only).
  With expansion OFF it is structurally $0.00 / 0 tokens.
- Retrieval is fully CPU-local: no embedding inference, no vector DB, no
  network round-trip. `total_recognition_token_cost = 0` across all 492
  questions is the structural receipt.

Full breakdown (per-category cost, campaign totals, cross-system definition
flags): [`internal/COST_BENCHMARK.md`](internal/COST_BENCHMARK.md).

---

## Honest framing

We state these up front; they bound how the number should be read.

- **In-sample.** Spectral's retrieval/routing was developed against
  LongMemEval-S. This 81.5% is therefore in-sample (training-distribution)
  performance; held-out performance on a different distribution is expected to
  be **lower**. This is not a state-of-the-art claim.
- **single-session-preference rests on n=25** (after transport drops) and is
  the lowest and most volatile category (56.0%); small-n swings dominate it.
- **Failures are synthesis-dominated, not retrieval-dominated.** Retrieval
  recall is 93–97% across categories
  ([`internal/N500_FAILURE_ANALYSIS.md`](internal/N500_FAILURE_ANALYSIS.md),
  [`internal/MISSING_KEY_AUTOPSY.md`](internal/MISSING_KEY_AUTOPSY.md)); the
  remaining gap is mostly the actor failing to synthesize a correct answer
  from correctly-retrieved context, not the memory layer missing evidence.
- **±2pp noise band.** Default sampling (no pinned temperature/seed); ~2.4% of
  questions are stochastic coin-flips. Differences under 2pp are noise.
- **Competitor comparisons are not apples-to-apples.** Actor models differ or
  are undisclosed across systems, and no competitor publishes an isolated
  retrieval-machinery token cost. See the cost doc's mismatch flags.

---

## Detailed analyses

- [`internal/BENCH_METHODOLOGY.md`](internal/BENCH_METHODOLOGY.md) — full method, scoring integrity, results tables, limitations
- [`internal/COST_BENCHMARK.md`](internal/COST_BENCHMARK.md) — token/latency/cost breakdown
- [`internal/N500_FAILURE_ANALYSIS.md`](internal/N500_FAILURE_ANALYSIS.md) — failure autopsy
- [`internal/MISSING_KEY_AUTOPSY.md`](internal/MISSING_KEY_AUTOPSY.md) — retrieval-miss taxonomy
- [`internal/RESIDUAL_FLOOR.md`](internal/RESIDUAL_FLOOR.md) — residual-failure floor
- [`internal/pr-173-blast-radius-verification.md`](internal/pr-173-blast-radius-verification.md) — integrity check behind 401/492
- [`internal/ROLE_TOKEN_PROBE.md`](internal/ROLE_TOKEN_PROBE.md) — assistant-turn token-reduction probe (shelved: −40% tokens held recall but regressed SSA accuracy)
