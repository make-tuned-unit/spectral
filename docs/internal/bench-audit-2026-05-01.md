# spectral-bench-accuracy Audit Report

**Date:** 2026-05-01
**Auditor:** Claude (automated)
**Scope:** Full source review of `crates/spectral-bench-accuracy/src/` (9 files)
**Branch:** chore/bench-accuracy-audit

---

## Executive Summary

The bench harness has several issues that silently corrupt accuracy
numbers. The most severe: 51.8% of questions are miscategorized due
to incomplete category mapping, and the actor prompt omits
`created_at` timestamps — the single piece of metadata that
temporal_reasoning questions depend on. These two issues alone mean
published numbers are unreliable across all categories.

---

## Issues

### 1. Category mapping silently miscategorizes 51.8% of questions

**Severity:** BLOCKER
**File:** `dataset.rs:53-68` (`Category::from_question_type`)

The real LongMemEval_S dataset uses these `question_type` values:

| Dataset value                | Count | After normalization        | Maps to                 | Correct? |
|------------------------------|-------|----------------------------|-------------------------|----------|
| `multi-session`              | 133   | `multi_session`            | `InformationExtraction` | WRONG    |
| `temporal-reasoning`         | 133   | `temporal_reasoning`       | `TemporalReasoning`     | OK       |
| `knowledge-update`           | 78    | `knowledge_update`         | `KnowledgeUpdate`       | OK       |
| `single-session-user`        | 70    | `single_session_user`      | `InformationExtraction` | WRONG    |
| `single-session-assistant`   | 56    | `single_session_assistant` | `InformationExtraction` | WRONG    |
| `single-session-preference`  | 30    | `single_session_preference`| `SingleSessionPreference`| OK      |

The match arm `"multi_session_reasoning"` requires the suffix
`_reasoning` which isn't in the dataset. The `"single_session"`
match arm only catches the exact string `"single_session"`, not
`"single_session_user"` or `"single_session_assistant"`.

259 out of 500 questions (51.8%) silently fall through to the
`_ => Self::InformationExtraction` default. This corrupts:

- **Per-category accuracy numbers**: `information_extraction` gets
  inflated with 259 foreign questions. Real multi-session,
  single-session-user, and single-session-assistant categories
  don't appear in the report at all.
- **Judge rubric selection**: These 259 questions get the generic
  "factual equivalence" rubric instead of their category-specific
  rubric. Multi-session questions may need different evaluation
  criteria than information extraction.
- **Category filter (`--categories`)**: Filtering by
  `multi_session_reasoning` matches zero real questions.

**Recommended fix:** Add match arms:
- `"multi_session"` → `MultiSessionReasoning`
- `"single_session_user" | "single_session_assistant"` →
  new variant or map to `SingleSessionPreference`

Consider whether `single-session-user` and
`single-session-assistant` should be distinct Category variants or
merged with `SingleSessionPreference`. The dataset treats them as
separate types. Also consider adding `"information_extraction"` and
`"abstention"` to the never-hit category: if the dataset doesn't
contain these values, the Category enum variants are dead code.

### 2. Actor prompt omits created_at — temporal_reasoning is blind

**Severity:** BLOCKER
**File:** `retrieval.rs:28-32`, `actor.rs:39-46`

Retrieved memories are formatted as:

```
[wing/hall] key: content
```

The `created_at` timestamp is available on `MemoryHit` but is
**not included** in the formatted string passed to the actor.

For temporal_reasoning questions (133 questions, 26.6% of the
dataset), the actor is asked "when did X happen?" but sees no
dates in its context. This guarantees 0% accuracy on temporal
questions regardless of how well Spectral retrieves.

The key also contains no date info — it's
`"{session_id}:turn:{turn_idx}:{role}"`.

**Recommended fix:** Include `created_at` in the retrieval format
string:

```rust
format!("[{wing}/{hall} | {date}] {}: {}", hit.key, hit.content)
```

where `date` is `hit.created_at.as_deref().unwrap_or("unknown")`.

Also consider including the session date in the memory content
itself during ingest (e.g., prepending "On 2023/02/15:") so the
information is embedded in the text regardless of formatting.

### 3. No API error handling — silent wrong answers

**Severity:** MAJOR
**File:** `actor.rs:55-68`

The actor HTTP call has no error checking on the response status
code. If the API returns a 429 (rate limit), 500 (server error),
or 401 (bad key), the code parses the error JSON, extracts
`content[0].text` (which is `null` on error responses), and returns
an empty string `""` as the actor's answer.

This empty string then goes to the judge, which grades it as
incorrect. The failure is recorded as a wrong answer, not as an
infrastructure error. This means:

- Rate limiting silently tanks accuracy numbers
- Transient API failures are indistinguishable from wrong answers
- There's no way to distinguish "Spectral retrieved poorly" from
  "the API was down"

The same issue exists in the judge (`judge.rs:95-105`): a failed
API call returns `GradeResult { correct: false }` — silently
marking the answer wrong.

**Recommended fix:** Check `resp.status()` before parsing. On
non-2xx, return `Err(...)` so the eval loop's error branch handles
it properly (it already marks errors distinctly with
`[error: ...]`). Consider adding retry with backoff for 429s.

### 4. Checkpoint resume is a no-op

**Severity:** MAJOR
**File:** `eval.rs:224-236` (`load_completed_ids`)

The method loads checkpoint data, collects question IDs, then
immediately calls `ids.clear()` with a TODO comment. This means:

- If a 500-question run crashes at question 300, restarting
  re-evaluates all 500 questions
- The checkpoint file is written every 10 questions but never read
  back usefully
- This wastes API budget on re-runs (at ~$0.08-0.10/question,
  a 500-question re-run costs ~$40-50)

**Recommended fix:** Remove the `ids.clear()` line. The collected
IDs are already correct for resume purposes. If there's a concern
about data integrity, validate by checking that the checkpoint's
question results are non-empty before trusting them.

### 5. Cost estimate is 2x too low

**Severity:** MINOR
**File:** `eval.rs:44-49` (`estimate_cost`)

The comment says "$0.04 per call" with 2 calls per question =
$0.08/question. But observed cost from a smoke run is
~$0.10/question. The discrepancy likely comes from:

- The estimate assumes ~10K input tokens, but with 20 retrieved
  memories (each potentially hundreds of tokens), the actor prompt
  can easily reach 15-20K tokens
- Output tokens are underestimated at 0.5K — actor answers and
  judge reasoning often exceed 1K

For a 500-question run, the estimate says $40 but actual cost
would be ~$50. Not a huge gap but worth correcting so the
`--confirm-cost` gate is accurate.

**Recommended fix:** Bump the per-call estimate to $0.05 or
compute more precisely based on `max_results * avg_memory_tokens`.

### 6. Judge rubric has no category-specific rubric for multi-session

**Severity:** MAJOR
**File:** `judge.rs:27-44`

The judge prompt has specific rubrics for:
- `Abstention` — "should say it doesn't know"
- `KnowledgeUpdate` — "most recent information"
- `TemporalReasoning` — "temporal aspect is accurately captured"
- Everything else — generic "factual equivalence"

Multi-session reasoning questions (the largest category at 133
questions) fall into the generic "factual equivalence" rubric.
These questions specifically test cross-session synthesis — the
judge should evaluate whether the answer correctly integrates
information from multiple conversations.

Additionally, since `multi-session` currently miscategorizes to
`InformationExtraction` (Issue 1), fixing the category mapping
alone won't help — a multi-session rubric also needs to be added.

**Recommended fix:** Add a `MultiSessionReasoning`-specific rubric:

```
"The question requires synthesizing information across multiple
 conversation sessions. The answer is correct if it accurately
 combines relevant facts from different sessions."
```

### 7. Memory key extraction is fragile

**Severity:** MINOR
**File:** `eval.rs:174-180`

Memory keys are extracted from formatted strings by splitting on
`"] "` then `": "`. If memory content contains `"] "` or the key
contains `": "`, the extraction produces wrong keys. The
`retrieved_memory_keys` field in the report would contain truncated
or incorrect values.

This doesn't affect accuracy scoring (keys are only for
reporting), but makes failure analysis harder.

**Recommended fix:** Return structured data from `retrieve()`
instead of pre-formatted strings. Return `Vec<RetrievedMemory>`
with separate `key`, `content`, `wing`, `hall`, `created_at`
fields, and format for the actor prompt separately from the
report data.

### 8. DryRun only processes first question

**Severity:** MINOR
**File:** `main.rs:132`

`DryRun` always takes `ds.first()` — the first question in the
array. If the user wants to verify a specific category (e.g.,
temporal-reasoning), they can't without reordering the dataset
file.

**Recommended fix:** Accept an optional `--question-id` or
`--max-questions N` flag for dry-run to allow sampling different
questions.

### 9. Brain cleanup deletes evidence on failure

**Severity:** MINOR
**File:** `eval.rs:194`

After `eval_single` runs (whether success or failure), the brain
directory is deleted:

```rust
let _ = std::fs::remove_dir_all(&brain_dir);
```

This happens inside `eval_single`, so even on success (where the
result is used), the brain is gone before the caller could inspect
it. For debugging failures — especially at 0% accuracy on a
category — there's no way to inspect what was actually ingested
or what the recall query returned.

**Recommended fix:** Add a `keep_brains: bool` config option
(default false). When true, skip the `remove_dir_all` so failed
runs can be inspected.

### 10. Category enum has phantom variants

**Severity:** MINOR
**File:** `dataset.rs:43-49`

Two Category enum variants are never produced from real dataset
data:
- `InformationExtraction` — no question has this type in
  LongMemEval_S (it only appears as the fallback default)
- `Abstention` — no question has this type in LongMemEval_S

These variants exist in the enum, in `Category::all()`, and in
the summary report output. The report will always show 0/0 for
`abstention` (invisible) and an inflated count for
`information_extraction` (from the fallback).

Two real dataset types have no enum variant:
- `single-session-user` (70 questions)
- `single-session-assistant` (56 questions)

**Recommended fix:** Align the enum with the actual dataset. Either
add `SingleSessionUser` and `SingleSessionAssistant` variants, or
map them to `SingleSessionPreference`. Remove or repurpose
`InformationExtraction` and `Abstention` if they don't appear in
the dataset.

### 11. PerTurn ingest splits context the actor needs together

**Severity:** MINOR (design concern, not a bug)
**File:** `ingest.rs:74-86`

With `PerTurn` (the default), each user and assistant message
becomes a separate memory. For a 10-turn session, that's 10
memories. With `max_results: 20`, the actor may only see 20 out of
potentially hundreds of turns across multiple sessions.

More critically, splitting turns means the actor sees fragments
like "That sounds great!" (an assistant reply) without the user
message that prompted it. Context is lost.

`PerSession` concatenates all turns, which preserves context but
creates longer memories that may not fingerprint or retrieve as
well.

This is a design trade-off, not a bug, but worth noting that the
default strategy may be suboptimal for accuracy.

**Recommended fix:** Consider testing both strategies and reporting
which performs better. Also consider a hybrid: ingest per-turn for
retrieval granularity but include surrounding context (e.g.,
user+assistant pairs) in each memory's content.

---

## Summary Table

| # | Issue | Severity | Category |
|---|-------|----------|----------|
| 1 | Category mapping miscategorizes 51.8% of questions | BLOCKER | dataset.rs |
| 2 | Actor prompt omits created_at timestamps | BLOCKER | retrieval.rs, actor.rs |
| 3 | No API error handling — silent wrong answers | MAJOR | actor.rs, judge.rs |
| 4 | Checkpoint resume is a no-op | MAJOR | eval.rs |
| 5 | Cost estimate is 2x too low | MINOR | eval.rs |
| 6 | Judge rubric missing for multi-session | MAJOR | judge.rs |
| 7 | Memory key extraction is fragile | MINOR | eval.rs |
| 8 | DryRun only processes first question | MINOR | main.rs |
| 9 | Brain cleanup deletes evidence on failure | MINOR | eval.rs |
| 10 | Category enum has phantom variants | MINOR | dataset.rs |
| 11 | PerTurn ingest splits needed context | MINOR | ingest.rs |

## Recommended Fix Priority

1. **Issue 1** (category mapping) — fix first, everything else is
   noise if half the questions are miscategorized
2. **Issue 2** (created_at in actor prompt) — fix second, unblocks
   temporal_reasoning
3. **Issue 3** (API error handling) — fix third, ensures accuracy
   numbers reflect Spectral quality not API flakiness
4. **Issue 6** (multi-session rubric) — fix with Issue 1
5. **Issue 4** (checkpoint resume) — fix before next full run to
   avoid $50 re-run waste
6. Everything else — address as time permits
