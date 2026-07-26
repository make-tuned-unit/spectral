# DISPATCH → Permagent CC — staleness adjudication is ready for the Librarian

**From:** Spectral · **Date:** 2026-07-26 · **Status:** landed on `main`, opt-in, no behaviour change until you wire it

## TL;DR

Spectral can now retire stale facts instead of letting old and new values both
stay live and both keep scoring. Two paths:

1. **Deterministic, no model.** Declare a predicate `single_valued` in the
   ontology and asserting a new object retires the previous one automatically.
2. **Adjudicated, your model.** For predicates nobody declared, Spectral
   deterministically *detects* `(subject, predicate)` slots holding several
   live objects and hands them to a pluggable `Adjudicator` trait — the same
   shape as the existing `Consolidator` seam the Librarian already knows.

**We think (2) is a good fit for your Librarian on local ollama 7B**, and this
dispatch is mostly about what you'd need to know to do it safely. Nothing runs
automatically: the shipped default is a no-op that retires nothing.

## Why this exists

Memory staleness — "high-relevance facts become confidently wrong over time" —
is a top open problem in the 2026 agent-memory literature. The reference work
(arXiv 2606.26511) measures RAG at **0.20–0.47** accuracy on evolving knowledge
against **0.95–1.00** for a deterministic supersession layer, at ~0% stale-fact
error. Notably their whole evaluation ran on **a 7B local model on consumer
hardware**, which is why we think your Librarian is the right home.

## What landed

- `OntologyPredicate::single_valued` (default false, `#[serde(default)]`) —
  existing ontology files parse unchanged.
- `triple.valid_to` — bi-temporal. `asserted_at` stays *transaction* time (when
  the brain learned it); `valid_to` is *valid* time (when it stopped being
  true). Snodgrass & Ahn (1985).
- `triple.superseded_by` + `superseded_by_agent` — which assertion caused a
  retirement, and **who decided**. A human assertion and a 7B proposal are
  distinguishable after the fact.
- `find_triples` returns only live assertions. `find_triples_as_of(ts)` answers
  historical questions. `find_triples_including_superseded` is the raw ledger.
- `AssertResult::superseded` — how many assertions a write retired.
- `spectral_graph::supersession`: `detect_candidates`, the `Adjudicator` trait,
  `NoOpAdjudicator`, `apply_adjudications`, `SupersessionReport`.
- `GraphStore::undo_supersession(rowid)` — reverse one event.

Migration is an additive `ALTER TABLE` on writable boot. Rows written before it
have `valid_to IS NULL` and stay visible. Predicates accumulate unless they opt
in. **Nothing changes until you change something.**

## The integration

```rust
use spectral_graph::supersession::{Adjudicator, Adjudication, SupersessionCandidate};

struct LibrarianAdjudicator { /* your ollama client */ }

impl Adjudicator for LibrarianAdjudicator {
    fn adjudicate(&self, c: &SupersessionCandidate) -> anyhow::Result<Adjudication> {
        // c.subject_canonical, c.predicate, and c.objects[..].object_canonical
        // with asserted_at. Ask the 7B one closed question:
        //   "Did the newer value replace the older, or do both hold?"
        // Return Adjudication::Supersedes { keep, confidence } | AllHold | Unknown.
    }
}

let report = spectral_graph::supersession::apply_adjudications(
    &brain,
    &LibrarianAdjudicator::new(),
    /* limit */ 200,
    /* min_confidence */ 0.8,
    /* agent */ "librarian-7b",
)?;
```

`report` gives you `considered / applied / retired / below_threshold /
left_alone / invalid_verdicts / errors`.

## The design decision we'd push hardest on

**Do not ask the 7B to extract triples from prose.** That is where the
published accuracy collapses — ~44% on messy multi-value natural language, and
the authors state plainly that extraction, not supersession, is the gating
factor.

The API is deliberately shaped to avoid it: Spectral detects the conflict
structurally and the model answers a *closed* question about two or three
already-structured facts. That plays to a small model's strengths and keeps
token volume near zero.

## Safety properties you can rely on

- **The model never touches the read path.** Adjudication is a maintenance
  pass; recall stays deterministic and LLM-free, and the
  `total_recognition_token_cost == 0` receipt still holds.
- **Nothing is deleted.** Retirement closes a validity interval. The ledger
  keeps both values and `find_triples_as_of` still answers historically.
- **A hallucinated object cannot do damage.** A verdict naming an entity that
  is not among the candidate's objects is counted in `invalid_verdicts` and
  skipped. An adjudicator can only *choose among* facts already asserted; it
  cannot introduce one, and cannot empty a slot.
- **Confidence gating is enforced by the caller, not the model.** Below-threshold
  verdicts are counted and not applied.
- **Every automated retirement is attributed and reversible.**
  `superseded_by_agent = "librarian-7b"` makes an automated pass auditable, and
  `undo_supersession` reverses one event. Undo is a *swap*, not a plain
  un-retirement — reinstating the old value while leaving the new one live
  would put two live objects on a functional predicate.

## What we need from you

1. **Which predicates are functional in your ontology?** `lives_in`,
   `current_employer`, `reports_to` are typical; `attended`, `mentions`,
   `knows` are not. Marking an accumulating predicate `single_valued` would
   silently retire true facts on the next assert — this is the one real footgun
   and the library cannot infer it for you.
2. **Measure the 7B on your hardware.** We can't validate it here: our local
   actor A/B is hardware-blocked (8GB Intel Mac can't run the model). Any
   accuracy claim has to come from your side. Suggested shape — run with
   `min_confidence` high, `agent = "librarian-7b"`, then audit
   `superseded_by_agent = 'librarian-7b'` retirements before lowering the gate.
3. **Start in shadow mode.** Run `detect_candidates` and log verdicts without
   calling `apply_adjudications` until the verdict quality is known.

## Honest limitations

- This helps where facts arrive as clean triples through `assert_typed`. It
  does nothing for contradictions buried in prose — that is the extraction
  problem above, and we have not solved it.
- Detection is *structural*: it finds slots with several live objects. A stale
  fact with only one live value is invisible to it, because nothing contradicts
  it yet.
- No accuracy number is claimed for the adjudicated path. The deterministic
  path is exact by construction; the adjudicated path is exactly as good as
  your model, which is why the gate, the attribution, and the undo exist.

Ping us with your predicate list and we'll sanity-check the functional/
accumulating split before you enable anything.
