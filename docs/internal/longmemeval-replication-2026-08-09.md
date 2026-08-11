# R24 does not replicate on LongMemEval — because the mechanism is not there

**$0. Offline backfill onto archived arms plus diagnostics. No new retrieval,
no model calls.** Attempted as the second-corpus replication of R24.

**Result: the replication cannot be run as specified, and that bounds the R24
claim rather than supporting it.** Recorded because a lever that appears to work
everywhere is less credible than one whose domain is measured.

## Why it cannot be run

R24 restores **speaker names** so a question naming a person can match the turns
that person spoke. LongMemEval has no named speakers:

- Turns carry `role` (`user` / `assistant`) and `content` only — no `speaker`,
  and no raw source with one to restore.
- Questions ask about **"I"** ("What breed is my dog?", "How many playlists do
  I have?"). The capitalized tokens in them are **places and brands** — Spotify,
  Costa Rica, Lake Michigan — not people.

There is no speaker metadata to attach and no named-person coreference to
resolve. **This is a structural absence, not a failed measurement**, and it is
the honest reason R24 stays a LoCoMo-scoped result.

## What DOES generalize: the failure family

Evidence labels backfilled onto the archived 500-question arm
(`oracle-evidence`, $0, offline), then the same diagnostic used on LoCoMo:

| missed evidence turns in zero-evidence questions | LongMemEval | LoCoMo |
|---|---:|---:|
| **0 shared content words** | **72.1%** | 62.9% |
| 1 shared (thin, ranked low) | 23.3% | 32.9% |
| 2+ shared | 4.7% | 4.3% |

**The "no lexical bridge" failure mode is not a LoCoMo artifact — it is worse on
LongMemEval.** That is the general finding, and it survives.

## What does NOT generalize: the specific pathology

The LoCoMo mechanism was that BM25 spends its budget on turns *mentioning* the
queried person while the evidence is that person's own turn — a 4.8× inversion
(38.4% of retrieved turns contained the name against 8.1% of missed evidence).

On LongMemEval, for zero-evidence questions naming a capitalized entity, only
**18.9% (17/90)** of retrieved turns contain that entity — **half** LoCoMo's
38.4%. The pathology R24 corrects is roughly half as strong, and there is no
speaker channel to correct it through.

The referent is also different in kind:

> **Q:** What breed is my dog?
> **A:** *I'm thinking of getting **Max** a new collar with a nice name tag.*

The missing bridge is **"my dog" ↔ "Max"** — an *entity* reference, not a
speaker one. Same family, one level up: the question names a thing by category
and the evidence names it by proper noun.

## What this changes

- **R24's claim is now bounded by measurement rather than by caveat.** It
  requires a corpus with named speakers. LoCoMo has them; LongMemEval does not;
  Permagent does (speaker identity is metadata there).
- **The next generalization is entity attribution, not speaker attribution** —
  binding "my dog" to "Max". That is a genuinely harder problem: speaker
  identity is metadata we already hold, whereas entity coreference must be
  derived, and deriving it from the evidence would be fitting. **No lever is
  proposed here.**
- **A null on LongMemEval would have been the wrong conclusion.** The lever
  cannot be applied at all, so reporting "it failed to replicate" would have
  implied the mechanism was tested and found absent, when in fact the
  *precondition* for the mechanism is absent.

## Honest limits

- The archived arm used the **Cascade** path at k=30; the LoCoMo work used
  `topk_fts` at k=40. Fine for characterising a failure mode, not a
  like-for-like comparison, and the two inversion percentages should be read as
  indicative rather than paired.
- The entity-inversion sample is small (90 retrieved turns) and the
  capitalized-token heuristic conflates places, brands and names.
- 21 of 500 questions are excluded for carrying no `has_answer` label
  (LongMemEval's `_abs` abstention set); they are undefined, not zero.

**Refs:** `speaker-field-result-2026-08-09.md` (R24),
`speaker-attribution-diagnostic-2026-08-09.md` (the LoCoMo mechanism),
`r19-locomo-turn-labels-2026-08-08.md`, `turn-level-evidence-recall-2026-08-07.md`.
