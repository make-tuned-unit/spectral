# Entity-fields API — consumer contract (Permagent People/CRM)

**Status:** load-bearing contract, per Permagent CC dispatch 2026-07-13. Do not
refactor these invariants away without coordinating the People-overlay change
with the Permagent collaborator.

## What it's for (consumer context)

Entity-fields is the typed-attribute store behind Permagent's People/CRM
(their Decision A, #255): the **graph is the single source of truth for person
attributes**. Identity rows live in Permagent's `people` table, but every
attribute (email, phone, role, company, …) is read through from
`entity_fields` at request time and **replaces** any column value — columns are
a legacy safety net, never the response source.

Consumption (via Permagent's SafeBrain wrappers, `spawn_blocking` over
`brain::Brain`):
- `get_entity_fields(entity_id)` per entity; Permagent's own batched
  read-through helper (`entity_fields_for`, **Permagent-side**, not a Spectral
  API) overlays fields onto people rows in one hop per request.
- `set_entity_field(entity_id, field, value, source, source_url)` writes with
  `FieldSource` provenance.
- Their Librarian reads `get_entity_fields` during entity description.

## The invariants (treat as contract)

1. **Manual beats Enriched.** An `Enriched` write MUST NOT overwrite a `Manual`
   value (user-stated facts are sacred; machine enrichment never clobbers
   them). The `bool` return of `set_entity_field` reports whether the write
   took — Permagent surfaces this to users (their Enricher #495 lands approved
   enrichments as `FieldSource::Enriched`, so precedence + the bool are
   user-visible behavior, not just test assertions).
   Covered by their smoke test
   (`spectral_smoke.rs::entity_fields_round_trip_with_provenance`) and our
   ingest tests.
2. **Upsert idempotency per `(entity_id, field)`.** One current value per
   field — not an append log.
3. **Provenance survives reads.** `EntityField.source` (+ `source_url`)
   round-trips; Permagent's Brain UI renders it so users can see why a fact is
   believed.
4. **The read path is hot.** The batched overlay runs on every People/Brain
   request (they measure this hop; their Decision E: measure before caching).
   Keep `get_entity_fields` cheap — **no LLM, no network IO, no heavy work** on
   that path.

## Evolving the store

FieldSource variants, TTL/staleness, per-field visibility etc. are all open —
but flag the change to the Permagent collaborator first so the People overlay
is adjusted in the same motion.
