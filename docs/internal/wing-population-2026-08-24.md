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

## Root cause: one writer, and the signal it already has in hand

Grouping the catch-all by key prefix names the culprit exactly:

| writer | in `general` | total | unwinged |
|---|---:|---:|---:|
| **`chat-<session>`** | **1,002** | 1,049 | **96%** |
| `activity:…` | 249 | 871 | 29% |
| `task_<uuid>` | 81 | 259 | 31% |
| `http_http_henry-<id>` | 52 | 68 | 76% |
| `note:…` | 21 | 21 | 100% |
| `reader:…` | 9 | 9 | 100% |

One writer is 65% of the whole problem, and it is not a hard case: it is the
chat turn path, which passes `wing: None` unconditionally
(`goose-server/src/brain_ops.rs::chat_turn_opts`).

**The signal it needs is already maintained in the same process.**
`ActivityIngester` holds `active_project`, which carries a `.wing`, and
`activity/ingestion.rs` reads it as `wing_override` — which is precisely why
activity memories are only 29% unwinged while chat is 96%. The chat path
simply never asks.

### The obvious fix is a trap, and the size of the trap is measurable

Setting `wing: active_project.wing` would wing **96%** of chat turns. It would
also be wrong at scale. Distribution of *how stale* the active project was when
each turn was written:

| age of the active project | turns | |
|---|---:|---:|
| ≤ 30 min | 248 | 24% |
| ≤ 2 h | 104 | 10% |
| ≤ 8 h | 172 | 16% |
| ≤ 24 h | 173 | 16% |
| ≤ 7 d | 115 | 11% |
| **> 7 d stale** | **199** | **19%** |
| no prior selection | 39 | 4% |

**19% of turns would be filed under a project last touched more than a week
earlier.** That is scope leakage — the failure mode the literature names — and
it is strictly worse than the honest `general` we have now, because a wrong
wing is invisible while an empty one is not.

### Pin per session, not per turn

Session scoping cannot be recovered after the fact: **0** of the
`project_selected` events carry a session id, so no chat turn can be joined to
one. But the shape is right, and it matches R45's session-is-the-episode rule.
Measured per session (157 sessions, 1,002 turns), using the active project at
*session start*:

| project age at session start | sessions | turns |
|---|---:|---:|
| ≤ 30 min | 22 | 277 (28%) |
| ≤ 2 h | 8 | 57 (6%) |
| ≤ 8 h | 17 | 202 (20%) |
| ≤ 24 h | 34 | 127 (13%) |
| **> 24 h stale** | 61 | 303 (30%) |
| none | 15 | 36 (4%) |

So a per-session pin with a 2-hour bound wings **33%** of chat turns
confidently and leaves the stale 30% honestly `general` — which is the right
answer for them.

## The durable fix: capture the project, do not infer it

Every option above is a heuristic reconstructing something the application
knew and discarded. The UI knows which project is open when a chat session
begins. **Persist it on the session and let every turn inherit it**, and the
staleness question disappears: no time constant, no model, no leakage, and it
holds for turns written hours into a long conversation.

Concretely, in order:

1. **Capture at session creation** — record the active project on the chat
   session, pass it as `RememberOpts.wing` for every turn of that session.
   This is the "always classified" fix; everything else is cleanup.
2. **Tag `project_selected` events with their session** so the association is
   reconstructable at all — today it is not, which is why the analysis above
   had to fall back on wall-clock proximity.
3. Until (1) ships, a per-session pin bounded at 2 hours: 33% coverage,
   conservative by design.
4. Note the smaller writers are 100% unwinged and are *not* hard: `note:` and
   `reader:` memories belong to a project the app already knows at write time.
   They are 30 memories, but they are pure plumbing gaps.

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

---

# Addendum — classification research, and why it stops before the algorithm

Asked to look for a novel classification approach. The literature offers two
that fit the shape of this problem well, a prototype was built and measured,
and the measurement found something that makes the algorithm question
premature.

## What the literature offers that actually fits

- **Programmatic weak supervision** (Snorkel lineage; CPWS and hyper label
  models more recently). Write several *abstaining* labelling functions —
  lexical, path, session hint — and let a label model estimate their accuracies
  from agreement patterns *without ground truth*. Squarely our situation: many
  weak signals, unknown reliabilities.
- **Conformal / selective prediction with calibrated abstention.** Gives a
  distribution-free way to abstain until a confidence threshold is met. Exactly
  right when a wrong answer is expensive and silence is free — which is the
  wing situation, since an empty wing is visible and a wrong one is not.

Both are sound. Neither can be applied yet, for a reason that only showed up in
the data.

## The reframe nobody's literature suggests: we already have labels

1,615 memories carry a real wing — 14 wings with ≥20 examples. That makes this
supervised classification, not taxonomy induction. A multinomial Naive Bayes
over stemmed tokens (pure statistics; no model, no inference runtime, C3-safe)
with an abstention margin was built and measured.

**In-domain it works well:**

| coverage | precision |
|---:|---:|
| 100% | 78% |
| 50% | 92% |
| 30% | **98%** |

Abstention behaves exactly as the selective-prediction literature predicts:
precision climbs steeply as coverage is traded away.

**Across the domain shift to chat turns it collapses:**

| coverage | precision |
|---:|---:|
| 100% | **32%** |
| 50% | 47% |
| 20% | 67% |

Only 47 of the 1,615 labelled memories are chat turns, and chat is 96% of the
gap. So the labels we have are the wrong domain.

**The obvious repair — bootstrapping in-domain pseudo-labels from the
high-precision lexical rule — made it worse, not better:** 32% → **12%** at
full coverage; pseudo-labels alone, 12%. Falsified cleanly.

## Why everything failed: there is no ground truth for chat wings

Inspecting the 47 "labelled" chat turns explains every result above:

```
[permagent]  User: Hi Earl!  Assistant: I'm actually **Permagent**, not Earl…
[permagent]  User: hello     Assistant: Hello! I'm Permagent, your persistent AI assistant…
[permagent]  User: What model are you using?
[permagent]  User: Henry, do you know who I am?
```

These are winged `permagent` because **the assistant says its own name**. Ten
of the 47 are this. They are not about a project at all.

So every candidate label source in the system is the same artefact —
*mention*, not *aboutness*:

| source | what it actually measures |
|---|---|
| existing wings on chat turns | the assistant naming itself, or UI context |
| lexical project mentions | the project being *referred to* |
| session/temporal hint | which project was open, measured at 21% precision |

**They disagree with each other because they are all noisy, and there is no
anchor to say which is right.** The 79% disagreement measured earlier is not
evidence that the temporal signal is bad and the lexical one is good; it is
evidence that at least one is bad and nothing on hand can tell us which.

**This is the finding: the blocker is not the algorithm, it is the absence of
ground truth.** Weak supervision and conformal abstention both need a labelled
calibration set — the label model to anchor its accuracy estimates, conformal
prediction to set its threshold. Without one we cannot distinguish a 90%
classifier from a 30% one, and the measurements above show that difference is
live rather than hypothetical.

## The cheap way through: ask, do not infer harder

There are **157 chat sessions** in the catch-all, not 1,002 problems — the unit
of labelling is the session, not the turn. Labelling every session by hand is
an afternoon; labelling the *informative* ones is minutes:

1. **Seed** — label ~40 sessions. In the app this is one question at the end of
   a session ("was this about X?" with the hint pre-filled), answered while the
   user still remembers, which is a fundamentally better instrument than
   reconstruction after the fact.
2. **Calibrate** — with a labelled set, the abstention threshold becomes a
   measured quantity rather than a guess, and the in-domain 98%-at-30% result
   suggests a usable operating point exists.
3. **Active learning** — the classifier's own margin says which unlabelled
   sessions are most informative. Ask about those, not random ones.
4. **Then** apply weak supervision to combine the lexical, path and hint
   functions, with the labelled set anchoring the label model.

The order matters and it is the opposite of the intuitive one: the classifier
is the *last* step, and it is cheap once the labels exist. Every attempt to
skip step 1 in this document produced a number that could not be trusted.

## Instruments

`/tmp/wingnb.py` and `/tmp/wingboot.py` (prototypes, stdlib only, read-only on
the brain) — Naive Bayes with abstention, in-domain and cross-domain
evaluation, and the bootstrap experiment that falsified itself. Worth keeping
if step 1 ever happens; worth nothing until it does.
