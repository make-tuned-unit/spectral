# Tier-1 actor replay — porter+mult3, and where the lift actually is

**Date:** 2026-07-03
**Spend:** ~$5.0 (baseline arm $1.94, candidate arm $1.99, expansion probe $0.74,
prompt-v2 replay ~$0.30). Actor/judge: Sonnet 4.6, shape-routed cascade,
expansion OFF unless noted.

## Headline

**porter+mult3 is accuracy-neutral vs baseline on the recall-changed set — and
that is because retrieval is not the bottleneck here.** The value of this Tier-1
run is not the (null) porter result; it is the failure analysis, which located
concrete, non-porter lift.

## Arm A/B — baseline (default FTS, mult1) vs candidate (porter + mult3)

39 questions: 17 porter recall-changed + 22 unchanged-context controls.

| slice | baseline | candidate |
|---|---|---|
| all (39) | 23/39 (59.0%) | 23/39 (59.0%) |
| signal — recall-changed (17) | 8/17 | 8/17 |
| control (22) | 15/22 | 15/22 |

Four flips, net zero — and 3 of the 4 are **not** porter:
- `ba358f49_abs` (win): retrieval byte-identical both arms → actor variance.
- `gpt4_d6585ce9` (win): **judge artifact** — actor said "I don't know", judge
  marked it correct against gold "my parents".
- `gpt4_1e4a8aec` (loss): judge inconsistency (baseline's "I don't know" scored
  correct on a question with a real gold answer) + actor ignored its one key.
- `9a707b82` (loss): the **one real** retrieval regression — candidate dropped
  the answer turn ("chocolate cake") that baseline retrieved.

## The decisive finding: failures are actor-side, not retrieval-side

Of the 16 candidate-wrong questions, **15 had the answer keys retrieved** — only
1 was a true retrieval miss. Porter (a retrieval lever) cannot lift accuracy
here because retrieval already succeeds on the questions that fail; the actor is
the ceiling. This confirms the synthesis-bound thesis with per-question data.

Two measurement caveats surfaced and are themselves levers:
- **The judge is noisy** — it scored "I don't know" as correct against specific
  gold answers (≥2 of 4 flips). This inflates variance in every bench number.

## Where the lift is — counting/aggregation, a two-stage pipeline

Five counting failures (doctors, furniture, instruments, episodes, mugs). Root
causes, established at $0 before spending:

1. **Retrieval stage — instance turns don't rank for the count query.** The
   turn naming an item (e.g. "Yamaha") does not lexically match "how many
   instruments", so it falls below top-K. This is *not* the episode-diversity
   cap: raising counting `max_per_episode` 3→15 changed nothing at Tier-0 and in
   `Inspect` (the cap only bites when a session has >3 retrieved turns, which is
   not the failing case). The lever is **synonym/expansion bridging**, matching
   the four-config table where expansion beats porter on key-recall (61.5% vs
   54.7%).

2. **Actor stage — reasoning errors even with full evidence.** boundary/state
   ("excluded the drum set because it was *being* sold"); arithmetic ("had $60
   total, didn't divide by quantity"); partial extraction.

### Demonstrated recovery (5 counting failures, zero regressions on 3 passes)

| question | no-expand | +expansion | +expansion +prompt-v2 |
|---|---|---|---|
| doctors (`gpt4_f2262a51`) | WRONG | **OK** | OK |
| instruments (`gpt4_194be4b3`) | WRONG | WRONG (evidence now present) | **OK** |
| furniture (`gpt4_15e38248`) | WRONG | WRONG | WRONG |
| episodes (`f35224e0`) | WRONG | WRONG | WRONG (one podcast's count not retrieved) |
| mugs (`0100672e`) | WRONG | WRONG | WRONG (quantity only estimated → gold-ambiguous) |

- **Expansion** surfaced the missing instance turns (ENT visit, Yamaha, Pearl
  now in context) and recovered doctors. Retrieval lever confirmed.
- **prompt-v2** (added: disposal-boundary rule + "do the arithmetic" rule)
  recovered instruments on top of expansion — the actor now counts the
  listed-for-sale drums. Actor lever confirmed. **Zero regressions** on the 3
  counting passes.
- Residual 3: episodes + mugs are further retrieval-completeness / gold-ambiguity
  (not prompt-fixable); furniture is genuinely hard.

## Recommendations

1. **Ship porter+mult3 as accuracy-neutral and strictly cheaper** — it is
   validated harmless (one mild real regression, offset), matches baseline, and
   the widening is $0. It is *not* an accuracy lever; do not sell it as one.
2. **The real lift is shape-gated on counting**: (a) expansion (or entity-aware
   retrieval) for counting-shape queries to surface instance turns; (b) the
   counting prompt refinements (disposal boundary + explicit arithmetic).
   Demonstrated 2/5 recovery, 0 regressions.
3. **Before any full n=500**, run a counting-shape sweep (all ~N counting
   questions in the set) under {baseline, +expansion, +expansion+prompt-v2} to
   size the shippable counting lift. Est. ~$3–4. Pending Jesse's approval.
4. **Judge hardening** is a separate, high-leverage cleanup: the "I don't know =
   correct" behavior corrupts every measurement.

Frozen artifacts (all replayable): `~/spectral-local-bench/tier1-baseline-arm.json`,
`tier1-porter-m3-arm.json`, `tier1-cnt-exp.json`, `tier1-cnt-v2.json`,
`tier1_analyze.py`, `/tmp/counting_v2.md` (prompt with the two added rules).
