# The deep misses are a coreference gap, not a vocabulary gap (2026-08-09)

**$0. Derived entirely from the R22 A0 arm — no new retrieval, no model calls.**
Diagnostic only. **No lever is claimed here, and none may be claimed without
its own prereg.**

## Why this exists

R22 refuted the composition hypothesis and left two queued levers, the first
being **vocabulary bridging via `query_aliases`** for what the failure analysis
called "true vocabulary misses."

`query_aliases` is a **consumer-curated table**. "Testing" it means authoring
one, and authoring it against the questions it will be scored on is fitting to
the evaluation set — the resulting number would be meaningless. So the lever was
priced first, by diagnosing the *shape* of the failures. That shape is a
property of the corpus and does not depend on any table we might write.

## The shape

All 70 missed evidence turns from questions that retrieved **zero** evidence:

| overlap with question (stemmed content words) | n | share |
|---|---:|---:|
| **0 shared — no lexical bridge** | 44 | **62.9%** |
| 1 shared — thin: admitted by FTS, ranked low | 23 | 32.9% |
| 2+ shared — ranking problem | 3 | 4.3% |

The 32.9% reproduces G4 exactly (88.8% of deep misses carry at most one query
term) and belongs to the ranking family, which is closed.

Only the 62.9% could be addressable by aliases. It is not.

## What the 62.9% actually is

**44 of 44 — 100% — name a proper noun in the question that never appears in
the evidence turn.** Not one is a pure predicate paraphrase.

> **Q:** What are some problems that Andrew faces before he adopted Toby?
> **A:** *Finding a pet-friendly place to live has been tough too. I'm
> contacting landlords and checking out neighbourhoods.*

> **Q:** Why did Evan decide to get the bonsai tree?
> **A:** *I got this because it symbolizes strength and resilience.*

The evidence is Andrew's own turn. Andrew does not say "Andrew"; he says "I".
LoCoMo carries no speaker metadata — turns have only `role` and `content`, and
the names surface as vocatives inside the text ("Hey John!", "Cool, James!").

**This is coreference, not synonymy.** A word→word table cannot express
"utterances *by* X" — and a table that hard-coded `Toby → pup` would be
corpus-specific knowledge lifted from the answers, i.e. fitting.

## The mechanism, quantified — an 8.5× inversion

For zero-evidence questions naming a person:

| | contains that name |
|---|---:|
| **missed evidence turns** | **4.3%** (3/70) |
| **retrieved top-40 turns** | **36.6%** (776/2120) |

**BM25 spends its top-40 on turns that _mention_ the person while the evidence
almost never mentions them.** The name is a high-IDF term, so matching it is
the single strongest thing BM25 can do — and on first-person dialogue it is
reliably the *wrong* thing: the person's name appears when *someone else*
addresses them, not when they speak.

This is a concrete mechanism for failure-analysis §3, which measured missed
evidence at **0.46× query-term overlap** against the distractors that outrank
it and concluded "BM25 is ranking correctly by its own criterion." It is right,
and this is *why* — the criterion is inverted for the single most
discriminative term in the query.

It also explains why every lexical refinement returned a null. Porter,
widening, spreading, proximity and RRF all re-weight or re-order a candidate
set selected by that inverted criterion. **None of them can add the turns that
were never competitive**, because those turns match the query on nothing but
common words.

## What this points at — NOT yet a claim

**Speaker attribution.** If a turn spoken by Andrew were retrievable *as*
Andrew's turn, "what problems did Andrew face" would match on the strongest
term in the query instead of being actively misled by it.

Two things must be true before this is worth measuring, and neither is
established:

1. **The name→speaker binding must be derivable deterministically.** In LoCoMo
   it would have to be inferred from vocatives, and inferring it from the
   evidence turns would be fitting. A prereg must fix the inference rule and
   demonstrate it on a held-out conversation **before** any arm runs.
2. **It must not be a LoCoMo artefact.** Our real consumer (Permagent) has
   speaker identity as *metadata*, not as text to be inferred — so for them
   this is a plumbing question, not an inference one. That asymmetry makes
   LoCoMo the harder case and the honest one to measure, but it also means a
   LoCoMo result would understate the production case rather than overstate it.

**Registered non-goals:** no alias table will be authored against these 250
questions; the ~34% "true vocabulary miss" figure should be read as **~63%
coreference / ~33% thin-lexical**, not as a synonym opportunity; and nothing
about this justifies re-opening the ranking family.

## Reproducing

```bash
python3 scripts/diagnose_lexical_misses.py \
  --rows a0.jsonl \
  --dataset ~/spectral-local-bench/locomo_full_answerable_labelled.json
```

The stopword list is deliberately broad: a word that FTS treats as a stopword
is still a word an alias table could bridge, so being generous can only make
the addressable bucket look **larger**. The conclusion is safe in the direction
that matters.

**Refs:** `rrf-composition-result-2026-08-09.md` (R22, which queued this),
`failure-analysis-2026-08-08.md` §3 (explained here),
`g4-proximity-result-2026-08-08.md` (the 32.9% bucket).
