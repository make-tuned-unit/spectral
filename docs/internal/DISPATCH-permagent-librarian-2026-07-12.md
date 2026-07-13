# Dispatch → Permagent: Librarian layered-atom integration

**Ask in one line:** extend the Librarian to write **durable, high-quality
consolidation atoms** into Spectral's new layered store, so the actor can count
across sessions from a deduplicated candidate set + its provenance — the piece
our data says will move multi-session accuracy.

## Why (the evidence, so you can trust the direction)

We ran the real LongMemEval-S oracle + actor eval this arc. Findings:
- **Retrieval is not the bottleneck.** Answer-session recall@40 is already
  98–100%. Fusion/embedding-style recall levers measured null/negative on real
  data. The gap is **actor synthesis**, dominated by **multi-session counting**
  (the actor mis-aggregates the same real-world item across sessions).
- **A counting-prompt fix helped** (identity-keying + inclusion strictness):
  **+8 pp on held-out**, 0 regressions. Good, but the residual is still
  cross-session dedup.
- **A cheap read-time LLM consolidation pre-pass HURTS: −9.2 pp.** A weak model
  as a lossy intermediate drops/mis-merges items and the actor over-trusts it.
- → The promising form is **write-time, strong-model, durable atoms**, gated to
  recurring clusters, consumed deterministically with provenance attached.

## What Spectral has shipped and ready for you (branch `feat/recall-recency-hardening-and-levers`)

A deterministic, LLM-optional layered store. Three calls you'll use (on
`spectral::Brain`):

1. `consolidation_candidates(min_co_count, scan_limit) -> Vec<ConsolidationCandidate>`
   — we hand you the recurring clusters worth abstracting (gated by recognition
   recurrence + co-retrieval, so you never scan the whole store).
2. `consolidate_as(source_keys, target_key, tier, atom_content)` — you store the
   atom you generated; we link the sources via `consolidation_edges` and tag the
   tier.
3. `recall_with_provenance(query, cfg, vis, max_sources) -> Vec<LayeredHit>` —
   the actor gets the atom **plus its ground-truth source turns** for
   verification.

Full contract + atom quality rules:
`docs/internal/librarian-layered-store-spec-2026-07-12.md`.

## What we need from you

1. **Librarian atom generator.** For each cluster from
   `consolidation_candidates`, one **strong-model** (actor-tier or better),
   **offline** call producing an entity-keyed, dedup-correct, inclusion-strict
   atom that **cites its source keys**. Store via `consolidate_as`. (Quality
   contract in the spec — this is the −9pp lesson: weak/lossy atoms regress.)
2. **Actor prompt update (your side).** The actor must treat atoms as a
   **candidate set to verify**, not ground truth: confirm each against the raw
   sources in `LayeredHit.sources`, and add items the atom missed. This framing
   is what separates the write-time path from the read-time one that regressed.
3. **Scheduling.** Decide when the Librarian sweep runs (async maintenance, off
   the recall hot path). A "consolidate stale clusters" cadence is fine.
4. **A sample of 5–10 atoms** on real Permagent data before the full run, so we
   can sanity-check quality against the contract cheaply.
5. **Interface confirmation.** Tell us if the three calls above cover what the
   Librarian needs, or if you need e.g. cluster *contents* returned inline (today
   you fetch member contents via `get_memory`), an atom-provenance metadata
   field, or a batch entry point. Small additions are easy.

## The measurement we run together (gate for default-ON)

Paired on held-out multi-session counting questions (harness + `heldout_ms.json`
brains already staged on our side):
- **Arm A:** flat context (current best: 80% on the held-out 25).
- **Arm B:** Librarian atoms pre-written → actor context via
  `recall_with_provenance` + the "atoms are hints" prompt.
Ship default-ON only if Arm B > Arm A with no regressions. Hypothesis: strong
write-time atoms + provenance + verify-don't-trust flips the sign the cheap
read-time version couldn't.

## Coordination

- Nothing here blocks you; the Spectral APIs are merged on the branch.
- Two decisions we'd like back first: the **atom cadence/scheduling** and whether
  the **three-call interface** is sufficient (item 5).
- Cheapest next step is item 4 (sample atoms) — low cost, tells us fast whether
  the quality contract is being met before spending on the full A/B.
