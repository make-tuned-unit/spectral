# Populating the wings — what the corpus says, and what the 2026 literature is actually good for

Written 2026-08-24. Wings are the topic/scope half of TACT's fingerprint
(`hash(hall, target_hall, wing, time_bucket)`) and the scope the recall path
filters on. **48.8% of the real brain sits in the `general` catch-all** — 1,542
of 3,160 memories — and that number has not moved in a week (49.3% on
2026-08-17). Unlike the hall gap, nobody is working it.

All figures measured against `~/.permagent/brain` today, read-only.

## What is actually in `general`

| shape | n | share |
|---|---:|---:|
| chat turns (`User: … Assistant: …`) | 1,050 | **68%** |
| other | 143 | 9% |
| automation runs | 110 | 7% |
| browser navigation | 104 | 7% |
| task records | 79 | 5% |
| other activity | 35 | 2% |
| notes / documents | 21 | 1% |

So this is not a long tail of exotic content. **It is one dominant class —
conversation — plus ambient noise that R46 will consolidate away.**

## The literature is mostly solving a different problem

The 2026 work on organising memory by topic is overwhelmingly about
**taxonomy induction**: discover a label space from unlabelled text.
Iterative LLM taxonomy induction, LHATM's density-sensitive hierarchy with
LLM split/merge, BERTopic and its successors, EvoTaxo's evolving taxonomies —
all assume the categories are unknown and must be found.

**Ours are known.** The projects exist: 22 distinct project display names
appear in the app's own `Started working in project X` records, and 45 wings
are already in use. Inducing a label space here would not fill the wings; it
would invent a *second*, competing one.

That is not a hypothetical risk. It is precisely what the retired demo wing
fixtures did — `alice`, `apollo`, `acme`, `vega` — capturing real memories by
keyword collision until the rules were emptied on 2026-08-04. An induced
taxonomy is the same failure with better production values.

**The applicable findings, and they are narrower than the volume of work
suggests:**

- **Scope leakage is the named failure mode** — memories from one context
  recalled in another. For a personal brain a *wrong* wing is worse than no
  wing: it poisons recognition ground truth and the TACT gate, and unlike an
  empty wing it is invisible. This independently validates the consumer's
  existing stance ("uninformative beats wrong", why they pass an empty
  rule-set rather than none).
- **Structured metadata beats embeddings for scoping.** Embedding retrieval
  is for fuzzy recall; it drifts, and drift across scopes is exactly scope
  leakage. Scope is a filter, and filters want facts.
- **Decouple encoding from consolidation** — assign provisionally on arrival,
  revisit when evidence accumulates, rather than forcing a decision at write
  time.
- Hybrid routing (keyword + structure + graph + embedding) beats any single
  signal.

## What the corpus makes available, cheapest first

Two signals need no model at all. Measured on the 1,542:

| signal | recovers | how |
|---|---:|---|
| **Lexical**: memory names a known project | **470 (30%)** | match against the 162-term vocabulary built from project display names + wings in use |
| **Temporal**: a project was selected ≤30 min before the write | **277 (18%)** | `Started working in project X` activity records as an "active project" timeline |
| **union** | **601 (39%)** | |
| both agree | 146 | a free precision check |

**Assigning only the 601 would take the corpus from 48.8% `general` to 29.8%
— without a single inference call.**

The residual 941 (61%) is where judgement is genuinely needed, and some of it
is *correctly* general: personal conversation belonging to no project. **The
target is not zero.**

### One approach that looks obvious and is not

Propagating a wing along the episode fails, for a structural reason worth
recording: episodes are created *per wing*
(`find_recent_episode(&wing, …)`), so a `general` memory's episode is a
`general` episode. Measured: of 1,301 `general` memories carrying an
`episode_id`, only **39 (3%)** sit in an episode that also contains a
wing-assigned memory, and **0** of those episodes offer an unambiguous
candidate. The signal is circular by construction.

## Proposal — a ladder, and the rungs are ordered by cost

**Rung 1 — plumbing, not classification (largest single win, zero risk).**
68% of the gap is chat turns, and the app *knows* which project is open when
it writes them. Passing `RememberOpts.wing` on the chat write path is not a
classification problem at all. Everything below only exists to clean up what
this rung fails to capture going forward.

**Rung 2 — deterministic backfill of the existing 601.** The API already
exists and is the safe restricted form:
`Brain::reclassify_wings_in(&["general"], apply)` — scoped to the catch-all,
so it *cannot* touch a genuine wing, with `apply: false` as a dry run. The
consumer supplies `wing_rules` built from its own project registry. Precision
first: require the project name, never a fuzzy stem. Note `wing_rules` is
replace-not-merge, the same trap as `hall_rules`.

**Rung 3 — the Librarian, gated exactly as HALL is.** It already reads every
memory. Have it emit `WING:` **from the closed list of known projects, or
`none`** — never a new name. Same gate as HALL emission: measure accuracy
against ground truth and self-consistency before shipping. The 7b agreed with
itself 51% on halls; if wings are no better, this rung does not ship.

**Rung 4 — embedding clustering / topic induction: not recommended.** Wrong
problem (the labels are known), wrong failure mode (scope leakage into a
personal brain), and it re-runs the fixture mistake with a better model.

## Falsification

If rungs 1–2 land and the corpus `general` share does not fall to roughly 30%,
the lexical/temporal attribution above is wrong and should be re-measured
before anyone builds rung 3. Re-run: the shape table, the 601 union, and
`brain_audit`'s wing entropy — which should rise from 0.53 as mass leaves the
catch-all.
