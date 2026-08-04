# Prereg — R8: deferrable turn-delivery write (2026-08-04)

Committed BEFORE implementation and measurement. Register row: R8.
Trigger: Permagent's dispatch response (2026-08-04) — they shipped `turn`
sampled+shadowed and will go to sample rate 1.0 and make it primary **iff**
this prereg's gate passes. The failed gate being repaired:
recall-only p95 +87–100% vs a +5% kill line, diagnosed as the synchronous
`record_turn_delivery` transaction commit on the read path
(`turn-latency-gate-2026-07-31.md`).

## Design (decided before measuring)

Mirror the existing `async_writeback` precedent (opt-in, default off, spawned
on the Brain's runtime), with one addition it doesn't need: **per-occurrence
ordering**.

- `Brain::set_async_turn_delivery(on: bool)` — default **off**; the off path
  must be byte-identical to today (no code motion on the sync path).
- When on, `record_turn_delivery` spawns the store write on the runtime and
  stashes the `JoinHandle` in a pending map keyed by `occurrence_id`
  (pruning completed entries as it goes).
- `commit_turn_outcomes` first takes and awaits any pending handle for ITS
  occurrence, surfacing a failed delivery write as a commit error. This is
  mandatory, not an optimization: measured store behaviour shows an outcome
  commit racing ahead of its delivery UPDATEs zero `turn_members` rows and
  **silently drops every outcome** while the delivery later lands as
  all-'unreported'. Fire-and-forget without ordering would corrupt the corpus
  Permagent is collecting.
- `Brain::flush_turn_deliveries()` — awaits all pending writes; for shutdown.

## Durability contract (the trade, stated upfront)

- What is traded: exposure rows of turns still in flight if the process dies
  before the spawned write lands (bounded by in-flight count; there is no
  batching window).
- What is NOT traded: an adjudicated outcome can never be lost, misordered,
  or applied to a missing delivery — the commit awaits its own delivery
  first. Callers needing every exposure durable before proceeding leave the
  mode off; that is why it is opt-in and off by default.

## Correctness gates (tests, must pass BEFORE any latency measurement)

1. Deferred mode: `turn` → immediate `record_turn_outcome` commits ALL
   outcomes (the race is closed; no silent 0-row update).
2. Off mode: behaviour unchanged; existing turn tests stay green untouched.
3. `flush_turn_deliveries` drains: after flush, `turn_events`/`turn_members`
   contain every delivery.

## Measurement gate (decides R8 and Permagent going primary)

`cargo run -p spectral --release --example turn_latency` with a new arm
"turn (uncommitted, deferred delivery)":

- **PASS** iff deferred-arm recall-only p95 ≤ legacy `recall_cascade_scoped`
  p95 × **1.05**, warm, two consecutive runs, BOTH runs individually under
  the line. p50 and the sync arm reported alongside for continuity.
- FAIL → `turn` remains non-default, mode ships disabled or not at all, and
  Permagent is told plainly. No optimising-until-it-passes beyond this
  design: if THIS mechanism misses the line, that is the result.
