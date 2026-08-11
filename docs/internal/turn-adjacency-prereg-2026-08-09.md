# R25 — turn adjacency emission · PREREGISTRATION

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
topk_fts`, R19 turn labels. No model calls, no paid runs, model-free.**
Written and committed **before implementation, and before R24's arms
completed** — so this design cannot have been shaped by R24's outcome.

## The claim being tested

`turn-adjacency-diagnostic-2026-08-09.md` measured, on the archived R22 A0 arm:
**58 of 125 missed evidence turns (46.4%) sit immediately adjacent to a turn we
already retrieved**, 73 (58.4%) within ±2. Emitting neighbours would take
evidence-turn micro-recall 64.89% → **81.18%** (+16.29pp) at **2.62×** context.

That recall figure is **arithmetic, not a prediction** — if evidence is at
distance 1 and we emit all distance-1 neighbours, we emit it. **So recall is
not the interesting question here. The interesting question is whether the
tokens are better spent this way than any other way.**

## The primary comparison is token-matched — and it is the unflattering one

Comparing ±1 adjacency (2.62× context) against plain k=40 (1.00×) is not a
fair fight, and reporting that as the headline would be misleading. Adjacency
emits ~105 turns; so the control emits ~105 turns *without* adjacency.

| arm | config | context |
|---|---|---:|
| **A0** | k=40 baseline | 1.00× |
| **KMATCH** | **k=105, no adjacency — the honest control** | ~2.62× |
| **ADJ1** | **PRIMARY** — k=40 + emit all ±1 neighbours | ~2.62× |
| ADJ2 | k=40 + emit all ±2 neighbours | ~4.03× |

**PRIMARY COMPARISON: ADJ1 vs KMATCH.** The question is *"is dialogue adjacency
a better use of a fixed token budget than simply retrieving more?"* — not *"does
more context help?"*, which is already known and uninteresting.

ADJ1 vs A0 is reported as a **secondary** and explicitly labelled the
flattering comparison. ADJ2 vs a ~4.03× k-matched control is **not** run; ADJ2
is exploratory and reported without a verdict.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn micro-recall.
**Statistic:** Wilcoxon signed-rank on per-question evidence-turn count
differences, two-sided, α = 0.05 (`scripts/score_r24.py`), with the nonzero-pair
count always reported.

**PASS** requires *both*: p < 0.05 **and** micro-recall increase ≥ **+2.0pp**,
**against KMATCH**. Anything else is **NULL**. A significant decrease vs KMATCH
is **REFUTED** — and that is a live outcome: it would mean adjacency is a worse
use of tokens than plain k-raising, which is a genuinely useful thing to learn.

**Power rule, fixed in advance:** fewer than **15 nonzero pairs** ⇒ reported as
**STILL UNDERPOWERED**, not as a null. At full N with an effect this size,
nonzero pairs should be in the hundreds; if they are not, something is wrong
with the implementation and the run is void rather than null.

**Precondition:** A0's first 250 rows must reproduce 231/356 with 53
zero-evidence.

**Secondary, reported, non-decisive:** zero-evidence count, the multi-session
slice (44.70% baseline — every lever so far has made it worse), exact context
tokens per arm (the 2.62× figure must be *confirmed*, not assumed), and the
share of recovered turns that were the ±1 cases the diagnostic identified.

## Stated risks

- **Token cost is the real regression.** 2.62× context is a direct hit to the
  axis the cap work optimised. A retrieval PASS makes **no accuracy claim**: it
  is entirely possible that 2.62× context lowers end-to-end accuracy by
  drowning the reader, and we have no budget to find out. That limit is
  recorded here so a PASS cannot later be read as an accuracy result.
- **Adjacency may just be "more context" wearing a structural costume.** That
  is precisely what KMATCH tests, and why it is the primary.
- **Two-party dialogue is the ideal case.** LoCoMo has exactly two speakers
  alternating strictly (272/272 sessions). A multi-party or document corpus
  would not have this structure, so a PASS here is corpus-shaped and must be
  described that way.

## Registered non-goals

- **No paid runs, no embeddings, no model** (model-free is a project decision).
- **No cascade measurement**, therefore no cascade change.
- No tuning of the adjacency radius after seeing results. ±1 is primary, ±2 is
  exploratory, and no other radius is run.
- No combination with R24's speaker field in this prereg. If both pass
  independently, the combination is a separate prereg — combining after seeing
  two results is how a forking path gets laundered into an architecture.

**Register row:** R25. **Refs:**
`turn-adjacency-diagnostic-2026-08-09.md` (the pricing),
`speaker-attribution-diagnostic-2026-08-09.md` (the mechanism that predicted
adjacency), `rrf-composition-result-2026-08-09.md` (why ranking levers are
closed and why this is a different family).
