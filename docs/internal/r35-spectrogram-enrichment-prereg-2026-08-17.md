# R35 prereg — does Librarian enrichment change the spectrogram's dimensions?

**Written before measurement. Published with the result regardless of outcome.**

Date: 2026-08-17. Author: Claude (Opus 5), at Jesse Sharratt's direction.

## Why this reopens a closed gate

Spectrogram-as-recall is **RETIRED**: enabling write-time spectrograms changed
**0/500** retrieval contexts (`ORACLE_TIER0`, 2026-07-02). This project's rule
is that a failed gate does not license improvisation, so reopening requires a
*structural* reason to believe the original experiment did not test the
hypothesis actually held.

There is one, and it is mechanical rather than hopeful:

`SpectrogramAnalyzer::analyze` computes all seven dimensions from
`memory.content` **only**:

```rust
let ed = dimensions::entity_density(&memory.content);
let cd = dimensions::causal_depth(&memory.content);
let nv = dimensions::novelty(&memory.content, &context.wing_corpus);
// ... and four more, all &memory.content
```

`Memory::description` — the prose gloss written by Permagent's Librarian —
exists on the struct and is **never read anywhere in `spectral-spectrogram`**
(verified by grep: zero occurrences of `description` in that crate's `src/`).

Meanwhile the production brain is now **96.6% enriched**: 2,712 of 2,807
memories carry a Librarian description (mean 283 chars against mean content
823 chars), and 135 of 136 entities do.

So ORACLE_TIER0 measured *content-only* fingerprints. The hypothesis "enriched
memories yield better-separated spectral peaks" has never been tested, because
the enrichment is not wired to the analyzer. This prereg tests the mechanism
before spending anything on a retrieval arm.

## Hypothesis

**H1.** Including the Librarian description in the analyzed text materially
changes the seven-dimensional fingerprint, and specifically **increases the
separation** between memories in that space.

The reasoning: resonance matching compares fingerprints. If every memory lands
in a tight cluster in 7-space, "resonant" is indistinguishable from "arbitrary"
and no retrieval gain is possible in principle. Enrichment could plausibly
spread the distribution by adding entities, causal language, and temporal
markers that raw activity text lacks.

## Design

Corpus: the real production brain, `~/.permagent/brain/`, opened **read-only**,
n = 2,807 memories (2,712 enriched). Access granted by the data owner.

Two arms, same analyzer, same config, differing only in the text analyzed:

- **Arm A (status quo):** `content`
- **Arm B (enriched):** `content + "\n" + description`

`wing_corpus` for novelty is built per-wing from the same arm's text, so each
arm is internally consistent.

Cost: $0, local, no LLM, single process. No retrieval run in this stage.

## Preregistered predictions

Stated before running. I expect H1 to be **partially** supported — a real shift
in dimension values, but **not** enough separation to revive retrieval:

1. **Dimensions move.** At least 60% of enriched memories change their
   fingerprint measurably (any dimension moving by >0.05).
2. **Peaks move less than dimensions.** Under 40% of memories change their
   `peak_dimensions` set. Magnitudes shift more easily than rank order.
3. **Separation improves only slightly.** Mean pairwise distance in 7-space
   rises by **less than 25%** relative to Arm A.
4. **Novelty is the dimension that moves most**, because it is a
   corpus-overlap ratio and descriptions introduce vocabulary the raw wing
   corpus lacks.
5. **Entity density rises**, since descriptions name things activity text
   refers to obliquely.

## Decision rule, fixed in advance

- **Advance to a retrieval arm** only if mean pairwise separation rises by
  **>= 25%** *and* >= 40% of memories change their peak-dimension set. Both,
  not either — a shift that does not reorder peaks cannot change which
  memories are judged resonant.
- **Stop** if separation rises < 25%. Report the null and leave the retirement
  standing. In that case enrichment is not the missing ingredient and the
  ORACLE_TIER0 verdict survives on a fair test rather than an unfair one.
- Either way, **wire `description` into the analyzer** if H1's mechanism is
  confirmed at all, because the analyzer silently ignoring 96.6% of available
  signal is a defect independent of whether it revives retrieval.

## What this cannot establish

Results come from one private corpus and are **not reproducible by third
parties**, so they can never be a published headline figure under the honesty
guardrails. This is a *mechanism check* and a *deployment-validity check*, not
a benchmark.

A positive result here is necessary but nowhere near sufficient: it would earn
a preregistered retrieval arm on the public LongMemEval oracle, which is where
any accuracy claim would have to be made.

## Privacy

Output is **statistics only** — counts, means, variances, distances. No memory
content, no descriptions, no keys, no ids are printed or written. The harness
opens the store read-only.
