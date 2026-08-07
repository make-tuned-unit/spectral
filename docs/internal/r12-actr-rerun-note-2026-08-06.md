# R12 — ACT-R record re-run (2026-08-06): expectation stated before running

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

Register row R12: every ACT-R measurement predating the F1/F2 fix was
diluted — `ACTR_POOL_WIDEN` used the post-hoc `take()` that made widening
inert on the cascade route (~70% of questions), so ACT-R's reordering never
reached the recorded output there. The lever is off-by-default and no
published number depends on it; this re-run exists so the record is
citable, not to ship anything.

## Design ($0, oracle)

Paired A/B over all 500 LME-S questions, brains REUSED from
`~/spectral-local-bench/oracle-work` (ranking-only lever ⇒ identical
stores are exactly what both arms should read; both arms run under
current-code retrieval, so this measures today's lever on today's read
path — deltas vs historical rows are NOT comparable and will not be
cited). Arms differ in ONE env var:

- A: no `SPECTRAL_ACTR_DECAY` (lever off — shipped default)
- B: `SPECTRAL_ACTR_DECAY=0.5` (the "typical" value from the lever's own
  docs)

Metrics: the standard oracle pair (answer-session recall, answer-key
recall) + `oracle-diff` paired changes.

## Expectation, in advance

Prior record (diluted): a wash. Post-fix honest prior: still likely a
small-or-no effect on session-recall (saturated ~97%+), possibly visible
on key-recall ordering. There is NO ship gate here — any outcome simply
becomes the citable record and closes R12. If B shows a delta worth
pursuing, that becomes a separate prereg with a real gate; nothing ships
from this run.

## Result (run 2026-08-06, both arms 500/500 rows)

| | baseline | ACTR d=0.5 |
|---|---|---|
| contexts changed | — | **389/500** |
| session-recall | 98.1% | +2 / −1 questions |
| zero-answer-key | 2 | fixed 0 / introduced 0 |
| net answer-keys | — | +14 |
| mean tokens | 14,320 | +107 |

**Verdict: ACTIVE but METRIC-NEUTRAL.** The F1/F2 fix worked — the lever
now changes 78% of contexts, so the old "inert" record is confirmed stale
— and the reordering it performs moves nothing the oracle can see:
session-recall churn 2-vs-1, zero-evidence untouched, key deltas noise at
this scale. The record is now citable in this form: enabling
`SPECTRAL_ACTR_DECAY` reshuffles context composition without measured
retrieval benefit. A paid actor replay over the 389 changed contexts is
the only way to detect an accuracy effect, and nothing in this signal
justifies that spend. R12 CLOSED; the lever stays off by default with an
honest record.

Rows: `~/spectral-local-bench/r12-{baseline,actr}.jsonl`.
