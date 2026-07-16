# TACT tiers cost session-recall vs pure FTS — 2026-07-14

**Finding.** On LongMemEval single-session categories, the TACT tier machinery
(fingerprint / wing routing) that the **cascade** path leads with is neutral-to-
harmful for **session-recall** — the metric that gates whether the actor can
answer at all. Pure wide FTS (`topk_fts`) matches or beats cascade everywhere
tested, and cascade *loses* 6.7pp session-recall on single-session-preference.
Isolated to TACT crowding, not pool width.

## Data ($0 deterministic oracle, cached brains) — session-recall

| category (n) | tact (tiers alone) | cascade (default) | topk_fts (pure wide FTS) |
|---|:-:|:-:|:-:|
| single-session-preference (30) | 83.3% | 93.3% | **100.0%** |
| single-session-assistant (56)  | 100.0% | 100.0% | 100.0% |
| single-session-user (70)       | 100.0% | 100.0% | 100.0% |
| knowledge-update (78)          | 97.4% | 98.7% | 98.7% |

TACT-alone is worst everywhere (and drops to 83.3% on preference). Cascade never
*beats* pure FTS on session-recall in any tested category and loses on preference.

## Isolation (single-session-preference) — TACT crowding, not pool width

| config | session-recall |
|---|:-:|
| topk_fts @ fetch_mult=1 (pure FTS, **narrow** pool) | **100.0%** |
| cascade @ fetch_mult=1 (published default) | 93.3% |
| cascade @ fetch_mult=3 (TACT+FTS, **wide** pool) | 96.7% |

Pure FTS at the narrow pool already achieves 100%, so the loss is not pool width.
Cascade leads with `cascade_retrieve` = TACT candidates, then supplements with FTS
up to `k`. TACT's wing detection uses persona-shaped rules (`alice|coffee|recipe|
travel|purchase|...`) whose generic words fire *spuriously* on LongMemEval,
surfacing wrong-wing memories that fill top slots and crowd out the answer-session
memory FTS would have ranked in-window. Widening the pool partially recovers it
but TACT still leads, so it never reaches pure FTS.

## Why this is not the fetch_mult trap

The fetch_mult lever moved *key-recall* (a bloated proxy: answer sets of 36+
evidence turns for a 3-item answer) while session-recall was already saturated —
and it did not convert to accuracy. Here the moving metric is **session-recall
itself**, a *necessary condition*: if the answer session is not retrieved, the
actor cannot answer it (structural failure, not a synthesis choice). Recovering
it removes a hard ceiling rather than reshuffling already-present evidence.

## Candidate lever

Route the affected question shapes to `topk_fts` instead of cascade — the same
path already used (and proven) for Temporal. The published router
(`QuestionType::retrieval_path`) sends only Temporal to topk_fts today; the
single-session/preference shapes go to cascade. Sending the GeneralPreference /
GeneralRecall / General shapes (which dominate single-session-preference) to
topk_fts recovers session-recall at similar token cost (preference: 9598 → ~9269
tokens).

**Status: candidate, not shipped.** Session-recall is accuracy-gating and its
gain here is real and deterministic, but end-to-end actor confirmation is blocked
by this environment's flaky API connectivity (transport failures). Unlike a
key-recall proxy, the downside is bounded — adding the correct answer session to
the context is monotonic-good for answerability — but per the discipline that
made us revert fetch_mult, a *default routing change* to production Brains should
carry an end-to-end number. Ship when a stable-network env can confirm, or accept
the (bounded) risk given session-recall is a necessary condition.

Broader implication: like the Kuzu graph path, the TACT tiers are a namesake
subsystem that does not earn its place on real data here — worth revisiting
whether cascade should lead with FTS (TACT as supplement, not primary) or whether
TACT's wing rules need to be dataset-appropriate rather than persona-shaped.
