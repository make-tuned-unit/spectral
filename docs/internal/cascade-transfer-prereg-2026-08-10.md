# R28 — do the topk_fts findings transfer to `recall_cascade`? · PREREGISTRATION

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
cascade`, R19 turn labels. No model calls, model-free.** Written and committed
before any arm ran.

## Why this is the most important $0 run outstanding

**Every result in this series was measured on `topk_fts`. Permagent calls
`recall_cascade` exclusively.**

R22, R23, R24, R25, R26 and R27 each correctly registered "no cascade
measurement, therefore no cascade change" — individually honest, cumulatively a
problem. We now hold a substantial body of retrieval findings on a path our only
consumer does not use:

| finding | measured on | transfers? |
|---|---|---|
| adjacency +18.93pp (vs k=40) / +6.73pp (token-matched) | `topk_fts` | **unknown** |
| speaker attribution +2.76pp | `topk_fts` | **unknown** |
| RRF refuted −6.96pp | `topk_fts` | **unknown** |
| k frontier (R27) | `topk_fts` | **unknown** |

**If these do not transfer, the session's practical value to Permagent is
zero.** That is worth knowing before anything is proposed for adoption.

## What is structurally different about cascade, and why transfer is not obvious

- **`k` is not controlled by `--max-results`.** The 2026-08-08 k re-test found
  `single-session-preference` came back **bit-identical** because it routes to
  cascade, whose k comes from the question-type profile. Cascade k is set by
  `SPECTRAL_CASCADE_K` / the profile, not the output cap.
- Cascade applies **episode diversity** (`max_per_episode`), which caps turns
  per session. Adjacency emits *within* sessions, so diversity may actively
  fight it.
- Cascade runs **ambient boost** and question-type routing that topk does not.
- Cascade already groups context by session (`format_hits_grouped_capped_dated`),
  which is closer to what adjacency produces — so adjacency may be **partly
  redundant** here in a way it is not on topk.

Those are reasons to expect a *different* answer, not a smaller one. Direction
is genuinely open.

## Arms — fixed before running

| arm | config | purpose |
|---|---|---|
| **C0** | cascade, defaults | baseline — establishes cascade's own evidence recall, never measured at full N |
| **C-ADJ** | `SPECTRAL_ADJACENCY=1` | **PRIMARY** — does adjacency transfer? |
| **C-SPK** | speaker-field dataset | does R24 transfer? |

Adjacency is newly wired into `retrieve_cascade` for this run (it previously
existed only on topk), after truncation, for the same reason as on topk: it
emits neighbours of what ranking chose and does not rank. Verified the topk path
is **unaffected** by that edit (0/8 context-hash diffs).

**No token-matched control arm.** On topk, KMATCH was constructible because
`--max-results` sets k. On cascade it does not, so an equal-budget control needs
a `SPECTRAL_CASCADE_K` sweep to locate the matching k — that is a **separate
prereg**, and until it exists **C-ADJ vs C0 is a cost-unmatched comparison and
will be reported as such.** This is the flattering-comparison problem R25 was
designed to avoid, and it is being declared rather than hidden.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn micro-recall, C-ADJ vs C0.
**Statistic:** Wilcoxon signed-rank on per-question evidence-turn counts,
two-sided, α = 0.05 (`scripts/score_r24.py`), nonzero-pair count always
reported.

- **TRANSFERS**: p < 0.05 and ≥ +2.0pp **and** context cost within ~1.5× of the
  topk arm's 2.62×. A recall gain bought with far more context than topk needed
  is a different result, not the same one.
- **DOES NOT TRANSFER**: NULL, or a gain whose token cost is materially worse.
- **REFUTED ON CASCADE**: significant decrease — a live outcome, since episode
  diversity may fight adjacency.
- **STILL UNDERPOWERED**: fewer than 15 nonzero pairs.

**Secondary:** C-SPK vs C0 (same gates), zero-evidence counts, the
multi-session slice, and cascade's own baseline recall against topk's 59.86% —
which is itself an unmeasured and interesting quantity.

## Registered non-goals

- **No paid runs, no embeddings, no model.**
- **No default change on any path**, whatever the outcome. A cascade PASS
  licenses a *proposal* to Permagent, not a flipped default.
- No cascade-k tuning, no `max_per_episode` tuning. Single variable per arm.
- No combination of adjacency with speaker attribution — that needs its own
  prereg on either path.

## Honest limits

- Retrieval only. **No accuracy claim**, on either path.
- If cascade's baseline recall differs materially from topk's, the two paths'
  results are not directly comparable and the cross-path comparison will be
  reported as descriptive only.
- LoCoMo only; adjacency remains a two-party-dialogue-shaped lever.

**Register row:** R28. **Refs:** `turn-adjacency-prereg-2026-08-09.md` (R25),
`speaker-field-result-2026-08-09.md` (R24),
`k-admission-frontier-prereg-2026-08-10.md` (R27, the cascade-k caveat).
