# Which Spectral capabilities is the production brain actually feeding?

Measured 2026-08-17 on the real Permagent brain (read-only, statistics only),
n = 2,807 memories. Reproduce:

```
cargo run -p spectral --example brain_audit -- ~/.permagent/brain
```

Spectral offers six kinds of memory, but each needs particular data to be
present **and varied**. A field that is null, or constant, silently disables the
engine that reads it — no error, no warning, just a capability that quietly does
nothing. That is the failure mode this audit exists to make visible.

## Findings, ordered by impact

| capability | substrate | verdict |
|---|---|---|
| **Recognition** | 1,214 of 2,807 enrolled (**43.2%**) | **DEGRADED** — 57% of the brain is invisible to it |
| **Relational graph** | 136 entities, **9 triples** (0.07 edges/entity) | **INERT** — nothing to traverse |
| **Integrity** | content_hash 61.1%, **signature 39.0%** | **DEGRADED** — 61% unsigned |
| **Recall (TACT hall)** | 9 values, normalised entropy **0.39**, `event` at 77.7% | **DEGRADED** — near-constant routing key |
| Recall (TACT wing) | 47 values, normalised entropy 0.52, `general` at 49.3% | LIVE, with a large catch-all |
| Episodic / temporal | 64.0% have `episode_id`, 403 episodes, 114 distinct days | **LIVE** |
| Adaptive | 152 distinct `signal_score` values, 17.4% still at default | **LIVE** |
| Visibility / federation | 100% `private` | inert — expected for a personal brain |

## 1. Recognition: 57% of the corpus cannot be recognised

The largest and cheapest-to-fix gap. `recognize()` will answer **Novel** for
content the brain demonstrably contains, because the memory was never indexed.

Enrolment happens inside `remember`, but **non-fatally** — a failure logs a
warning and pushes a derivation warning, then carries on
(`brain.rs`, "recognition enroll failed (non-fatal)"). So failures accumulate
silently and nothing aggregates them.

**Fixed on the Spectral side by this change.** `DerivationHealthReport` gained
`missing_recognition_enrollment`, and `is_healthy()` now accounts for it.
Previously the report tracked content hash, declarative density and signature
but *not* enrolment — `repair_derivations`'s own doc admitted the consequence:
"`derivation_health` has no missing-enrolment field, so there is nothing to
diff against". A brain could report healthy with most of itself unrecognisable.

Remediation for Permagent: `repair_derivations()` already re-enrols every
scanned memory unconditionally. One call closes this.

## 2. The knowledge graph is inert

**9 triples across 136 entities.** Maximum entity degree is 1. Two-hop
traversal, spreading activation and `related_memories` have nothing to walk, so
the entire relational capability is decorative on this brain.

Note the shape: 1,683 documents and 3,846 mentions *are* populated. Entity
mentions are being tracked; relations are not being asserted. Triples arrive
only two ways — an explicit `assert`/`assert_typed`, or LLM extraction
(`spectral-graph::extract` is LLM-based; there is no deterministic
text-to-triple path). Neither is running at scale.

This is the single largest *unrealised* capability in the system: the substrate
for entities exists, and the edges that would make it a graph do not.

## 3. Integrity: 61% of memories are unsigned

`content_hash` 61.1%, `signature` 39.0%. Unsigned memories cannot be
authenticated when shared, so any future federation under
`ImportPolicy::RequireSigned` would reject the majority of this brain.
`repair_derivations()` backfills both.

## 4. TACT's hall key is nearly constant

The TACT fingerprint is `hash(hall, target_hall, wing, time_bucket)`. With
`hall = "event"` for 77.7% of memories (normalised entropy 0.39), that component
contributes almost no routing signal and recall leans correspondingly harder on
FTS. `wing` is healthier — 47 values, normalised 0.52 — though `general` at
49.3% means half the corpus sits in a catch-all.

Neither is *broken*; both are lower-resolution than the design assumes. Finer
hall classification is the cheaper of the two to improve.

## What healthy would look like

- recognition enrolment **>= 95%**
- edges per entity **>= 1.0** (currently 0.07)
- signature coverage **>= 95%** (currently 39%)
- hall normalised entropy **>= 0.5**, no single value above ~60%

Re-run the audit after any change; it is read-only, $0, and takes seconds.
