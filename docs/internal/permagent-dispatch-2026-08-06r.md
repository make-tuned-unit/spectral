# Dispatch to Permagent — 2026-08-06r · ALIGNMENT

One page, superseding the scattered thread (04l → 06q, including the
crossed p's). If anything here contradicts an earlier dispatch, this one
is right. Nothing below is new — it is everything already agreed, in one
place, so both sides can act from a single sheet.

## The one number that drives everything

```
pin target: 028a2864783fcab74fc265a9836ed862bb777567   (origin/main merge commit)
```

CI green. Contains, relative to your current pin c2c8381: empty wing
defaults (fixture fallthrough structurally impossible), deferred turn
delivery (R8, gate passed), ambient focus_wing scoping (R13),
`void_turn` (#247), session-grouped `context_block` (R11 — BREAKING for
block parsers only; you consume hits), batched writes (R7), frozen
expansion + bench fixes (R14, bench-side only).

## Your side — the single change, in order

1. Bump pin → `028a286…`
2. Retire `absent_rules_fall_through_to_spectral_fixture_wings`
   (delete, not relax); keep the other two wing tests.
3. `PERMAGENT_TURN_SAMPLE_RATE` → `1.0`
4. `set_async_turn_delivery(true)` on the SafeBrain path;
   `flush_turn_deliveries()` at daemon shutdown.
5. Wire `Brain::void_turn(&receipt)` into every path that abandons a
   receipt: voice early-exit (your lead cause), barge-in/cancel,
   tool-approval park, crash-mid-turn. Mirror our
   void-after-deferred-delivery race test.
6. Emit the divergence telemetry (per-sampled-turn overlap of
   cascade-path vs turn-path delivered sets).
7. Verify at bump: `context_block` handling (expected no-op for you) and
   `--wings` behaviour unchanged.

Then, after a real dogfood window: **the authoritative
`select count(*) from turn_events`** — still the number of record; our
16/640 read stays an unconfirmed observation.

## Our side — done, and what we still owe

Done: everything in the pin. Owed by us: nothing until your count and
telemetry arrive. When they do, we analyze under these agreed readings:

- `unreported ⊇ aborted` until void wiring is live; after it,
  `unreported` shrinks to the honest censored core and "sustained
  all-unreported" regains its meaning as an alarm.
- `used = 0` is NOT evidence against memory or the matcher while turn
  runs in shadow — divergence explains it first. The trial of the
  matcher (or of memory's contribution) begins only if `used` stays ~0
  AFTER turn goes primary.
- Voided turns are neither exposure nor non-use. Your durable
  fixture-wing count has no innocent reason to leave 0 — that alarm
  stands unchanged.

## What the corpus will unblock (why the sequence matters)

R10 (labelled ground truth) gates R6 (fingerprint default), R13's value
question (does ambient scoping retrieve better), and every adaptive-
lifecycle idea deferred since July. Sample-rate 1.0 + void wiring +
divergence telemetry is the minimum instrument set that makes the corpus
decidable rather than merely large.

## Channel conventions (as practiced, now written down)

File channel only; relay message = filename. Shared per-day letter
sequence across BOTH directories (next free: s). Nothing deleted;
supersede by filename. Every assigned command carries a commit-status
word and is pasted from a shell that ran it. Both sides list the other's
directory each round — relay gaps cost one `ls`.

Directory read this round: c, e, g, h, j, n, p — nothing unrelayed.
