# Substrate Measurement Framework

## Purpose

This framework defines how the recognition layer's value will be measured
once production substrate accumulates. It exists because LongMemEval
structurally cannot test the recall→recognition→feedback loop — the bench
instantiates a fresh usage-history-free brain per question, so co-retrieval
pairs, auto-reinforce, and ambient boost have nothing to operate on.

The recognition layer's measurement instrument is Henry's brain, not
LongMemEval.

## Context

- 2026-05-15: Permagent activated query enrichment. Retrieval events began
  streaming with ambient context attached (focus_wing, recent_activity,
  persona, session_id).
- Substrate accumulation expected to mature in ~2 weeks of normal usage.
- This framework is pre-committed *before* looking at the data, to prevent
  the data from writing its own success criteria.

## The Four Claims

The recognition layer makes specific claims about what it does. Each claim
has a measurement, a what-to-look-for, and a pre-committed success
criterion.

### Claim 1 — Ambient boost reranks results usefully

**The claim:** When a query carries wing context, ambient boost's 1.5x
wing-match multiplier and 0.7x non-match penalty push the *right* memories
higher in ranking. Without ambient context, those same memories would rank
lower or be missed.

**The measurement:** A/B comparison on the same query corpus, with and
without ambient context attached.

Take ~50 real queries from the accumulated retrieval events. For each,
run the query twice — once via `recall_cascade` with the real
`RecognitionContext`, once with an empty context. Compare top-K results.

**What to look for:**
- **Reranking magnitude:** how often do the top results actually differ
  between context-on and context-off? If they're identical 90% of the
  time, ambient boost isn't doing meaningful work.
- **Reranking direction:** when results differ, which version's top
  results are *better* — judged by personal inspection of 10-20 samples.
- **Edge case:** queries where context-on returns *worse* results. If
  this happens, ambient boost is overweighting somewhere.

**Success criterion (pre-committed):** In a sample of 20 query pairs
judged personally, context-on is *better* in at least 12, *equivalent* in
most others, *worse* in at most 2-3. If context-on is worse more often
than that, ambient boost is mistuned.

**Cost:** 30-45 minutes of personal judgment time, once.

### Claim 2 — Co-retrieval pairs cluster around real patterns of use

**The claim:** Memories that surface together in real queries reflect
actual conceptual neighborhoods in the work. The co-retrieval graph is
not just noise.

**The measurement:** After ~2 weeks of usage, inspect the top
co-retrieval pairs by frequency.

For the top 20-30 most-frequently-co-retrieved memory pairs, ask: *do
these memories actually belong together in the mental model?*

**What to look for:**
- **Signal-to-noise:** what fraction of the top 30 pairs feel
  meaningfully related?
- **Distribution shape:** is there a clean head of high-frequency pairs
  (real patterns) or is everything flat and low-frequency (sparse usage)?
- **Surprise:** are there pairs that *correctly* belong together that
  weren't predicted? This is the strongest positive signal — co-retrieval
  finding structure that wasn't explicitly encoded.

**Success criterion (pre-committed):** ≥60% of top 30 pairs are
meaningfully related when judged. Distribution has a real head (top 10
pairs have >2x the frequency of pairs 20-30). At least 2-3 pairs surprise
positively.

**Cost:** 15 minutes of inspection.

### Claim 3 — signal_score is being modulated by real usage

**The claim:** Auto-reinforce is updating signal_score based on what
actually gets recalled. Memories retrieved frequently drift toward higher
signal_score; memories never touched do not.

**The measurement:** Snapshot the signal_score distribution at framework
start (initial values, week 0) and again in two weeks (week 2). Compare.

**What to look for:**
- **Variance change:** is the distribution spreading out (some memories
  drifting up, others not), or staying static (auto-reinforce not firing)?
- **Direction:** are the memories that drifted *up* actually the ones
  expected to be more important — connected to active projects, recent
  decisions? Or is the drift orthogonal to relevance?
- **Time decay:** is there evidence of decay on memories that weren't
  touched? Or is the system only reinforcing, not decaying?

**Success criterion (pre-committed):** Distribution standard deviation
increases by at least 20%. The top 20 highest-signal_score memories at
week 2 are recognizably more "important to current work" than the top 20
at week 0. No evidence of pathological one-way drift (everything going
up, or everything going down).

**Cost:** Two SQL snapshots plus eyeballing the top-20 list. Minimal.

### Claim 4 — The system gets noticeably more useful in real work

**The claim:** Two weeks from now, when Henry's queries hit the brain
during real work, the answers should be measurably better than they are
at framework start, because the brain is consulting accumulated signal.

**The measurement:** A daily journal, two lines per day, ~10 seconds:
- One time the brain query produced a usefully precise answer
- One time it missed or returned irrelevant context

Maintained for two weeks. At the end, examine the trajectory.

**What to look for:**
- Is the ratio of hits to misses improving over the two-week window?
- Are the misses concentrated in a specific category? (Which would
  identify what to fix.)

**Success criterion (pre-committed):** In days 8-14 of the journal, hits
outnumber misses by at least 2:1, and the misses cluster in patterns that
can be articulated. If failure modes can't be named after two weeks of
logging, the system isn't producing enough signal yet — wait longer.

**Cost:** 10 seconds per day for 14 days. ~2.5 minutes total active time.

## Future Dispatch — Analysis Tooling

~10 days into the substrate accumulation window, dispatch Spectral CC to
build the analysis queries needed to execute the framework:

- A/B reranking comparison harness (claim 1) — script that runs N real
  queries through `recall_cascade` with and without `RecognitionContext`,
  produces side-by-side result sets for human judgment.
- Co-retrieval inspection query (claim 2) — SQL or harness that returns
  the top 30 co_retrieval_pairs by frequency with the memory content
  visible for inspection.
- signal_score snapshot diff (claim 3) — captures the current distribution
  and the top-20 list, in a format that diffs cleanly against an earlier
  snapshot.

Frame as future dispatch, not current TODO. The reason to wait is that
the analysis queries depend on what the real data shape looks like —
better to write them against real data than guessed data. Probably
half a day of CC time once dispatched.

## Honest Caveat

This framework is "good enough to start." Real measurement frameworks
evolve as the data surfaces unanticipated patterns. The first two weeks
may reveal that some measurement is harder than framed here, or that
there's a fifth claim that was missed. The version above is the
pre-committed starting point; it gets refined when real data lands, but
the original thresholds are preserved as the *initial* discipline so the
data can't write its own criteria.

## What This Framework Is For

Three uses:

1. **The measurement review itself.** ~2 weeks after activation, this is
   the doc that gets executed against real substrate.
2. **Project artifact.** This is the document that proves the recognition
   layer works (or doesn't) — to me, to anyone reviewing the project, to
   future technical writeups or audits.
3. **Discipline preservation.** Pre-committing thresholds before looking
   at data is the discipline that's served this project at every prior
   inflection point. Capturing the thresholds in writing — not just in
   conversation — makes the discipline auditable.

## Related Artifacts

- `analysis/shipped-components-active-dormant-audit.md` — what was
  dormant and what's now active
- `analysis/recall-ambient-contract-spec.md` — the contract ambient
  enrichment activates against
- `analysis/librarian-fts-format-alignment.md` — corpus quality
  precondition (confirmed aligned)
- Backlog item #28 — tracking entry for this framework

## Open Questions

None at framework-creation time. Questions are expected to emerge once
real data arrives.
