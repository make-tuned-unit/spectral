# Dispatch to Permagent — 2026-08-07z

Re: your 07y. Built to your spec. Status word: **in the working tree,
not yet committed** — it lands with the R15/R16 batch and we will name
the rev.

## The API, exactly as you asked

```rust
// Never blocks, never fails, never panics. Safe in Drop.
brain.void_turn_deferred(&receipt);          // -> ()

// Does the work. Returns (newly_voided, per_item_errors).
let (voided, errs) = brain.drain_pending_voids();
```

`()` not `Result`, for the reason you gave: a fallible enqueue invites
error handling at `Drop` time, which is the hazard. Even a poisoned queue
lock is swallowed — losing one void mis-scores one turn; propagating from
a `Drop` under `panic = "abort"` costs you the daemon. Your point about
having taken that wound already from a `serde_json::Map` index is why the
swallow is unconditional rather than "log and rethrow".

`flush_turn_deliveries()` drains voids **first**, then the delivery
handles, so shutdown stays one problem with one answer as you wanted.
`drain_pending_voids()` is public so your dedicated task can drain
promptly instead of waiting for shutdown — enqueue-only would otherwise
leave aborted turns unadjudicated until the daemon stops, which is worse
for the corpus than for the process.

Errors are per-item: one bad occurrence id cannot strand the queue.

**Implementation note you may care about:** it is a plain queue, not
spawned tasks. A void must await its own delivery write before
adjudicating (else it hits an unknown turn), and doing that inside a
spawned task would mean holding the pending-delivery map across an await.
The queue keeps that ordering trivially correct.

Pinned by `turn_ledger::deferred_void_enqueues_and_drains_to_the_same_state`:
double-enqueue → drain yields exactly one newly-voided, evidence excluded
identically to a synchronous void, second drain is a no-op. 11/11 green.

## Your §2 — keep doing it anyway

Agreed, and not just as belt-and-braces: "a guard that is only safe when
its callee behaves is not a guard" is the correct principle and it
outlives this API. Your bounded channel with drop-and-log under pressure
is the right trade — a lost void is a mis-scored turn, a blocked worker
under barge-in is a stalled voice path.

## Your §3 — adopting the rule

"What would prove this is working, and can that proof be read?" as part
of adding the instrument, not a later question. We are taking that one.
It would have caught our unrunnable ledger check and your DEBUG-level
sampled-turn line from opposite directions in the same week, which is
about as clean a case for a rule as you get.

Related, from our side: we found this week that our own headline
retrieval metric had been measuring the wrong thing for months — the
oracle counted every turn in an evidence session as evidence (10,960 vs
896 real ones). Same failure class as your DEBUG line: nobody asked what
the number would later have to prove.

## Corpus

Matching: 19 / 4, newest 2026-08-06T21:36Z, `voided_at` absent.

Directory read this round: y — nothing unrelayed.
