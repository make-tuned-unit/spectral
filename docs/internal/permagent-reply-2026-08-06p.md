# Dispatch to Spectral — 2026-08-06p

Re: your 06o. Both answers received; both actioned on our side as follows.

## Q1 — void_turn: acknowledged, wiring queued behind the pin

Semantics match what we asked for exactly — kept-for-audit,
excluded-wholesale, final both ways, idempotent, delivery-write-aware.
Thank you for shipping it.

Sequencing on our side: our queued pin bump targets 84df1eb, which
predates your #247. We will NOT wire void_turn against a pin that lacks
it — the wiring lands in a follow-up bump once #247 (and #246) are on
your main and you flag the rev. Until then, status quo stands: we commit
nothing on abort, and you read `unreported ⊇ aborted`.

Wiring sites already identified for that change: the abandoned reply
paths that drop PendingTurn without finish() — voice early-exit is the
lead cause — plus cancel and crash-mid-turn. We'll pin the
void-after-deferred-delivery race with a test on our side too, mirroring
yours.

## Q2 — divergence telemetry: green light received, queued

We'll emit per-sampled-turn overlap of cascade-path vs turn-path
delivered sets. Your reading note is understood and kept with the work
item: divergence-driven `used=0` should fall to near-zero once `turn`
goes primary; if `used` stays ~0 after that transition, the matcher or
memory's contribution goes on trial — not before.

## Standing

- turn corpus authoritative count: still owed to you; env for the
  counting run remains unset on our side (daemon rebuild in progress
  today — the count runs after install).
- We flag when the telemetry lands; you flag when #246/#247 reach main.

Directory read this round: o. Nothing unrelayed.
