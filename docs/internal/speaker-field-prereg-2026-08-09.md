# R24 — speaker as a separate indexed field, at full N · PREREGISTRATION

**$0. Retrieval-only oracle, LoCoMo, `--retrieval-path topk_fts`, k=40, R19 turn
labels. No model calls, no paid runs, model-free (no embeddings — explicit
project constraint).** Written and committed before any arm executed.

## Two things R23 got wrong, both fixed here

**1. The statistic was underpowered — but the real cause was the sample.**

R23 used exact McNemar on the all-or-nothing "all evidence turns retrieved"
indicator. With 3 discordant pairs the smallest attainable two-sided p is
`2 × 0.5³ = 0.25`, so it could not have passed at any effect size.

The obvious fix is a finer statistic. Measured on the R23 output, that is not
enough on its own: switching to per-question evidence-turn **counts** yields
only **6 nonzero pairs** at N=250 — 2.0× more information, still marginal
(minimum attainable two-sided sign-test p = `2 × 0.5⁶` = 0.031, barely under
α).

**The binding constraint is N.** The labelled LoCoMo file holds **1,438**
questions and R23 used **250** — 17% of the corpus, for no reason other than
inheriting G4's setting.

**2. N was capped by a false belief about disk.** `oracle.rs` deletes each
brain immediately after its row is written (`if !config.keep_brains` sits
*inside* the question loop), so `--no-keep-brains` already streams cleanup and
peak disk is **one brain (~20MB)**, not the whole set. N was never disk-bound.

## Arms — fixed before running

All arms: full **N = 1,438**, `per_turn`, `topk_fts`, k=40,
`--fresh-brains --no-keep-brains`. Each arm is a separate ingest by
construction (they differ in what is written), so brains are not shared and
each arm pays a full ingest.

| arm | change | lever |
|---|---|---|
| **A0″** | baseline at full N | *(none)* |
| **C** | **PRIMARY** — speaker in the separate indexed FTS `description` column; content untouched | `SPECTRAL_SPEAKER_FIELD=1` |
| **B′** | R23 arm B replicated at full N — speaker prefixed inline into content | prefixed dataset |

**Why C is expected to beat B′, stated in advance:** `memories_fts` indexes
`(key, content, description)` as separate columns. Prefixing (B) makes the name
present in the right turns *and* in every other turn by that speaker, diluting
the content channel — the name stops being a discriminator. C puts it in a
channel of its own, so a query naming a person can match turns that person
**spoke** without competing with content matching. If C ≈ B, the separation
buys nothing and the dilution account is wrong.

**B′ is not decoration.** R23's arm B was a NULL under a test that could not
pass. Replicating it at 5.75× the data is the only honest way to learn whether
that null was an absence or an underpowered true effect.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn micro-recall.
**Primary comparison:** C vs A0″.
**Primary statistic:** **Wilcoxon signed-rank** on per-question evidence-turn
count differences, two-sided, α = 0.05. Zero differences are dropped (standard);
the count of nonzero pairs is **always reported**, because that number is what
made R23 uninterpretable.

**PASS** requires *both*: p < 0.05 **and** micro-recall increase ≥ **+2.0pp**.
Anything else is **NULL**. A significant decrease is **REFUTED** and published
with equal prominence.

**Power, computed in advance (the step R23 skipped):** R23 produced 6 nonzero
pairs from 250 questions (2.4%). At N = 1,438 that rate predicts **~34 nonzero
pairs**. A two-sided sign test at n = 34 needs roughly ≥24 in one direction for
p < 0.05; R23 observed **6/6 positive**. If the true positive rate is ≥70%, this
design detects it. **If the nonzero-pair count comes in under ~15, the run is
reported as still underpowered rather than as a null** — that call is made here,
not after seeing p.

**Secondary, reported but not decisive:** McNemar on the full-evidence indicator
(continuity with R23), zero-evidence count, multi-session slice, context tokens,
and the mechanism check — the share of retrieved top-40 turns containing the
queried name, which was 36.4% at baseline and 19.2% under R23 arm B.

## What would make this uninterpretable

- **A0″ failing to reproduce the 250-question baseline on its first 250 rows**
  (231/356 micro, 53 zero-evidence). The first 250 questions of A0″ are the same
  questions under the same config, so this is a real check.
- Fewer than ~15 nonzero pairs, per the power rule above.
- Comparing C or B′ against R23's 250-question control rather than A0″.

## Registered non-goals

- **No paid runs. No embeddings, no model of any kind** — the model-free
  constraint is a project decision, and R24 does not relitigate it.
- **No cascade measurement**, therefore no cascade change. `recall_cascade` is
  the only path Permagent calls; nothing here licenses touching it.
- No FTS column weighting (`bm25(memories_fts, …)`) tuning. Arm C uses the
  default unweighted match. Weighting after seeing results would be a forking
  path; if C passes, weighting is a separate prereg.
- R23 is **not** re-scored under this statistic. It stands as published.

## Honest limits

- LoCoMo is a **two-speaker** corpus, the worst case for prefix dilution (B′)
  and the easiest case for a speaker field to look good (C). A many-speaker
  corpus would narrow the gap between them.
- Retrieval only. Even a PASS makes no accuracy claim.
- Speaker metadata is restored from raw LoCoMo (`speaker_a`/`speaker_b`, per-turn
  `speaker`, 149,456/149,456 turns matched at N=250; 865,369/865,369 at full N).
  It is corpus metadata, never a question, answer, or evidence label — and it
  mirrors production, where Permagent holds speaker identity as metadata.

**Register row:** R24. **Refs:** `speaker-attribution-result-2026-08-09.md`
(R23, whose power failure this fixes), `speaker-attribution-diagnostic-2026-08-09.md`
(the 8.5× inversion this targets), `rrf-composition-result-2026-08-09.md`.
