# Descriptions in the actor context — measured, negative result

**Date:** 2026-07-11
**Flag:** `SPECTRAL_ACTOR_DESCRIPTIONS` (bench formatter). **Verdict: keep OFF.**
**Spend:** ~$0.11 total (validation + both arms; deliberately sparing).

## Question

Librarian descriptions are a retrieval-side signal today (FTS column, item #8,
+2.5pp) — invisible to the actor. Does *also* showing the actor the per-turn
gloss improve **synthesis** on the categories FTS enrichment alone could not
move? And is any gain worth the token cost, given the "least expensive" goal?

## Method

The full LongMemEval dataset is not on this machine, and a random subset would
be too noisy for failure analysis. So: a **difficulty-engineered 10-question
micro-eval** (`scratchpad/micro_dataset.json`) targeting the documented
actor-synthesis failure modes — embedded reference (buried fact), counting /
aggregation, value-selection / knowledge-update, temporal ordering — plus 2
controls (obvious single-turn fact). Each answer-bearing turn carries an honest
Librarian gloss (the fact in that turn, never the answer to the question).

Clean isolation: **descriptions applied in BOTH arms** (constant FTS effect),
query expansion off (no confound), `max_results 40` (haystacks fully
retrieved, so retrieval is not the variable). The only difference:

- **Baseline** — descriptions in FTS only.
- **Treatment** — descriptions ALSO injected into the actor context as a
  labeled `[librarian: …]` note.

Actor + judge: Claude Sonnet 4.6. Verified end-to-end that the note actually
reaches the actor (`actor_context` in the report contains `[librarian:]`).

## Result

| | Baseline | Treatment |
|---|---|---|
| **Accuracy** | **9/10** | **9/10** |
| Actor-context tokens (sum) | 6,442 | 6,883 (**+7%**) |
| System $ cost | $0.0293 | $0.0326 (**+11%**) |

**Zero accuracy change. No flips in either direction on any of the 10
questions.** Treatment cost +7% context tokens / +11% dollars for +0pp.

## Failure analysis (why zero)

1. **The one failure (`emb-promo`) is a RETRIEVAL miss, not a synthesis miss.**
   The answer-bearing turn ("Marcus got bumped up to Director of Engineering")
   was **not retrieved** (only 3 of the session's turns came back, all
   assistant turns; the user turn with the fact was absent). Its context
   therefore had **zero** librarian notes in *both* arms. You cannot gloss a
   turn that was never retrieved — descriptions-in-actor-context is
   structurally unable to fix a retrieval miss. (Side-note worth a separate
   look: a turn containing the query entity "Marcus" was not retrieved —
   possibly a real retrieval quirk, but constant across arms and orthogonal to
   this question.)

2. **On the 9 retrieval-successful questions the treatment context DID carry
   the glosses** (1–3 notes each, verified) — and every one was already correct
   in the baseline. The actor extracted each buried fact from the **content**,
   which always states it; the gloss was **redundant**. Predictions were
   frequently byte-identical across arms ("Sarah", "8:30pm", "Rufus",
   "Mushrooms") — the gloss did not change the actor's reasoning.

## Conclusion (mechanistic, not just this sample)

Descriptions in the actor context are **redundant when the memory is retrieved**
(its content already states the fact) and **useless when it is not** (no turn
to attach the gloss to). Net: pure token cost, no accuracy benefit.
**Descriptions earn their keep at RETRIEVAL time** (bridging vocabulary so the
right turn is *found* — item #8's +2.5pp), **not at synthesis time.** For a
system optimizing "least expensive with best recall," the disposition is clear:
**leave `SPECTRAL_ACTOR_DESCRIPTIONS` OFF; keep descriptions retrieval-side.**

## Honest caveats

- **N=10, curated (author-written), not LongMemEval.** The regime where a gloss
  *could* still help — the actor failing synthesis *despite* the fact being in a
  very long context (LongMemEval multi-session counting over hundreds of turns)
  — did not occur at this context scale (Sonnet read every buried fact from
  content, and even counted correctly). Testing that regime needs the full
  dataset. But the redundancy mechanism holds regardless, and the burden of
  proof is on showing a benefit that beats the token cost — which this
  experiment could not find.
- **Stochastic actor**, but 9/9 identical outcomes on the retrieval-successful
  questions (often identical predictions) is a clean null with no borderline
  flips; repeats were not warranted.
- The +7% here is smaller than the standalone formatter measurement (+62% over
  the retrieval lines) because only retrieved turns get glosses and the full
  prompt (system + question) dilutes the description fraction. Both are real
  costs; both bought zero accuracy.

## Disposition

The feature stays as a **reversible, off-by-default flag** (a future
LongMemEval-scale test could revisit it), but this measurement is the record:
do not enable it expecting an accuracy win. The dataset, descriptions, and both
reports are under the session scratchpad for replay.
