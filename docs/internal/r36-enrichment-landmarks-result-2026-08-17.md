# R36 result — Librarian enrichment and the recognition engine

Companion to [R35](r35-spectrogram-enrichment-result-2026-08-17.md), which
measured the spectrogram. Recognition is a different engine and the brief does
**not** transfer, so it was measured separately rather than assumed.

Corpus: real production brain, read-only, **2,712 enriched memories**.
Statistics only. $0, local, no LLM.

## Result

| | arm A (content) | arm B (content+description) | change |
|---|---:|---:|---:|
| landmarks per memory | 19.03 | 26.26 | **+38.0%** |
| landmark density (per 1k chars) | 63.64 | 45.96 | **−27.8%** |
| median density | 67.90 | 53.33 | — |
| verbatim anchors per memory | 7.88 | 8.39 | **+6.5%** |

| | count | share |
|---|---:|---:|
| memories gaining an anchor | 1,041 | 38.4% |
| memories **losing** an anchor | **0** | **0.0%** |
| all content anchors preserved | 2,273 | 83.8% |

## Reading: net positive, but inefficient

Unlike the spectrogram, where enrichment actively pointed the wrong way, for
recognition it **helps**:

- **No anchor is ever destroyed** — 0 memories lose one. Anchors are verbatim
  numbers, identifiers and error codes: the strongest evidence recognition has,
  because they are both rare and exactly repeatable across a re-encounter. The
  Librarian is *not* paraphrasing identifiers away. That is the single most
  important property here and it should be protected in any style change.
- **38.4% of memories gain an anchor**, and absolute landmark count rises 38%.

But it is **verbose from recognition's point of view**. Density falls 27.8%:
the 283 characters of description buy roughly 7 extra landmarks, and most of the
added text is connective summary prose the engine cannot key on. More material
to match, diluted with more material that will never match.

## Consequence

Recognition would benefit from the *same* change the spectrogram needs, arrived
at from a different direction:

| engine | what it needs | what current enrichment does |
|---|---|---|
| spectrogram | memories **spread apart** in feature space | homogenises — 4 of 6 dimensions lose variance |
| recognition | **dense, rare, stable** landmarks | adds landmarks but dilutes density 27.8% |

Both are hurt by the same thing — **uniform, connective, summarising prose** —
and both are helped by the same fix: **terse, specific, distinctive, judgement-
bearing enrichment**. Density rises for recognition; variance rises for the
spectrogram. There is no trade-off between the two engines to arbitrate.

## Brief for the Librarian, measured rather than asserted

1. **Never paraphrase away identifiers, numbers, dates or error codes.**
   Currently at 0% anchor loss — this is working, protect it.
2. **Cut connective and summarising prose.** It costs density without adding
   landmarks and it collapses spectral variance.
3. **Vary register; lean into judgement.** Decision polarity and emotional
   valence are the only two dimensions that *gained* spread — explicit
   decisions, commitments, reversals and stance differentiate; neutral summary
   does not.
4. **Prefer specific nouns over generic phrasing.** Serves both engines at once.

## Instrument

```
cargo run -p spectral-recognition --example enrichment_landmarks -- <memory.db>
cargo run -p spectral-spectrogram  --example enrichment_probe      -- <memory.db>
```

Both read-only, statistics-only, ~2 minutes, $0. Re-run after any Librarian
style change: **density up** and **separation positive** are the two gates.
