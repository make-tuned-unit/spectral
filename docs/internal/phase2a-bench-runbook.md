# Phase 2a Bench Runbook

**Purpose:** Document every failure mode of the bench harness so you
know exactly what to do if the run fails mid-way through 500 questions.

## Quick reference

```bash
ANTHROPIC_API_KEY=<key> target/release/spectral-bench-accuracy run \
  --dataset /Users/jessesharratt/spectral-local-bench/longmemeval/longmemeval_s.json \
  --retrieval-path cascade \
  --confirm-cost \
  --output docs/internal/phase2a-report.json
```

## How checkpointing works

- Every 10 questions (configurable via `checkpoint_interval`),
  the harness writes `eval-work/checkpoint.json` — a full
  `EvalReport` JSON with all results so far.
- The checkpoint is a valid report: `summary()`, per-category
  stats, and all `QuestionResult` entries are present.
- **However, checkpoint resume is a no-op.** The
  `load_completed_ids()` method loads the checkpoint, collects
  question IDs, then calls `ids.clear()` (eval.rs:375). This
  means restarting always re-evaluates from question 1.
- The TODO comment indicates this is known but unfixed.

## Failure modes

### 1. API rate limit (429)

**What happens:** `reqwest` returns 429. The actor/judge checks
`resp.status()`, sees non-2xx, returns `Err(...)`.

**Harness behavior:** The error enters the `Err(e)` branch in
`eval_single`. The question is recorded as failed with
`predicted: "[error: Actor API returned 429: ...]"`.
`consecutive_errors` increments by 1.

**Does NOT retry.** No backoff, no delay. Moves to next question.

**If 3 consecutive 429s occur:** Halts with
`RunStatus::HaltedOnErrors { consecutive_errors: 3 }`. The partial
report is finalized and written to the output path.

**What to do:** The partial JSON at `--output` is usable — it
contains all questions evaluated before the halt, with correct
per-category stats. To avoid 429s: add a 1-2 second sleep between
questions (not implemented — would require code change) or use a
higher-tier API key.

### 2. Network blip / connection error

**What happens:** `reqwest::send()` returns an `Err` (DNS failure,
TCP timeout, TLS error). The `?` propagates to `eval_single`'s
`Err(e)` branch.

**Harness behavior:** Same as rate limit — recorded as failed,
consecutive_errors increments.

**What to do:** Same as above. If the network is flaky, the
3-consecutive-error halt will trigger quickly. Fix the network,
then re-run from scratch (no resume).

### 3. Disk full

**What happens:** Brain creation (`std::fs::create_dir_all`) or
SQLite write fails. `ingest_question` returns `Err`.

**Harness behavior:** Same error path — question recorded as
failed. But if disk is truly full, the checkpoint write and final
report write will also fail silently (both use `let _ =`).

**What to do:** Check disk before running:
```bash
df -h .
```
Each brain is created and deleted per question, so peak disk usage
is one brain at a time (~5-50 MB depending on question size). The
report JSON is small (<5 MB). You need ~100 MB of free space.

### 4. Process killed (Ctrl-C, tmux disconnect, OOM)

**What happens:** The process terminates immediately.

**What is saved:**
- Last checkpoint at `eval-work/checkpoint.json` (up to 10
  questions stale)
- Nothing at `--output` — the final report is only written after
  `report.finalize()` in the `run()` method

**What is lost:**
- All results since the last checkpoint (up to 9 questions)
- The final report file

**What to do:** The checkpoint is a valid `EvalReport` JSON. You
can view it with:
```bash
target/release/spectral-bench-accuracy report --path eval-work/checkpoint.json
```
However, since resume is broken (ids.clear), you cannot continue
from the checkpoint. You must re-run from scratch.

**tmux tip:** Use `tmux` with a named session:
```bash
tmux new -s bench
# run the command
# Ctrl-B D to detach (safe, process keeps running)
# tmux attach -t bench to reattach
```
Closing the terminal with Ctrl-C will kill the process. Detaching
with Ctrl-B D is safe.

### 5. 3-consecutive-error halt (from PR #38)

**What happens:** Three questions in a row return errors (API,
network, disk, any `Err`). The harness breaks out of the question
loop.

**What is saved:**
- `report.run_status` is set to
  `HaltedOnErrors { consecutive_errors: 3 }`
- `report.finalize()` is called
- The report is written to `--output` (unlike Ctrl-C)
- The report contains ALL questions evaluated before the halt,
  including the 3 error questions

**Is the partial JSON usable?** Yes. Per-category accuracy is
computed from all evaluated questions. The `run_status` field
tells you it halted early. Errors are marked with
`predicted: "[error: ...]"` and `correct: false`.

**What to do:** Inspect the report to understand why 3 consecutive
errors occurred. Common causes:
- Bad API key (401 on every call)
- Rate limit storm (429 burst)
- Network down

Fix the root cause, then re-run from scratch.

### 6. Single isolated API error

**What happens:** One question's API call fails, but the next
succeeds.

**Harness behavior:** `consecutive_errors` resets to 0 on the
next success. The failed question is recorded with
`correct: false` and error details in the `predicted` field.
The run continues.

**Impact on accuracy:** The failed question counts as incorrect.
With 500 questions, one error changes overall accuracy by 0.2%.
The `results` array lets you identify which questions had errors
vs. genuine wrong answers.

## How to resume from a partial run

**Short answer: you cannot.** The checkpoint resume code is
disabled (eval.rs:375 `ids.clear()`).

**Workaround:** If you have a partial report (from checkpoint or
halt), you can:

1. Note which questions were evaluated (from `results` array)
2. Use `--categories` to run only the remaining categories
3. Or re-run the full 500 — it's the cleanest approach

**Cost of re-run:** ~$19 expected. If you got halfway before a
halt, the re-run costs the full amount (no deduction for prior
work).

## Pre-run checklist

- [ ] `ANTHROPIC_API_KEY` is set and valid
- [ ] `df -h .` shows >100 MB free
- [ ] `tmux` session is active (to survive terminal disconnect)
- [ ] Preflight passed: `target/release/spectral-bench-accuracy preflight --dataset <path> --all`
- [ ] Output path does not already exist (or has been backed up)
- [ ] Network is stable (test: `curl -s https://api.anthropic.com/v1/messages -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" -H "content-type: application/json" -d '{"model":"claude-sonnet-4-6","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}'`)
