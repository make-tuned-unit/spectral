# Permagent Librarian ↔ Spectral layered store — integration spec

**Status:** design input for the Permagent collaborator ("what the Librarian
builds"). Spectral side already shipped on `feat/recall-recency-hardening-and-levers`.

## Why this exists (and what we already learned)

Real-corpus measurement this arc showed retrieval recall@40 is near-ceiling
(98–100% answer-session recall); the LongMemEval gap is **actor synthesis**,
dominated by **multi-session counting** (the actor mis-aggregates the same
real-world item across sessions). Two things are now measured, not assumed:

- **A cheap read-time LLM consolidation pre-pass HURTS: −9.2 pp** (80.0%→70.8% on
  25 held-out multi-session questions). The weak model is a *lossy intermediate*
  the strong actor over-trusts; its dropped/mis-merged items propagate.
  → So consolidation must be **high-quality and offline (write-time)**, and the
  actor must always be able to **verify against the raw sources**.
- The **deterministic** layered structure works and is the right shape: recall
  surfaces a compact atom; `recall_with_provenance` attaches the ground-truth
  sources on demand. No lossy intermediate; fully auditable.

This spec is the one remaining, most-promising form: the Librarian produces
**durable, high-quality write-time atoms** with a strong model, offline, gated to
recurring clusters — consumed through Spectral's deterministic provenance recall.

## The loop

```
                Spectral (deterministic, $0)                    Librarian (LLM, offline, sparse)
  writes ──▶ recognition recurrence + co-retrieval  ──▶ consolidation_candidates()  ──▶ pick clusters
                                                                                          │
  recall_with_provenance() ◀── consolidate_as(sources,target,tier,atom) ◀── strong-model atom + verify
      (atom + its sources)
```

1. **Spectral surfaces candidates.** `Brain::consolidation_candidates(min_co_count, scan_limit)`
   returns recurring clusters (ambient co-retrieval + recognition recurrence gate
   them, so only *already-recurring* groups appear). This is the deterministic
   selector — the Librarian never scans the whole store.
2. **Librarian summarizes each cluster, offline, with a strong model.** Fetch the
   members' contents (`get_memory` per `member_key`), produce ONE high-quality,
   entity-keyed atom. Quality contract below.
3. **Store the atom with provenance.** `Brain::consolidate_as(&member_keys, target_key, tier, &atom)`
   writes a higher-`compaction_tier` memory and links the sources via
   `consolidation_edges` (hiding them from ordinary recall while keeping them
   reachable). Or `consolidate_with(..., summarize_closure)` to inline the call.
4. **Actor consumes via provenance recall.** `Brain::recall_with_provenance(query, cfg, vis, max_sources)`
   returns each hit as `LayeredHit { hit, sources }` — the atom plus the exact
   raw turns it distilled. The actor uses the atom as a **hint** and verifies the
   count against `sources`.

## Spectral API (already shipped)

| Call | Purpose |
|---|---|
| `consolidation_candidates(min_co_count, scan_limit) -> Vec<ConsolidationCandidate>` | recurring clusters to abstract (`member_keys`, `cohesion`, `signal`) |
| `get_memory(id)` / key→id is `blake3(key)[..8]` hex | fetch source contents |
| `consolidate_as(sources, target, tier, content)` | store a pre-computed Librarian atom + provenance |
| `consolidate_with(sources, target, tier, summarize)` | inline-closure variant (LLM or extractive) |
| `consolidate_extractive(sources, target, tier)` | `$0` deterministic fallback (longest source) |
| `recall_with_provenance(query, cfg, vis, max_sources) -> Vec<LayeredHit>` | atom + drill-down sources |
| `RememberResult::recurrence` (write time) | recognition re-encounter signal, complementary candidate source |

Also on the umbrella `spectral::Brain`.

## Atom quality contract (the −9pp lesson, encoded)

The Librarian atom must be **additive and lossless-adjacent**, never an
authoritative replacement:
1. **Entity-keyed, dedup-correct.** One line per distinct real-world item, keyed
   by its most distinctive identifier (name/couple/project/object). Merge
   cross-session mentions of the SAME item; never split one item across sessions.
2. **Inclusion-strict.** Only items the user actually did/attended/owns — exclude
   hypotheticals, planned-not-done, assistant suggestions.
3. **Cite sources.** Each atom line references the session/turn keys it came from
   (the atom text should name them), so the actor can cross-check.
4. **Prefer omission-safe wording.** If unsure whether two mentions are the same
   item, say so in the atom rather than silently merging or dropping — the actor
   still has the raw sources to adjudicate.
5. **Strong model, not cheap.** Use the same tier as the actor (or better) for the
   atom; the failure mode was a weak model's errors propagating. This is
   affordable because atoms are **durable** (written once, reused across many
   queries) and **gated** (only recurring clusters), so per-query cost ≈ 0.

The actor prompt (Permagent side) must state: *"The CONSOLIDATED atoms are a
candidate set to verify, not ground truth; confirm each against the raw sessions
before counting; add items the atoms missed."* (Do NOT let the atom be
authoritative — that is exactly what regressed in the read-time A/B.)

## Cost model — why this stays cheap

- **Gated:** only `consolidation_candidates` clusters (recurring, high-value) get
  an atom — not every memory.
- **Offline & batched:** the Librarian runs async (a maintenance sweep), not on
  the recall hot path. No latency added to recall.
- **Durable:** one atom serves unbounded future recalls. Amortized LLM cost per
  query → ~0.
- **Deterministic fallback:** with no Librarian, `consolidate_extractive` still
  gives layered recall at `$0` — the LLM is pure optional lift.

## Build split

| Piece | Permagent (Librarian) | Spectral |
|---|---|---|
| candidate discovery | call `consolidation_candidates` | ✅ shipped |
| atom generation | strong-model prompt (quality contract) | — |
| store atom + provenance | call `consolidate_as` | ✅ shipped |
| layered recall for actor | call `recall_with_provenance`; format atom+sources | ✅ shipped |
| actor prompt | "atoms are hints, verify vs sources" | ✅ counting-prompt patterns (this arc) |
| scheduling | when/how often to sweep | (reuse "rebuild if stale" idea) |

## Measurement plan (prove it helps, unlike the read-time version)

Paired on held-out multi-session counting questions, sonnet actor+judge:
- **Arm A:** flat context (current best; 80% on the held-out 25).
- **Arm B:** Librarian pre-populates atoms offline → actor context via
  `recall_with_provenance` (atom + sources), with the "atoms are hints" prompt.
Gate default-ON only if Arm B > Arm A with no regressions. The read-time cheap
variant was −9.2 pp; the hypothesis is the write-time strong-model + provenance +
hint-not-authoritative framing flips the sign. Reuse the eval harness
(`eval-heldout` brains, `heldout_ms.json`) already set up.
