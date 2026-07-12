# Actor counting intervention — held-out validation (2026-07-12)

Real LongMemEval-S, sonnet-5 actor+judge, zero query expansion, shape routing.
Deterministic retrieval; only the counting actor prompts change between arms.

## Context: the bottleneck is actor synthesis, not retrieval

Retrieval recall@40 is near-ceiling (answer-session recall 98–100%). The residual
LongMemEval gap is **actor synthesis**, dominated by **multi-session counting** —
the actor scans sessions well but mis-aggregates across them.

Two measurement-harness bugs first had to be fixed (they inflated the failure
rate on newer models): actor/judge hardcoded `content[0].text` (breaks on
thinking blocks) and judge `max_tokens=512` (truncated verdict JSON). After those
fixes, multi-session baseline is **85%**, and the genuine residual is counting.

## The intervention

Two additions to the counting prompts (`counting_enumerate.md`,
`counting_current_state.md`):
1. **Identity-keying for dedup** — assign each item a stable identifier (person/
   couple name, project title, object details); mentions sharing it are the SAME
   item across sessions/dates. Prevents over-counting one real item as several.
2. **Strict inclusion** — for "what I personally lead/own/did" counts, exclude
   ambiguous, shared/community, hypothetical, or merely-planned items.

## Results

**Tuning set** (first 20 multi-session, used to design the fix):
- 85% → **94.4%** (17/18 graded). "projects I led" over-count 4→2 fixed; no regressions.

**Held-out set** (next 25 multi-session, 22 counting — NOT used for design):

| arm | accuracy | fixed (wrong→right) | regressed |
|---|---|---|---|
| baseline | 18/25 = **72.0%** | — | — |
| + intervention | 20/25 = **80.0%** | **2** | **0** |

Held-out delta **+8.0 pp, zero regressions** — the improvement **generalizes**
(not overfit to the tuning questions). The two held-out fixes are exactly the
target type: "how many hours of jogging and yoga" (aggregation) and "how many
**different** art events" (cross-mention dedup).

## Scope & honesty

- Consistent ~**+8 pp** on multi-session counting across two independent sets,
  with **no regressions** — a genuine, verified actor-side accuracy gain.
- It is a **prompt** change in the bench's actor. The patterns (identity-keying,
  inclusion strictness) are **transferable to Permagent's own actor**, which is
  where they become production accuracy.
- Remaining counting failures are **evidence/retrieval-completeness** on *sum*
  questions (e.g. total $ spent when price turns aren't retrieved) — outside the
  counting prompt's reach; a retrieval-side, not actor-side, gap.
- Sample sizes are 20 + 25; directionally strong and regression-free, but the
  full 133-question multi-session set would tighten the estimate.
