# R15 evidence-recall acceptance fixture

Pins the R15 evidence-turn metric in CI. Two files, both derived from data on
the bench machine, both committed so the acceptance criterion is a test that
actually runs.

## `r12-rows-subset.jsonl`

**Verbatim, unmodified lines** from `~/spectral-local-bench/r12-baseline.jsonl`
(the shipped-config oracle run: cascade + shape routing, porter). They still
carry the pre-rename field names `answer_keys_total`,
`answer_keys_retrieved`, `rank_first_answer_key` — so loading this file is
itself the proof that `#[serde(alias = ...)]` keeps the whole archive
readable.

## `dataset-subset.json`

A **trimmed** slice of `longmemeval/longmemeval_s.json`, matched by
`question_id`. Selection is deterministic: dataset indices where `i % 12 == 0`,
plus indices 64, 126 and 227 (each the first question of a distinct `_abs`
block) so the fixture contains unlabelled questions and the
"undefined ≠ zero" rule is pinned by real data. 45 questions: 42 labelled,
3 unlabelled.

Trimming, and why it is sound: evidence recall is a pure function of
(session id, turn index, role, `has_answer`, `retrieved_keys`). It does not
read turn text, and it does not read non-evidence sessions. So the fixture
keeps every session containing a `has_answer: true` turn **whole** (turn
indices must be preserved), drops the rest, and truncates `content` to 32
characters.

**Consequence: this file must never be used to run retrieval.** Ingesting it
would build a haystack with no distractors and truncated text — a different
corpus entirely. It is a label source and nothing else.

## What is pinned

`evidence_recall_pinned_on_committed_fixture` in `src/oracle.rs`:

| quantity | value |
|---|---:|
| questions | 45 |
| labelled / unlabelled | 42 / 3 |
| micro evidence recall | 68/78 = 87.18% |
| macro evidence recall | 91.746% |
| zero-evidence questions | 1 |
| full-evidence questions | 35 |

These are figures for the **subset**, not for LongMemEval-S. The full-corpus
numbers (793/896 micro, 90.5% macro, 27 zero, 409 full) are pinned separately
by the `#[ignore]`d `evidence_recall_reproduces_r15_note`, which reads the
machine-local originals.
