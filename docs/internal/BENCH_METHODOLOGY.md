# Spectral Accuracy Benchmark: Methodology

**Status**: Publication artifact. Results filled from the n=500 run.
**Main SHA**: `f29c6c1` (all file:line citations on this SHA unless noted)
**Date**: 2026-06-10
**n=500 run**: executed on main `cdd793e` (#172), 2026-06-15→16
(`started_at` 2026-06-15T17:31Z → `completed_at` 2026-06-16T01:00Z),
actor & judge `claude-sonnet-4-6`, expansion-ON (Haiku 4.5), cascade path,
`--max-results 40`, per-turn ingest. Report:
`~/spectral-local-bench/eval-report-n500.json` (`spectral_version` 0.0.1).
The run predates the #173 prompt-continuation strip; the integrity-checked
401/492 applies that sanitizer to the on-disk answers via re-judge (no actor
re-run) — see `pr-173-blast-radius-verification` and §f. **This campaign is
expansion-ON only; the expansion-OFF ablation was not executed.**

---

## a. System Under Test

Spectral is a frequency-domain memory system for AI agents. It stores
memories in SQLite (FTS5 full-text index + constellation fingerprints) and
an embedded Kuzu graph database. Retrieval is a deterministic pipeline:
TACT (Topic-Aware Context Triage) routes queries through fingerprint
hash lookup, wing-scoped search, and FTS fallback, then merges results
through an additive re-ranking pipeline (signal score weighting, recency
decay, ambient boost, declarative density, co-retrieval boost, episode
diversity). No embedding model, no vector database, no neural inference
of any kind runs inside the library recall path.

The bench evaluates this pipeline against LongMemEval-S (500 questions,
6 categories, 246,930 conversation turns) with a Claude Sonnet 4.6 actor
(`claude-sonnet-4-6`) reading the retrieved context and answering, and a
Claude Sonnet 4.6 judge evaluating correctness against ground truth.

**Zero-LLM library recall path**: No `Brain::recall_*()` method makes an
LLM call. The field `total_recognition_token_cost` on `CascadeResult`
(`spectral-cascade/src/result.rs:13-17`) exists as a load-bearing receipt
for this commitment. It is structurally set to 0 at `brain.rs:1232`.
Integration tests assert it equals 0 (`brain_tests.rs:793,825,942`). The
cascade pipeline (`cascade_layers.rs:138-210`) contains no LLM invocation.

**Consumer-side expansion (separate from library recall)**: The bench's
expansion-ON configuration adds one pre-retrieval Haiku call per query
*before* the `Brain` entry point. This call lives in `spectral-bench-accuracy`
(consumer code), not in any Spectral library crate. The expansion-OFF
ablation removes this call entirely, producing a fully zero-LLM pipeline
end-to-end. Both configurations are published so readers can see the
boundary clearly.

---

## b. Configurations

Two configurations are benchmarked. Both run all 500 questions across all
6 LongMemEval-S categories.

### Baseline: Expansion OFF (zero-LLM end-to-end)

```bash
ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
cargo run --release -p spectral-bench-accuracy -- run \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --work-dir ~/spectral-local-bench/eval-work-n500-noexp \
  --output ~/spectral-local-bench/eval-report-n500-noexp.json \
  --use-cascade \
  --actor-model claude-sonnet-4-6 \
  --judge-model claude-sonnet-4-6 \
  --base-url https://api.anthropic.com \
  --max-results 40 \
  --ingest-strategy per_turn \
  --no-expand-queries \
  --confirm-cost
```

This is the zero-LLM retrieval configuration. No LLM call of any kind
occurs between the user's question and the assembled context window.
Memory-layer overhead is structurally 0 tokens.

### With expansion: Expansion ON (default bench configuration)

```bash
ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
cargo run --release -p spectral-bench-accuracy -- run \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --work-dir ~/spectral-local-bench/eval-work-n500 \
  --output ~/spectral-local-bench/eval-report-n500.json \
  --use-cascade \
  --actor-model claude-sonnet-4-6 \
  --judge-model claude-sonnet-4-6 \
  --base-url https://api.anthropic.com \
  --max-results 40 \
  --ingest-strategy per_turn \
  --expansion-model claude-haiku-4-5-20251001 \
  --confirm-cost
```

Pre-retrieval query expansion generates up to 10 additional search terms
via Claude Haiku 4.5 (`main.rs:93,358`). These terms produce parallel
FTS queries whose results are fused with the primary retrieval. Expansion
is on by default in the bench CLI (`main.rs:88-90,351`) but OFF by
default in the Spectral library.

### Why both are published

Expansion is the only LLM call in the bench pipeline and represents the
only non-zero cost in memory-layer overhead. Publishing both lets readers
see: (1) the accuracy delta attributable to expansion alone, (2) the cost
of that delta, and (3) a structurally zero-overhead baseline that runs the
full deterministic pipeline with no model dependency.

### Shape routing

With `--use-cascade`, the bench uses per-question-type routing
(`retrieval.rs:199-206`): Temporal questions route to `topk_fts`
(cascade hurts temporal by ~15pp); all other shapes route through the
cascade pipeline. Without `--use-cascade`, all questions use `topk_fts`
with no shape routing — this is NOT the intended configuration.

### Per-type K profiles (cascade path)

| Question Type | K | max_per_episode | recency_half_life_days |
|---------------|---|-----------------|------------------------|
| Counting / CountingCurrentState | 60 | 3 | 730 |
| Temporal | 40 | 5 | 60 |
| Factual / FactualCurrentState | 30 | 8 | 365 |
| General / Preference / Recall | 40 | 5 | 365 |

Source: `retrieval.rs:176-196`.

---

## c. Scoring Integrity

### Clean denominator

The denominator for accuracy is the number of questions that received a
valid actor + judge response. Transport failures (network timeouts, 5xx
errors) are excluded from accuracy and reported separately. Auth failures
are excluded. Questions that fail on the first attempt but succeed after
retry are counted normally — their accuracy reflects the retried response.

Source: `eval.rs:423-435`, `report.rs:175-211`.

### outcome_class quarantine

Questions classified as `outcome_class: transport_failure` are quarantined
from accuracy computation entirely. They appear in the report JSON as
`transport_failures: N` and `auth_failures: N` but do not inflate or
deflate the accuracy percentage. The formula is:

```
accuracy = correct / (total - transport_failures - auth_failures)
```

This is the "clean denominator" referenced throughout.

### Retry policy

Max 4 attempts per question (1 initial + 3 retries) with exponential
backoff (`eval.rs:418`, `retry.rs:59`). If all 4 attempts fail, the
question is classified as a transport failure and quarantined.

### Sampling configuration and precision statement

Actor and judge API calls use default sampling parameters (no explicit
`temperature` or `seed` set). This means two runs on the same question
will produce different actor responses. The bench measures expected
behavior under the model's default stochasticity, not a deterministic
pin.

The precision of all reported accuracy numbers is bounded by the measured
noise band: **+/-2pp**. This was established empirically via the n=5
variance discipline (below). Any accuracy difference smaller than 2pp
between configurations or runs is within noise and should not be
interpreted as signal.

### n=5 variance discipline

Established via the denoised variance analysis (VARIANCE_ANALYSIS_FINAL.md):
41 questions from the failure set were each run 5 times with frozen
retrieval context (replay-actor mode). Results:

| Class | Count | Definition |
|-------|-------|------------|
| STABLE_FAIL | 26 | 0/5 pass rate — deterministic failures |
| STOCHASTIC | 12 | 1-4/5 pass rate — model-variance sensitive |
| STABLE_PASS | 3 | 5/5 pass rate — deterministic passes |

The provisional 2-run classification was correct for 13/14 upgraded
questions (93% accuracy at n=2), with one reclassification
(`a96c20ee_abs`: STABLE_FAIL → STOCHASTIC at 2/5). 12 of 500 questions
(2.4%) are stochastic coin-flips under default sampling, producing the
+/-2pp noise band.

### Judge model identity

Judge: `claude-sonnet-4-6` (`main.rs:67`). The judge uses 8 shape-specific
prompt templates with a reasoning-aware rubric (PRs #134, #138). Judge
cost is NOT included in memory-layer overhead calculations — it is a
measurement instrument cost, not a system cost.

---

## d. Efficiency Metrics

### Memory-layer overhead per query

All tokens consumed and API calls made by the retrieval/memory machinery,
EXCLUDING the final actor/synthesis call. This isolates the cost of
*remembering* from the cost of *answering*.

For Spectral, this includes:
- Query expansion LLM call (Haiku, ~200 output tokens) — when enabled
- Retrieval: **0 LLM calls, 0 tokens**. FTS + deterministic re-ranking
  pipeline. All CPU-local.

For Spectral, this excludes:
- The final actor call (common to all systems)
- Judge calls (measurement instrument)
- Ingest-time costs (see "What this metric excludes" below)

**Receipt**: `total_recognition_token_cost` is always 0 across all bench
runs (`result.rs:13-17`, `brain.rs:1232`). This field is the structural
proof that no LLM call occurs inside `Brain::recall_*()`.

#### What this metric excludes and why

**Ingest-time costs are excluded.** Spectral's ingest pipeline (TACT
classification, SHA-256 fingerprint generation, FTS indexing, signal
scoring) runs at memory-write time, not query time. These are non-trivial
CPU operations but involve zero LLM calls. Other systems pay different
ingest costs: Mem0/Zep embed each memory (embedding model call), Letta
uses LLM calls to decide what to store. These ingest costs are amortized
over all future queries against that memory and are not commensurable
across architectures. Per-query overhead is the marginal cost of one
additional question — the metric most relevant to production cost planning.

**Infrastructure fixed costs are excluded.** Spectral uses local SQLite
and Kuzu files (no hosting cost). Hosted systems (Mem0 Cloud, Zep Cloud,
Memanto) have per-seat or per-usage infrastructure costs not captured here.

### system_tokens_per_query

Total tokens in the context window delivered to the actor per question.
This is the retrieval pipeline's output size, measured as `estimated
tokens = content_len / 4 + 5` per hit (`result.rs:9-10`). Reported as
an aggregate (mean, p50, p95) across all 500 questions.

### Null-over-estimate rule

When Spectral expansion is OFF, memory-layer overhead is structurally 0.
This is not a measurement — it is an architectural property. You cannot
over-estimate zero. When expansion is ON, overhead equals exactly the
Haiku expansion call's token count (logged per question). There is no
hidden cost to under-count because the pipeline has no other LLM calls.

### Cost computation

```
overhead_cost_per_query = expansion_input_tokens * haiku_input_price
                        + expansion_output_tokens * haiku_output_price
```

Haiku pricing (as of 2026-06-10): $0.80/MTok input, $4.00/MTok output.
With expansion OFF: $0.00/query.

---

## e. Known Limitations

These are stated by us, proactively, before publication.

### 1. Reader-bound failure mass

The dominant failure mode is not retrieval — it is actor-side synthesis.
Retrieval recall is 93-97% across categories (ALL_CATEGORY_RECALL_AUDIT.md).
The remaining accuracy gap is driven by the actor model's ability to
synthesize correct answers from correctly-retrieved context. This is
particularly acute for multi-session counting questions, where 62% of
stable failures are WRONG_FACT errors (the evidence was retrieved but the
actor miscounted or misidentified entities).

The STABLE_FAIL 2x2 decomposition (branch `bench/stable-fail-2x2`) is
pending and will attribute failures to DILUTION (retrieval noise),
MODEL_BOUND (reader separation needed), or CEILING (bench instrument
limit). Until this completes, the failure attribution is incomplete.

**Impact**: The headline accuracy number reflects both Spectral's
retrieval quality AND Claude Sonnet 4.6's synthesis quality. A different
actor model would produce a different accuracy number from identical
retrieval. We cannot fully separate retrieval contribution from reader
contribution until the 2x2 decomposition ships.

### 2. Single-dataset scope

All accuracy numbers are measured on LongMemEval-S (500 questions,
246,930 conversation turns, 6 categories). This is the most comprehensive
published agent-memory benchmark available, but it is one dataset with
one distribution of question types. Performance on other distributions
(e.g., domain-specific knowledge bases, longer conversation histories,
non-English content) is unmeasured.

### 3. Self-run numbers, iterative development on test set

This benchmark was run by the Spectral team on the Spectral codebase.
No independent third party has reproduced these results. The harness,
dataset, and instructions are public (see Reproducibility section), but
until an external party runs the bench independently, these are
first-party numbers.

Additionally, the K profiles, question-type routing rules, prompt
templates, and re-ranking weights were developed iteratively against this
dataset during the investigation arc (PRs #134-#159). This means the
reported accuracy is closer to training-set performance than held-out
performance. The variance analysis and denoised eval set partially
mitigate overfitting concerns, but the iterative development is
acknowledged.

### 4. Two-category measured baseline only

The existing measured result (77.8% on MS+SSP, n=153) covers only 2 of
6 categories. The strong categories (SSU, SSA, KU) have 98-100%
retrieval recall and are expected to score higher, but this is projected,
not measured. The all-category n=500 number is the first corpus-wide
measurement. An earlier RUN_NOTES recall-map projection of ~73–74% has been
**superseded** (see "Pre-registered expectation" below): it sat *below* an
already-measured expansion-OFF full-500 run (77.6% accuracy, 2026-05-18) under
a stricter-or-equal judge, so it understated the floor.

### 5. Actor-side synthesis gap (counting, temporal)

The extract-operate pipeline for operational questions (counting, date
math) was investigated and shelved at 7/26 best-case (RUN_NOTES). No
cheap architectural lever exists for this failure class. This means:
- Multi-session counting questions have a structural accuracy ceiling
  bounded by LLM counting ability over long context
- Temporal reasoning depends on the actor's date-math capability
- These ceilings are model-capability-bound, not architecture-bound

### 6. Expansion model dependency

The expansion-ON configuration depends on Claude Haiku 4.5 availability
and pricing. Model deprecation, pricing changes, or API changes would
affect reproducibility. The expansion-OFF configuration has no external
model dependency and is the model-independent baseline.

### 7. No cross-system head-to-head

We do not run Mem0, Zep, Memanto, or Letta on the same dataset with the
same actor/judge. Comparisons in the architecture table use self-reported
numbers from those systems' publications, which may use different datasets,
models, and methodology. Direct accuracy comparison should be read with
this asymmetry in mind.

### 8. Judge-actor model overlap

The judge (`claude-sonnet-4-6`) is the same model family and version as
the actor. This creates a potential evaluation bias: the judge may be
more lenient toward responses that match its own reasoning style or
phrasing patterns. The reasoning-aware judge rubric (PRs #134, #138)
mitigates this partially by requiring explicit evidence citation and
structured correctness criteria rather than open-ended quality assessment.

The judge model was held constant across all prior measurements (V3
baseline, expansion runs, variance analysis) to preserve internal
comparability. Changing the judge mid-arc would invalidate all
delta-over-baseline claims.

**Planned mitigation**: Post-bench judge-agreement audit on a 100-verdict
sample re-judged by a different model family (e.g., GPT-4o or Gemini).
The inter-judge agreement rate will be published alongside the results
to quantify the bias surface.

---

## e′. Pre-registered Expectation (recorded before the n=500 publish run)

This is the pre-registration: it is written **before** the headline n=500 run
and is published with the result **regardless of outcome**, alongside the cost
table and the per-category breakdown.

### Expected accuracy

- **Expansion-ON primary: point ~79–80%, range 79–82%.**
- **Expansion-OFF ablation: ~77–79%.**

**Basis (Projection B — the honest-attribution estimate).** A full-500 run on
2026-05-18 (cascade, expansion OFF, `claude-sonnet-4-6` actor/judge) measured
**77.6% accuracy** (388/500; per-category KU 83.3%, MS 64.7%, SSA 91.1%,
SSP 60.0%, SSU 91.4%, TR 78.2% — judged pass-rate, not recall). That run's
judge predates #138 ("accept superset answers"), which is more lenient and is
present on the run-target main, so 77.6% is a **stricter-or-equal floor**.
Expansion's measured contribution is **+5.3pp on MS+SSP** (72.5%→77.8%,
RUN_NOTES); MS+SSP is 32.6% of the set, so the full-set lift is ~+1.7pp →
**~79.3%** expansion-ON. The upper edge of the 79–82% band assumes MS+SSP lands
at its clean-denominator expansion-ON level (MS 78.6%, SSP 72.7%) with the
strong categories holding.

We do **not** adopt the denominator-inflated per-category stitch (which reaches
82.1%) as the point estimate: roughly half of its MS gain is the
clean-denominator switch, not expansion.

### Framing tier (at ~80%)

Accuracy is stated **plainly** ("~80% on LongMemEval-S, n=500") and **paired
with the memory-layer cost/overhead figure as a co-headline** — the claim is
accuracy-at-this-cost, not accuracy alone. No "state-of-the-art" or ranking
claim is made against systems not run head-to-head (see Limitation 7).

### Caveats specific to this expectation

- **In-sample.** The K profiles, shape-routing rules, prompt templates, and
  re-ranking weights were tuned against this exact 500-question set (PRs
  #134–#159; see Limitation 3). The published number is therefore in-sample
  (training-set) performance; held-out performance on a different distribution
  is expected to be lower.
- **SSP rests on n=22.** The "current" single-session-preference accuracy
  feeding the projection comes from the expansion-ON clean-denominator run where
  SSP had only 22 evaluated questions — the least stable input, and SSP is the
  lowest-scoring category.

---

## f. Results Tables

All accuracy numbers carry a measurement precision of **+/-2pp** from
stochastic model variance (see Sampling Configuration, section c). Any
delta smaller than 2pp is within noise.

### Overall Accuracy

| Configuration | n | Correct | Accuracy (+/-2pp) | Clean Denominator | Transport Failures |
|---------------|---|---------|-------------------|-------------------|--------------------|
| Expansion OFF (zero-LLM) | — | not run | not run | — | — |
| Expansion ON (+ Haiku) | 500 | **401** | **81.5%** | 492 | 8 |
| **Ablation delta** | | | **not run** | | |

Expansion ON is the published configuration. `correct=401` is the
integrity-checked count: the on-disk run scored 398/492 (80.9%) as-judged;
the #173 blast-radius verification re-judged the prompt-continuation artifact
cases (sanitizer applied, no actor re-run) and confirmed +3 true-negative→
correct flips with 0 false positives → **401/492 = 81.5%**. The 8 transport
failures (network, quarantined) give the clean denominator 492.

### Per-Category Accuracy (Expansion OFF)

Not run in this campaign (expansion-ON only).

### Per-Category Accuracy (Expansion ON) — integrity-checked

`n` is the clean (evaluated) denominator: dataset count minus that category's
transport failures (8 total: 5 SSP, 2 MS, 1 SSA). Accuracy = correct / n.

| Category | n | Correct | Accuracy (+/-2pp) |
|----------|---|---------|-------------------|
| single-session-user | 70 | 60 | 85.7% |
| single-session-assistant | 55 | 51 | 92.7% |
| knowledge-update | 78 | 68 | 87.2% |
| temporal-reasoning | 133 | 110 | 82.7% |
| multi-session | 131 | 98 | 74.8% |
| single-session-preference | 25 | 14 | 56.0% |
| **Overall** | **492** | **401** | **81.5%** |

The integrity-check +3 vs the as-judged run lands on SSA (+1, `8b9d4367`),
multi-session (+1, `55241a1f`), and SSP (+1, `b6025781`).

### Ablation Delta by Category

Not computable — the expansion-OFF arm was not run.

### Efficiency Aggregates (Expansion OFF)

Not run. By construction memory_layer_overhead_tokens would be 0 (no LLM call
in the recall path); system_tokens_per_query and retrieval_latency_ms would
match the ON run minus the expansion call (retrieval is identical).

### Efficiency Aggregates (Expansion ON)

n=492 instrumented. Percentile method: sorted array, index `round(p·(n−1))`
(reproduces the report's precomputed aggregates). See `COST_BENCHMARK.md`.

| Metric | Mean | p50 | p95 |
|--------|------|-----|-----|
| system_tokens_per_query | 16,554 | 15,476 | 25,476 |
| expansion_input_tokens | 134 | 131 | 154 |
| expansion_output_tokens | 35 | 35 | 47 |
| memory_layer_overhead_tokens | 169 | 166 | 198 |
| retrieval_latency_ms | 18.1 | 17 | 42 |

Campaign cost: system $26.30 (actor $26.18 + expansion $0.12), judge $1.55
(instrument, excluded from headline). Memory-layer overhead ≈ $0.25/1k queries.

### Architecture Comparison

This table compares what each system does at query time. The
architectural differences drive the cost differences in the derived
table below.

| System | Embedding at query time | LLM calls in retrieval | Retrieval method | Infra dependencies |
|--------|------------------------|----------------------|------------------|-------------------|
| **Spectral** (expansion OFF) | None | 0 | FTS5 + TACT fingerprint + deterministic re-ranking | SQLite + Kuzu (local files) |
| **Spectral** (expansion ON) | None | 1 (Haiku, pre-retrieval) | FTS5 + TACT fingerprint + deterministic re-ranking | SQLite + Kuzu (local files) |
| **Mem0** | 1 embedding call | Undisclosed | Vector + graph + KV fusion | Qdrant + embedding model (hosted or self-hosted) |
| **Zep** | 1 embedding call | Undisclosed | Vector + graph + full-text hybrid | PostgreSQL + Neo4j + embedding model |
| **Memanto** | Undisclosed | Undisclosed | Undisclosed (closed-source) | Hosted service |
| **Letta** | 1+ embedding calls | 0-N (agent-controlled) | Agent tool calls → embedding + archival search | Letta server + PostgreSQL + embedding model |
| **Mastra** | 1 embedding call (developer-chosen) | Developer-dependent | Vector search (framework) | Developer-chosen vector DB + embedding model |

### Derived: Per-Query Overhead Cost

| System | Overhead tokens/query | $/1k queries (overhead) | Accuracy (LongMemEval) | Accuracy source |
|--------|----------------------|------------------------|------------------------|-----------------|
| **Spectral** (expansion OFF) | 0 | $0.00 | not run | — (ablation not executed this campaign) |
| **Spectral** (expansion ON) | 169 | $0.25 | 81.5% (+/-2pp) | This bench (n=492 clean, integrity-checked) |
| **Mem0** | Undisclosed | Undisclosed | +26pp over OpenAI baseline (LOCOMO) | Self-reported (mem0.ai blog, methodology partial) |
| **Zep** | Undisclosed | Undisclosed | Not published on LongMemEval | — |
| **Memanto** | Undisclosed | Undisclosed | ~89.8% (LongMemEval, version unclear) | Self-reported (methodology unpublished) |
| **Letta** | Undisclosed (entangled with agent loop) | Undisclosed | Not published on LongMemEval | — |
| **Mastra** | Developer-dependent | Developer-dependent | Not published | — |

**Note on accuracy comparison**: Actor models differ or are undisclosed
across systems. Raw accuracy percentages reflect retrieval quality *and*
synthesis quality combined. A stronger actor model can compensate for
weaker retrieval. These numbers are not directly commensurable without
controlling for the actor.

---

## Reproducibility

### Dataset
- **Name**: LongMemEval-S (the "small" split of LongMemEval)
- **Citation**: Wang et al., "LongMemEval: Benchmarking Long-Term Memory
  in AI Assistants" (2024). Available at:
  https://github.com/xiaowu0162/LongMemEval
- **File used**: `longmemeval_s.json`
- **SHA256**: `08d8dad4be43ee20...` (full hash to be recorded from the
  file used in the n=500 run and published in the report JSON)
- **Contents**: 500 questions, 246,930 conversation turns, 6 categories
  (single-session-user: 70, single-session-assistant: 56,
  knowledge-update: 78, temporal-reasoning: 133, multi-session: 133,
  single-session-preference: 30)
- **Format**: JSON array of question objects, each with `question_id`,
  `question`, `answer`, `category`, `haystack` (conversation turns),
  `answer_session_ids`, and optional `question_date`

### Software
- Spectral: commit SHA listed in report JSON (`spectral_version` field)
- Rust toolchain: 1.95.0 (2026-04-14), specified in `rust-toolchain.toml`
- All bench code is in `crates/spectral-bench-accuracy/`

### Stochastic reproducibility

The bench uses default sampling parameters (no pinned temperature or
seed). Two runs will produce different actor/judge outputs. The n=5
variance discipline (section c) established a +/-2pp noise band: any
reproduction within this band of the published number is consistent.

### Cost to reproduce

An Anthropic API key is required. Estimated cost: ~$40 per configuration
(500 questions x ~$0.08/question for actor + judge). Running both
configurations (expansion OFF + expansion ON): ~$80 total.

### To reproduce
1. Clone repo at the stated SHA
2. Obtain `longmemeval_s.json` from the LongMemEval repository
3. Verify SHA256 matches the published hash
4. Set `ANTHROPIC_API_KEY`
5. Run the command of record above (expansion OFF or ON)
6. Report JSON contains per-question results for full auditability
7. `--work-dir` contains per-question intermediate state (brain
   directories, checkpoints) for post-hoc retrieval inspection
