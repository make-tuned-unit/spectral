# Bench Pre-Flight Audit

**Date**: 2026-06-10
**Main SHA**: `f29c6c1`
**All citations**: file:line on SHA `f29c6c1` unless noted

---

## CHECK 1 — Temporal Recency Anchoring

### 1a. Bench Path B call chain

Temporal questions route via `QuestionType::retrieval_path()` → `RetrievalPath::TopkFts` (`retrieval.rs:203`).

Call chain for the bench:
1. `eval.rs:324-325` — `qtype = QuestionType::classify(&question.question)`
2. `eval.rs:333-339` — with `--use-cascade`, effective_path = `qtype.retrieval_path()` → `TopkFts` for Temporal
3. `eval.rs:353` — `let question_date = question.question_date.as_deref();` — passes the dataset's date
4. `eval.rs:355-362` — calls `retrieval::retrieve_topk_fts(&brain, &retrieval_query, &config, question_date)`
5. `retrieval.rs:362` — `now: question_date.and_then(parse_question_date)` — converts "2023/05/30 (Tue) 23:40" to `DateTime<Utc>`
6. `retrieval.rs:356-364` — `RecallTopKConfig { ..., now: question_date.and_then(parse_question_date), ... }` — the `now` field carries the parsed question_date
7. `brain.rs:1151-1176` — `recall_topk_fts` passes config into `apply_reranking_pipeline` which uses `context.now` for recency decay
8. Specifically `brain.rs:1166-1168` — `let ctx = match config.now { Some(dt) => RecognitionContext::empty().with_now(dt), None => RecognitionContext::empty() };`

**Verdict: Bench temporal path anchored: YES.** `question_date` flows from dataset → eval → retrieve_topk_fts → RecallTopKConfig.now → RecognitionContext.now → recency decay. The `Utc::now()` default in `RecallTopKConfig::default()` (`brain.rs:349`, `now: None`) is overridden by the bench at `retrieval.rs:362`.

### 1b. All other callers of recall_topk_fts / recall_cascade

**Library-side callers** (NOT bench, production-relevant):
- `spectral/src/lib.rs:295-301` — `recall_topk_fts` wrapper. Passes config unchanged. **Callers control `now`**; if they use `RecallTopKConfig::default()`, `now: None` means `Utc::now()` is used for the empty RecognitionContext (`brain.rs:1167-1168`). This is correct for production (recency = wall-clock) but would be wrong for historical replay.
- `spectral/src/lib.rs:309-315` — `recall_cascade` wrapper. Takes `CascadePipelineConfig` directly. Context `now` defaults to `Utc::now()` in `RecognitionContext::empty()` (`context.rs:48`). Same story: production correct, replay without explicit `.with_now()` would anchor to wall-clock.

**Test callers** (not production):
- `brain_wrapper_delegation.rs:133,134,164,178` — use `RecallTopKConfig::default()` (now=None → Utc::now()). Fine for tests.
- `brain_wrapper_delegation.rs:207,235` — use `CascadePipelineConfig::default()` with `RecognitionContext::empty()` (now=Utc::now()). Fine for tests.

**Latent production bug**: Any consumer calling `recall_topk_fts` with `RecallTopKConfig::default()` on historical data gets `Utc::now()` as the recency anchor, which makes all old memories equally ancient. The bench fixed this in PR #159 (`retrieval.rs:362`). Library callers (Permagent) are exposed if they replay historical queries without setting `now`. This is a backlog documentation item, not a bench blocker.

### 1c. Verdict

**Bench temporal path anchored: YES** — `retrieval.rs:362` sets `now` to the parsed question_date for every temporal question. Evidence: `eval.rs:353`, `retrieval.rs:347-362`, `brain.rs:1166-1168`.

---

## CHECK 2 — Bench Configuration Ground Truth

Configuration for an n=500 `spectral-bench-accuracy run` with defaults only, no flags:

| Parameter | Value | Source |
|-----------|-------|--------|
| Actor model | `claude-sonnet-4-6` | `main.rs:63` |
| Judge model | `claude-sonnet-4-6` | `main.rs:67` |
| Base URL | `https://api.anthropic.com` | `main.rs:72` |
| Expansion enabled | **YES** (on unless `--no-expand-queries`) | `main.rs:88-90`, `main.rs:351` |
| Expansion model | `claude-haiku-4-5-20251001` | `main.rs:93` |
| Expansion max_terms | 10 | `main.rs:358` |
| Ingest strategy | `per_turn` | `main.rs:46` |
| Retrieval path (no flags) | `topk_fts` (default), no shape routing | `eval.rs:54`, `main.rs:312-318` |
| Retrieval path (`--use-cascade`) | Shape-routed: Temporal→topk_fts, others→cascade | `main.rs:312-318`, `retrieval.rs:199-206` |
| max_results | 40 | `main.rs:76` |
| K per question type (cascade) | Counting: 60, Temporal: 40, Factual: 30, General: 40 | `retrieval.rs:176-196` |
| Recency half-life (Counting) | 730 days | `retrieval.rs:179` |
| Recency half-life (Temporal) | 60 days | `retrieval.rs:185` |
| Recency half-life (Factual) | 365 days (default) | `retrieval.rs:188-191` |
| Recency half-life (General) | 365 days (default) | `retrieval.rs:193-195` |
| Retry policy | max 4 attempts (1 + 3 retries), exponential backoff | `eval.rs:418` (`with_retry(4,...)`), `retry.rs:59` |
| Denominator handling | Transport failures excluded from accuracy; auth failures excluded; recovered-after-retry counted normally | `eval.rs:423-435`, `report.rs:175-211` |
| Cost estimate per question | $0.04 actor + $0.04 judge = $0.08 | `eval.rs:67` |
| Checkpoint interval | 10 questions | `eval.rs:57` |
| `use_cascade` default | **false** | `eval.rs:55` |

**Critical note**: Without `--use-cascade`, the bench runs ALL questions through `topk_fts` with no shape routing. The intended n=500 configuration (from RUN_NOTES) is `--use-cascade` — this enables shape routing. An n=500 run WITHOUT `--use-cascade` would regress by ~15pp on temporal questions.

**No silent changes detected**: All defaults match the values established in PRs #155-#159. The `--use-cascade` flag has been the operational default since PR #86 (RUN_NOTES confirms). The most recent bench run (expansion, per RUN_NOTES) used cascade.

### Verdict

**READY** — configuration stable. Operator must pass `--use-cascade` for the intended configuration. No defaults have silently drifted.

---

## CHECK 3 — Description Density Status

### 3a. State of descriptions in the bench path

The bench `--descriptions` flag is **opt-in** (`main.rs:82-83`). When provided, descriptions are applied per-question after ingest (`eval.rs:303-305` → `describe::apply_descriptions`). They are set on the `Memory.description` field and indexed by FTS via the `memories_fts` virtual table (`sqlite_store.rs:174-176`: `key, content, description`).

**Description artifacts found:**
- `bench_descriptions.json`: 3,095 keys (Anthropic-generated, early version)
- `bench_descriptions_v3_qwen_ms_ssp.cleaned.json`: 74,128 keys (Qwen 7B via Ollama)
- Total dataset turns: 246,930

**Coverage**: 74,128 / 246,930 = **30.0%**. Only MS+SSP categories have descriptions generated. The remaining 70% (SSU, SSA, KU, TR) have no descriptions.

**Sample bench DB check** (bench-v3-ms-ssp): 5 DBs sampled — all show `descriptions_non_null = memories_count` (100% coverage *within the MS+SSP subset*). FTS rowcount matches memories rowcount in all samples. Zero orphan fingerprints.

### 3b. Would an n=500 run today bench WITH or WITHOUT descriptions?

**WITHOUT descriptions by default.** The `--descriptions` flag is opt-in. An n=500 run without `--descriptions` would have zero descriptions on any memory. The most recent expansion run (per RUN_NOTES) noted "descriptions (INERT — retrieval-neutral, not the regression cause, not a lever)" — descriptions were tested and found to have no retrieval impact.

**If descriptions are desired**: the existing 74K-key artifact covers only MS+SSP (30% of turns). An all-category n=500 run would need descriptions for the remaining 70%, which requires a Qwen/Ollama generation pass (~173K additional keys). This is a multi-hour local job.

**Consistency with publish intent**: RUN_NOTES established that descriptions are retrieval-neutral. The intended n=500 configuration (per RUN_NOTES) does NOT require descriptions. Running without them is consistent with the measured baseline.

### Verdict

**READY** — descriptions are opt-in and retrieval-neutral. An n=500 run without `--descriptions` is consistent with the established bench discipline. If full-corpus descriptions are wanted for completeness, that's a separate ~multi-hour generation pass (DECISION-NEEDED: whether to run it).

---

## CHECK 4 — Temporal-Synthesis Branch

**Branch**: `feat/temporal-synthesis-pipeline`
**Fork point**: `ad4ca52` (PR #159, 2026-06-08)
**Commits ahead of fork**: 2 (`e161a35`, `76730fd`)
**Commits on main since fork**: 3 (PRs #160, #161, #162)

### What it does

Two-stage pipeline for temporal OPERATION questions:
1. **EXTRACT** (LLM): from retrieved context, extract structured facts with dates/session IDs (`temporal_synthesis.rs:1-656`)
2. **OPERATE** (code): recency-selection, date-math, chronological sorting — deterministic operations on extracted facts

A `temporal_validate` binary (`bin/temporal_validate.rs:1-562`) tests against 10 hardcoded temporal cases. Supports `--question-ids` filtering and `--judge` flag for LLM grading.

### Measured/claimed lift

**RUN_NOTES explicitly shelved this approach** under "extract→operate (SHELVED, negative result)":
- Extraction from monolithic 20K context: "model drowns"
- Chunked per-session extraction: "exposed a recall-vs-precision tradeoff with no sweet spot"
- Code-side qualification: "ceilings at 5/26"
- LLM qualification: "5/26 measured (vs 13 projected), and BROKE 2 cases code got right"
- Combined optimal: "7/26"

No 20/20 pass-rate recorded. The branch appears to be the implementation that produced the shelved results documented in RUN_NOTES.

### Merge-conflict surface

**0 conflicts** with current main (verified via `git merge-tree`). The 3 post-fork commits on main (PRs #160-162) touch FK migration, foreign_keys pragma, and cascade dead-code removal — none overlap with the temporal synthesis files.

### Distance from kill criterion

The dispatch mentions a "20/20 kill criterion." The branch's measured result (from RUN_NOTES) is **7/26** at best — far below any 20/20 bar. The approach was explicitly shelved with the conclusion "No cheap architectural lever exists for this class."

### Verdict

**DECISION-NEEDED**: Branch implements the shelved extract→operate approach. Measured at 7/26 best-case. RUN_NOTES says "SHELVED — do not re-attempt." Clean merge possible (0 conflicts). Jesse decides ship-or-close.

---

## CHECK 5 — Corpus and Harness Integrity

### 5a. LongMemEval dataset

- **File**: `~/spectral-local-bench/longmemeval/longmemeval_s.json`
- **Question count**: 500
- **By category**: knowledge-update: 78, multi-session: 133, single-session-assistant: 56, single-session-preference: 30, single-session-user: 70, temporal-reasoning: 133
- **Total per-turn ingest keys**: 246,930
- **SHA256 prefix**: `08d8dad4be43ee20`
- No recorded reference hash found to compare against; this is the only copy.

### 5b. FTS integrity (bench-v3-ms-ssp DBs)

5 sampled brain DBs from the most recent bench run:

| DB | memories | fts_rows | match | descriptions | orphan_fps |
|----|----------|----------|-------|-------------|-----------|
| brain_gpt4_ab202e7f | 481 | 481 | YES | 481 | 0 |
| brain_b6025781 | 460 | 460 | YES | 460 | 0 |
| brain_88432d0a | 479 | 479 | YES | 479 | 0 |
| brain_720133ac | 466 | 466 | YES | 466 | 0 |
| brain_a89d7624 | 496 | 496 | YES | 496 | 0 |

`PRAGMA integrity_check`: `ok` on all. FTS rowcount matches memories rowcount. Zero orphan fingerprints post-FK-migration. **Note**: these are v3-ms-ssp DBs (15 questions); an n=500 run creates fresh DBs per-question.

### 5c. Disk

| Metric | Value |
|--------|-------|
| Free space | **12 GB** |
| `~/projects/spectral/target` | 16 GB |
| `~/dev/permagent-runtime/target` | 42 GB |
| Per-question brain dir | ~15 MB |
| Estimated n=500 brain dirs (concurrent) | 500 x 15 MB = ~7.5 GB (if not cleaned) |
| Per-question JSON results | ~10 KB each = ~5 MB total |

**12 GB free is critically tight.** The bench creates per-question brain dirs during the run. If the harness cleans them after each question (it does: `eval.rs` removes brain_dir on transport failure at `eval.rs:429`), peak usage is ~15 MB. But the compiled binary + target/ needs ~16 GB already present. The permagent target (42 GB) should be cleaned if the bench binary needs recompilation.

**Recommendation: `cargo clean` on permagent-runtime/target before starting the run.** This frees ~42 GB. The spectral target/ can also be cleaned (~16 GB) but will need recompilation (~10-15 min).

### 5d. Screen availability

`screen` binary present at `/usr/bin/screen`. No active sessions. Available for use.

Last long run invocation pattern (from RUN_NOTES): not explicitly recorded as a screen command, but the expansion run was a single `spectral-bench-accuracy run` with flags. Expected invocation:
```
screen -S bench
ANTHROPIC_API_KEY=... cargo run --release -p spectral-bench-accuracy -- run \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --work-dir ~/spectral-local-bench/eval-work-n500 \
  --use-cascade \
  --confirm-cost \
  --output ~/spectral-local-bench/eval-report-n500.json
```

### Verdict

**BLOCKED(disk)** — 12 GB free is insufficient for a safe n=500 run with potential recompilation. Need `cargo clean` on permagent-runtime/target (~42 GB) or spectral/target (~16 GB) first.

---

## CHECK 6 — Cost-Comparison Design

Produced separately at `~/spectral-local-bench/COST_COMPARISON_DRAFT.md`.

### Verdict

**READY** — skeleton produced, no Spectral numbers required yet.

---

## Consolidated Gate List

| # | Check | Status | What |
|---|-------|--------|------|
| 1 | Temporal anchoring | READY | Bench path confirmed anchored to question_date |
| 2 | Bench config | READY | Defaults stable; operator must pass `--use-cascade` |
| 3 | Descriptions | READY | Opt-in, retrieval-neutral. Full-corpus gen is separate optional work |
| 4 | Temporal-synthesis branch | DECISION-NEEDED | 7/26 best-case, shelved in RUN_NOTES. Jesse decides ship-or-close. |
| 5a | Dataset | READY | 500 questions, categories confirmed |
| 5b | FTS integrity | READY | All sampled DBs clean |
| 5c | Disk | BLOCKED(disk) | 12 GB free; need `cargo clean` on permagent target (~42 GB) or spectral target (~16 GB) |
| 5d | Screen | READY | Available at /usr/bin/screen |
| 6 | Cost comparison | READY | Skeleton at COST_COMPARISON_DRAFT.md |

---

## Bench Command of Record

### Primary: n=500 with expansion (default)

```bash
screen -S bench-n500

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

### Ablation: n=500 without expansion

```bash
screen -S bench-n500-noexp

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

### Flag justification

| Flag | Value | Source (Check 2 table) |
|------|-------|----------------------|
| `--dataset` | longmemeval_s.json | Dataset path; 500 questions (Check 5a) |
| `--work-dir` | dedicated per-run dir | Avoids contamination between runs |
| `--output` | dedicated per-run report | Separate from prior runs |
| `--use-cascade` | (flag present) | Enables shape routing: Temporal→topk_fts, rest→cascade. `main.rs:312-318`. **Without this flag, all questions route through topk_fts — temporal regresses ~15pp.** |
| `--actor-model` | `claude-sonnet-4-6` | `main.rs:63` (default) |
| `--judge-model` | `claude-sonnet-4-6` | `main.rs:67` (default) |
| `--base-url` | `https://api.anthropic.com` | `main.rs:72` (default) |
| `--max-results` | `40` | `main.rs:76` (default). Shape routing overrides K per type: Counting=60, Temporal=40, Factual=30, General=40 (`retrieval.rs:176-196`) |
| `--ingest-strategy` | `per_turn` | `main.rs:46` (default) |
| `--expansion-model` | `claude-haiku-4-5-20251001` | `main.rs:93` (default). Expansion on by default (`main.rs:351`). |
| `--no-expand-queries` | (ablation only) | `main.rs:88-90`. Disables the Haiku expansion call. |
| `--confirm-cost` | (flag present) | `main.rs:276`. Required when estimate > $10. n=500 × $0.08/q = ~$40. |

### Flags NOT included (with reason)

| Omitted flag | Why |
|-------------|-----|
| `--descriptions` | Decision ratified: no descriptions for n=500. Retrieval-neutral per testing. |
| `--retrieval-path` | Omitted so `--use-cascade` activates per-question shape routing (`main.rs:312-318`). Explicit `--retrieval-path` would override shape routing. |
| `--max-questions` | Omitted = all 500 questions. |
| `--categories` | Omitted = all 6 categories. |
| `--question-id` | Omitted = run all questions. |
| `--dump-scores` | Not needed for the headline run. |

### Pre-run checklist

1. `cargo clean` on `~/dev/permagent-runtime` to free ~42 GB (Check 5c: BLOCKED on disk)
2. Verify `ANTHROPIC_API_KEY` is set
3. Verify `screen` session starts cleanly
4. Estimated cost: ~$40 primary + ~$40 ablation = ~$80 total (both runs)
5. Estimated time: ~4-6 hours per run (500q × ~30s/q including expansion + retries)
