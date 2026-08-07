# Post-hardening retrieval benchmark — 2026-07-24

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

## Question

Did the public-API, visibility, deletion-verification, ontology-persistence,
derived-state repair, retrieval-receipt, and build-portability hardening change
retrieval quality on an existing dataset?

## Method

- Baseline: clean repository `HEAD`, exported to an isolated directory.
- Candidate: working tree containing the hardening changes.
- Dataset: local 25-question held-out multi-session set,
  `heldout_ms.json`, SHA-256
  `efb123fb2f6511799dc297a62a9780935fbb8ad8e64a4ca662fda410d691a8b6`.
- Harness: `spectral-bench-accuracy oracle`, published shape routing,
  per-turn ingestion, fresh brain per question, no query expansion.
- Cost: zero LLM calls; no actor or judge credentials were configured.
- Comparison: repository `oracle-diff`, joined by `question_id`.

## Result

| Metric | HEAD | Candidate | Delta |
|---|---:|---:|---:|
| Questions | 25 | 25 | 0 |
| Session recall | 100.0% | 100.0% | 0.0 pp |
| Key recall | 48.6% | 48.6% | 0.0 pp |
| Zero-recall questions | 0 | 0 | 0 |
| Mean context tokens | 17,943 | 17,943 | 0 |
| P95 context tokens | 24,527 | 24,527 | 0 |
| Contexts changed | — | 0/25 | — |
| Net retrieved answer keys | — | — | 0 |

Every candidate context hash and ordered retrieval output matched HEAD. The
hardening pass therefore produced **no retrieval-quality improvement and no
retrieval-quality regression** on this held-out slice. That is the expected
result: the pass changes safety and operability contracts, not ranking math.

## LongMemEval-S smoke comparison

A second paired comparison used the local 500-question LongMemEval-S file
(SHA-256 `08d8dad4be43ee2049a22ff5674eb86725d0ce5ff434cde2627e5e8e7e117894`).
Because a full fresh-brain run is several hours on this host, this verification
sampled the first two questions from three distinct routing/memory shapes:

| Category | n | Session recall | Key recall | Contexts changed | Answer-key delta |
|---|---:|---:|---:|---:|---:|
| single-session-user | 2 | 100.0% | 20.8% | 0/2 | 0 |
| temporal-reasoning | 2 | 100.0% | 55.3% | 0/2 | 0 |
| knowledge-update | 2 | 100.0% | 56.2% | 0/2 | 0 |

HEAD and candidate were identical on every metric and context hash. This is a
cross-path smoke test, not a replacement for the published full-500 result.

## Latency caveat

The two campaigns ran concurrently to reduce turnaround time. Recorded internal
retrieval latency was HEAD 24.8 ms mean / 22 ms median / 50 ms p95 and candidate
28.92 ms mean / 23 ms median / 69 ms p95. The candidate was compiling and
running while the baseline release build consumed the same CPU, so this is a
confounded systems measurement and **must not be interpreted as a candidate
regression**. A latency claim requires sequential repeated-query runs on reused
brains with warm-up and process isolation.

## What this establishes

- Dataset retrieval behavior is byte-stable across the hardening pass on the
  held-out multi-session slice.
- Accuracy improvement is neither claimed nor observed.
- Improvements established elsewhere in the same verification campaign are
  ontology safety, visibility-safe public access, fail-closed deletion
  verification, derived-state recovery, audit receipts, and build portability.

## Raw artifacts

Generated locally during the run:

- `/private/tmp/spectral-oracle-baseline-heldout.jsonl`
- `/private/tmp/spectral-oracle-candidate-heldout.jsonl`
- `/private/tmp/spectral-oracle-{baseline,candidate}-lme-user.jsonl`
- `/private/tmp/spectral-oracle-{baseline,candidate}-lme-temporal.jsonl`
- `/private/tmp/spectral-oracle-{baseline,candidate}-lme-update.jsonl`

These files contain one auditable row per question, including retrieved keys,
answer-session/key coverage, context hash, token estimate, and retrieval wall
time.
