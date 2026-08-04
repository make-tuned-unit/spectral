# Read-time supersession suppression — REJECTED (extraction precision)

Prereg: `supersession-prereg-2026-08-03.md`. The prereg's stated failure mode is
the one that occurred.

## Verdict

**REJECTED at gate 3.** The lever fires, but overwhelmingly on the wrong text.
Default stays OFF.

## Direct extraction measurement

`crates/spectral/examples/supersession_coverage.rs` runs the real
`supersession::topic_key` over the LongMemEval-S haystacks — no retrieval, no
brains, no confound:

| quantity | value |
|---|---|
| haystack turns | 246,930 |
| turns yielding a topic key | **2,529 (1.02%)** |
| questions with one topic asserted in >1 session (suppressible at all) | **41 / 500 (8.2%)** |
| suppressible topic groups | 43 |

Coverage that low is survivable. Precision is not. Sampled extractions:

```
[aim]              As an AI language model, I have access to a vast amount of information…
[knowledge]        My knowledge is derived from publicly available data, so here's a broad…
[language]         I apologize if my previous response was too lengthy or unclear…
[team and ensure everyone]  I'll definitely check out these tools… Since I'm leading a team of five engi…
[main character]   I think I'll try to start small and focus on a specific region…
```

The `my <attr> is …` frame matches **assistant self-description boilerplate**
far more often than user facts. "My knowledge is derived from…" is not a fact
about the user that can be superseded; it is chatter that recurs in every
session, so it looks exactly like a repeatedly-restated attribute.

Suppressing those is mostly harmless and entirely useless. The 8.2% of
questions where a genuine conflict might exist is not worth a mechanism whose
dominant behaviour is misfiring.

## The oracle run was confounded — no safety conclusion drawn from it

The paired run (`SPECTRAL_SUPERSESSION=1`, knowledge-update + temporal-reasoning,
211 questions) changed the context on **209/211** questions. That is not
suppression firing that often — it is the **pool widening** the lever enables
(`widen = 2`), which changes `pipeline_config.k` and therefore what
`max_per_episode` diversity selects.

So the run measures *supersession + widening* as a bundle and cannot attribute
between them:

| | baseline | bundle | Δ |
|---|---:|---:|---:|
| knowledge-update sess-rec | 99.4% | 99.4% | 0.0 |
| knowledge-update key-rec | 58.1% | 57.7% | −0.4pp |
| temporal-reasoning sess-rec (control) | 96.0% | 96.2% | +0.2pp |
| answer keys, knowledge-update | 1061 | 1053 | −8 |
| answer keys, temporal-reasoning | 1605 | 1600 | −5 |

Gates 1 and 2 are within tolerance on these numbers, but **the control moving
at all is itself the evidence of confound** — supersession should not touch
temporal-reasoning. I am not recording a safety pass from a confounded arm.
Designing the run without a widening-only control arm was a repeat of the
mistake the run-3 answerability design had already fixed with its arm D.

## The prereg predicted this

Verbatim from the prereg, written before the measurement:

> If gate 3 fails, the honest conclusion is that deterministic supersession
> extraction needs a broader pattern set than can be written conservatively —
> which is an argument that this belongs in the graph/triple layer (where
> Spectral already has real supersession) rather than in a regex over free text.

That is the conclusion. Widening the patterns would raise coverage and lower
precision, and precision is already the failure.

## Where supersession actually belongs

Spectral **already has** correct supersession, in the right layer:

- `graph_store::insert_triple_superseding` / `insert_triple_superseding_by`
- ontology `single_valued` predicates
- `valid_to` / `superseded_by` / `superseded_by_agent` columns
- `Brain::retire_conflicting_objects`, `undo_supersession`

That machinery is typed, auditable and reversible. Its input is
`assert`/`assert_typed`/`ingest_text` entity extraction, not a regex over raw
conversational turns. If knowledge-update is to be improved by supersession, the
lever is *"route more of the corpus through typed assertions"*, not *"pattern-
match free text at read time"*.

Note the honest ceiling on that too: knowledge-update session-recall is 99.4%,
so nothing here is a retrieval problem. The 12.8pp gap to 87.2% accuracy is
actor-side, consistent with everything else measured this session.

## What is kept

`spectral::supersession` stays, `enabled: false`, documented as rejected.
Kept because the partition machinery is correct and reusable if a better topic
source appears (typed triples being the obvious one), and because its tests pin
non-obvious safety properties a future attempt would need anyway: read-time
only, never deletion; undated memories never suppress dated ones; order
independence; same-session restatement treated as elaboration.

`supersession_coverage.rs` is kept as the cheap gate — any future extraction
proposal should be run through it *before* any oracle time is spent, because it
answers "does this fire, and on what?" in seconds rather than in a 20-minute
paired run.
