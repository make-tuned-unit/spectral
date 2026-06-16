# Cost Benchmark — n=500 (Expansion ON)

**Status**: Measured artifact. Aggregation-only — no new runs.
**Source**: `~/spectral-local-bench/eval-report-n500.json` (per-question
token/latency/cost instrumentation from PR #163).
**Run**: actor `claude-sonnet-4-6`, judge `claude-sonnet-4-6`, retrieval
path `cascade`, expansion ON (Haiku). Started 2026-06-15T17:31Z, completed
2026-06-16T01:00Z (duration 26,919 s ≈ 7h28m).
**Spectral version**: 0.0.1.

## Sample

| | Count |
|---|---|
| Total questions | 500 |
| Evaluated with full instrumentation | **492** |
| Excluded (transport failures, no `efficiency` block) | 8 |

The 8 excluded are all transport failures (5 single-session-preference,
2 multi-session, 1 single-session-assistant) — the actor/expansion calls
never completed, so no token/cost record exists. All efficiency aggregates
below are over n=492. Percentiles use the harness method: sorted array,
index `round(p·(n−1))` (verified to reproduce the report's precomputed
mean/median/p95 exactly).

---

## 1. Memory-layer overhead per query — THE headline

The marginal token cost of *remembering* (the retrieval/memory machinery),
**excluding the actor/synthesis call and the judge**. For Spectral this is
exactly the one pre-retrieval Haiku query-expansion call; FTS recall and
deterministic re-ranking are CPU-local and consume **zero** LLM tokens.

| Metric | Mean | Median (p50) | p95 | Min | Max |
|--------|------|------|-----|-----|-----|
| **Memory-layer overhead (tokens)** | **169.1** | **166** | **198** | 0 | 233 |
| ↳ expansion input tokens | 133.7 | 131 | 154 | — | — |
| ↳ expansion output tokens | 35.4 | 35 | 47 | — | — |
| **Memory-layer overhead ($/query)** | **$0.000249** | — | — | — | — |
| **Memory-layer overhead ($/1k queries)** | **$0.25** | — | — | — | — |

Cost computed at the documented Haiku rate ($0.80/MTok input,
$4.00/MTok output) — the same rate embedded in the harness's per-question
`estimated_cost_usd` (verified to the cent against the raw token counts).

**Zero-LLM-recall receipt**: `cascade_telemetry.total_recognition_token_cost`
is **0 across all 492 questions** (sum = 0, max = 0). This is the structural
proof that `Brain::recall_*()` makes no LLM call — the entire memory-layer
overhead is the optional expansion call and nothing else. With expansion OFF,
this figure is structurally $0.00 / 0 tokens.

### Context vs. competitors (read the mismatch flag)

The dispatch frames this against **Mem0's self-reported ~6,900 tokens/query**.
At face value 169 vs 6,900 is **~41× fewer tokens**. But the two numbers are
**not the same metric** — see the definition-mismatch flags below before
quoting this ratio. Spectral's 169 is retrieval-machinery overhead only;
Mem0's ~6,900 is (almost certainly) total prompt context delivered to the
reader, which corresponds to Spectral's `system_tokens_per_query` (§2), not
this number.

---

## 2. system_tokens_per_query (expansion + actor, excl. judge)

Total tokens in the context window delivered to the actor per question
(expansion call + actor call; judge excluded as an instrument).

| Metric | Mean | Median (p50) | p95 |
|--------|------|------|-----|
| system_tokens_per_query | **16,553.9** | **15,476** | **25,476** |

This is dominated by the actor call's retrieved-context input — the cost of
*answering*, common to every memory system. It is the correct axis to
compare against competitors' "tokens per query" figures (§5).

---

## 3. Retrieval latency per query (wall time)

Wall-clock time inside the retrieval pipeline (`retrieval_wall_ms`) — the
no-embedding-infrastructure speed story. No network round-trip, no vector
DB, no embedding model.

| Metric | Mean | Median (p50) | p95 | Min | Max |
|--------|------|------|-----|-----|-----|
| Retrieval latency (ms) | **18.1** | **17** | **42** | 1 | 54 |

Sub-20 ms median, sub-55 ms tail, entirely CPU-local (SQLite FTS5 + Kuzu).

---

## 4. Total campaign cost (system vs. judge separated)

| Component | Total (USD) | Per query (n=492) | Note |
|-----------|-------------|-------------------|------|
| **System total** (actor + expansion) | **$26.30** | $0.05345 | the system under test |
| ↳ actor (Sonnet 4.6) | $26.18 | $0.05320 | cost of answering |
| ↳ expansion (Haiku) | $0.12 | $0.00025 | cost of remembering (= §1) |
| **Judge** (Sonnet 4.6) | **$1.55** | $0.00316 | **instrument, excluded from headline** |

The judge is a measurement instrument, not a system cost — kept out of every
headline figure per the methodology. The entire memory-layer (expansion)
campaign cost across 492 questions was **$0.12**: the memory machinery is
~0.5% of system cost; the actor's synthesis call is the other ~99.5%.

---

## 5. Per-category cost

| Category | n | Mem-overhead tok (mean / p95) | system_tok (mean) | Retrieval ms (mean) | System cost | $/query |
|----------|---|------|------|------|------|------|
| temporal-reasoning | 133* | 168.5 / 192 | 14,583 | 6.46 | $6.05 | $0.0455 |
| multi-session | 131 | 165.8 / 182 | **20,764** | 28.15 | **$9.15** | **$0.0699** |
| knowledge-update | 78 | 165.2 / 176 | 16,897 | 19.72 | $4.28 | $0.0549 |
| single-session-user | 70 | 161.0 / 176 | 14,412 | 15.66 | $3.13 | $0.0447 |
| single-session-assistant | 55 | 195.3 / 218 | 14,247 | 17.89 | $2.43 | $0.0442 |
| single-session-preference | 25 | 167.2 / 176 | 14,979 | 28.84 | $1.25 | $0.0499 |

*n per category is the instrumented count (n=492 total); transport failures
are dropped per category. See dataset-count flag below.

**Does any category cost disproportionately?** Yes — **multi-session**.
It is the most expensive per query ($0.070, ~57% above the cheapest
categories) and the largest share of campaign cost ($9.15 / $26.30 = 35%).
The driver is `system_tokens_per_query` (20,764 mean vs ~14.5k for the
single-session categories): multi-session questions retrieve and inject far
more context into the actor call. The *memory-layer overhead* itself is flat
across categories (161–195 tokens) — the expansion call is roughly
constant-cost regardless of category. The cost spread is an actor-input-size
story, not a memory-machinery story.

---

## Cross-system definition mismatches (flags)

These must accompany any competitor comparison. The headline "169 vs 6,900"
is **not** a clean apples-to-apples claim.

1. **Mem0 ~6,900 tok/q ≠ Spectral 169 tok/q (overhead).** Mem0's
   self-reported per-query token figure is, by its own framing, the *total
   prompt context* sent to the reader LLM (their "tokens consumed" includes
   the injected memories). That corresponds to Spectral's
   **`system_tokens_per_query` = 16,554** (§2), *not* the memory-layer
   overhead. On that comparable axis Spectral is **higher**, not lower —
   Spectral injects more retrieved context into the actor. The genuinely
   favorable Spectral claim is the **memory-layer overhead** (169 tok, one
   Haiku call), for which **no competitor publishes a comparable isolated
   number** (all marked "Undisclosed" in the methodology's comparison table).
   Do not present 169 vs 6,900 without this caveat.

2. **Overhead isolation is unique to Spectral.** Spectral can separate
   "cost of remembering" (169) from "cost of answering" (16,554) because the
   memory layer is a single, instrumented, optional LLM call. Mem0/Zep/Letta
   entangle retrieval LLM/embedding calls with the agent loop and do not
   report the retrieval-machinery slice in isolation. The comparison table
   therefore lists their overhead as "Undisclosed" — that is the honest
   state, and this run does not change it.

3. **Ingest-time and embedding costs excluded (asymmetric across systems).**
   Spectral's overhead has no query-time embedding call; Mem0/Zep/Letta each
   make ≥1 embedding call at query time whose token/$ cost is not in their
   ~6,900 figure. Conversely, Spectral pays ingest-time TACT/FTS indexing
   cost (zero LLM, nonzero CPU) not counted here. Per-query overhead is the
   marginal-query metric; ingest is amortized and not commensurable.

4. **Accuracy is not actor-controlled across systems.** Raw LongMemEval/
   LOCOMO accuracy mixes retrieval quality and actor strength. Mem0's
   "+26pp" is LOCOMO over an OpenAI baseline; Spectral's 80.9% is
   LongMemEval-S with a Sonnet actor. Not directly comparable.

5. **Dataset category-count drift (minor).** The methodology's published
   category counts (single-session-assistant 56, single-session-preference
   30) differ from this run's instrumented counts (55, 25). The deltas are
   the transport-failure drops (8 total) plus one assistant question. Cost
   per-category above uses the *instrumented* counts; accuracy tables in the
   methodology should reconcile separately.

---

## Provenance

- All figures aggregated from `eval-report-n500.json` (`results[].efficiency`
  and `results[].cascade_telemetry`), n=492 instrumented.
- Mean/median/p95 reproduce the report's precomputed `efficiency` block
  exactly (same percentile method).
- Competitor figures: self-published, as recorded in
  `docs/internal/BENCH_METHODOLOGY.md` §f (Architecture Comparison /
  Derived: Per-Query Overhead Cost). This artifact adds no new competitor
  numbers — it fills Spectral's measured row and flags the mismatches.
