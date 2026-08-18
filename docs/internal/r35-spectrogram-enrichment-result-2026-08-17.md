# R35 result — Librarian enrichment does not separate the spectral space

Prereg: [`r35-spectrogram-enrichment-prereg-2026-08-17.md`](r35-spectrogram-enrichment-prereg-2026-08-17.md).
Published regardless of outcome, as committed.

**Verdict: STOP on reviving retrieval. But the mechanism finding is positive and
actionable, and the retirement stands on a *fairer* test than before.**

## Corpus

Real production brain, read-only, n = 2,807 memories, **2,712 enriched (96.6%)**.
Statistics only; no content was printed or written. $0, local, no LLM.

## A flaw in the first run, and why it mattered

The first pass built each wing corpus from *every* memory including the one
under analysis. Every word was therefore already "seen", `novelty` collapsed to
~0.0002 with **variance ratio 0.00**, and a sixth of the fingerprint contributed
nothing. Production scores novelty at write time against *pre-existing*
memories, so this was an artefact of the harness.

Corrected to build corpora incrementally — a memory joins the corpus only after
it has been analysed. Novelty came alive (0.3350 mean) and the separation figure
moved. **The uncorrected run would have produced the same verdict for partly
wrong reasons.**

## Result

| dimension | mean A | mean B | mean Δ | var B/A |
|---|---:|---:|---:|---:|
| entity_density | 0.4695 | 0.5483 | **+0.0788** | 0.81 |
| decision_polarity | −0.0073 | −0.0074 | −0.0002 | **1.08** |
| causal_depth | 0.0630 | 0.0583 | −0.0046 | 0.66 |
| emotional_valence | 0.0591 | 0.0619 | +0.0028 | **1.05** |
| temporal_specificity | 0.0979 | 0.0851 | −0.0129 | 0.74 |
| novelty | 0.3350 | 0.3020 | −0.0330 | 0.76 |

| | |
|---|---:|
| mean pairwise distance, arm A (content) | 0.7666 |
| mean pairwise distance, arm B (content + description) | 0.7316 |
| **separation change** | **−4.6%** |
| any dimension moved >0.05 | 2,305 (85.0%) |
| **peak_dimensions set changed** | **1,136 (41.9%)** |
| action_type changed | 67 (2.5%) |

## Against the preregistered predictions

| # | prediction | outcome |
|---|---|---|
| 1 | ≥60% of memories change a dimension by >0.05 | **CONFIRMED** (85.0%) |
| 2 | <40% change their peak set | **REFUTED** — 41.9%. Enrichment reorders peaks *more* than expected |
| 3 | separation rises <25% | **CONFIRMED**, and then some: it *fell* 4.6% |
| 4 | novelty moves most | **REFUTED** — entity_density moved most (+0.0788 vs −0.0330) |
| 5 | entity_density rises | **CONFIRMED** (+0.0788) |

Two of five predictions were wrong, both in the direction of enrichment having
*more* mechanical effect than I expected. The decision rule required separation
≥ +25% **and** peaks ≥ 40%. Peaks passed. Separation failed decisively and in
the wrong direction, so the rule says stop.

## The finding that matters: enrichment homogenises

The headline is not "enrichment does nothing" — it does a great deal. **85% of
fingerprints move and 42% change which dimensions are their peaks.** The
problem is the *direction*.

**Four of six dimensions lose variance under enrichment** (entity_density 0.81,
causal_depth 0.66, temporal_specificity 0.74, novelty 0.76). Mean entity density
rises for nearly everything while its spread shrinks: descriptions push every
memory toward the same region of the space. Resonance matching needs memories to
be *distinguishable*; the current enrichment makes them more alike.

The two dimensions that *gained* variance are the editorial ones —
**decision_polarity (1.08)** and **emotional_valence (1.05)**. That is a real
signal about what kind of writing differentiates: when the Librarian expresses a
judgement it spreads the space; when it summarises facts it compresses it.

## Consequence for the vision

The hypothesis — enriched memories yield better spectral separation — is
**refuted as currently implemented**, not in principle. What is refuted is
*summarising* enrichment. The measured brief for enrichment that could work:

- **Differentiate, do not normalise.** Uniform summary prose in a consistent
  register is exactly what collapses variance.
- **Target the dimensions that respond.** Decision polarity and emotional
  valence are the two that gained spread; explicit judgements, commitments and
  reversals move them.
- **Preserve distinctive surface detail** — identifiers, dates, numbers, proper
  nouns — rather than paraphrasing it into generic phrasing. Temporal
  specificity and novelty both *lost* variance, meaning descriptions are
  smoothing away exactly the markers those dimensions read.

## Independent defect found on the way

`SpectrogramAnalyzer::analyze` computes all seven dimensions from
`memory.content` and never reads `memory.description`; `description` does not
appear anywhere in `spectral-spectrogram/src/`. The ORACLE_TIER0 null was
therefore measured on content-only fingerprints against what is now a 96.6%
enriched corpus.

That does not overturn the retirement — this probe fed the enrichment in and
separation still fell — but it does mean the original experiment never tested
the enrichment hypothesis. The retirement now rests on a test that did.

## Reusable instrument

`cargo run -p spectral-spectrogram --example enrichment_probe -- <memory.db>`

~2 minutes, $0, read-only, statistics-only output. When the Librarian's
enrichment style changes, re-run: separation going positive is the gate to a
preregistered retrieval arm. This turns "does enrichment help the spectrogram"
from an argument into a measurement that can be repeated cheaply.
