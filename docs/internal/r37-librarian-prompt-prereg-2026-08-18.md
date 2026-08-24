# R37 prereg — a Librarian prompt written to the R36 brief

Preregistered 2026-08-18, before any arm result was read.

## Question

R35/R36 measured the current Permagent Librarian enrichment against the
spectrogram and recognition engines and found it *hurts* both — separation
−4.6%, landmark density −27.8% — while never destroying a verbatim anchor. The
brief that came out of it (`r36-enrichment-landmarks-result-2026-08-17.md`):
protect identifiers, cut connective prose, lean into judgement, prefer specific
nouns. Jesse's decision: the Librarian should write in whatever style best
serves Spectral's recognition libraries.

Does a prompt rewritten to that brief pass the two gates R36 set?

## What changes

`LIBRARIAN_SYSTEM_PROMPT` in
`permagent-runtime/crates/goose/src/agents/platform_extensions/librarian.rs`
(unchanged since 2026-05-13). The three-field wire format
(`FACTS … Related terms: …. Categories: ….`) is kept — `annotate_memory` parses
it into `term:`/`cat:` entity refs — so only the *content rules and examples*
change:

- FACTS: terse, ≤25 words, fragments allowed, lead with the outcome/decision,
  copy identifiers/numbers/dates/versions/paths/error codes verbatim, no filler.
- TERMS: proper nouns, names, identifiers, versions, error codes, dates and
  specific technical nouns; **no inflected forms** (the FTS index is
  `porter unicode61` — stemming already bridges them); no generic words.
- CATEGORIES: the concrete subject, never a genre.

Everything else in the pipeline is held fixed: `mask_opaque_ids`, the 2,000-char
truncation, model `qwen2.5:7b`, `temperature 0.2 / top_p 0.9 / num_predict 150`,
the parser and its floors (4 terms, 2 categories), one retry then raw fallback.

## Design

Paired, same model, same host, same sample. n = 300 memories drawn with seed 37
from the 2,712 enriched memories of the real brain, `~/.permagent/brain`.

Three arms, each written into a **copy** of `memory.db` in which only the 300
sample memories carry a description (all others NULL), so both instruments
measure exactly the same memory set:

- `stored` — the descriptions currently in the brain (mixed provenance).
- `old_regen` — the current prompt, regenerated now with `qwen2.5:7b`.
- `new_regen` — the rewritten prompt, same model, same run.

**The comparison that decides is `new_regen` vs `old_regen`** — same model,
same day, only the prompt differs. `stored` is context.

Instruments (read-only, statistics only, both from `spectral` main):

```
target/release/examples/enrichment_landmarks <db>   # R36, recognition
target/release/examples/enrichment_probe     <db>   # R35, spectrogram
```

## Decision rule (fixed before results)

PASS requires all of:

1. **Density gate** — landmark density per 1k chars (content+desc) is HIGHER
   for `new_regen` than `old_regen`.
2. **Separation gate** — mean pairwise fingerprint separation change
   (content → content+desc) is POSITIVE for `new_regen`. (Old was −4.6% on
   the full corpus; the sign on this subsample is what counts.)
3. **Anchor guard** — memories losing a verbatim anchor stays at 0.
4. **Fallback guard** — the raw-fallback rate for `new_regen` is not more than
   5pp above `old_regen`. If it is higher because sparse memories honestly
   yield fewer than 4 terms, that is a parser floor to revisit in the same PR,
   reported as such — not silently absorbed.

Anything short of all four is reported as FAIL or PARTIAL with the numbers.
No further prompt tuning is done against this sample after reading results —
a second iteration gets a fresh seed.

## Not measured here

FTS recall on the real brain (no ground truth exists for it). The risk from
dropping inflected TERMS is bounded by the stemmer; the risk from dropping
generic CATEGORIES is to `cat:` hub entities, which at present link thousands of
memories through nodes like `cat:software development` and are noise as
relations. Both are stated as assumptions, not results.

$0, local, ~40 min of generation.
