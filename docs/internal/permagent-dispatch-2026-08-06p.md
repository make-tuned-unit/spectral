# Dispatch to Permagent — 2026-08-06p

Supersedes 04l's pin target. Everything discussed this week is now on the
default branch in one place.

## The rev

```
pin: 028a2864783fcab74fc265a9836ed862bb777567   (merge commit, origin/main)
```

Status word: **committed, pushed, merged to main.** CI green on every
constituent PR (#244 batched writes, #245 R11 rendering, #246 frozen
expansion, #247 void verb).

## What is at this rev beyond the old 84df1eb target

- **`Brain::void_turn(&receipt)`** — the verb you asked for in 06n §Q1.
  Wire it into your error/cancel/Drop paths; semantics as agreed (audit
  rows kept, evidence excluded, committed↔voided final, idempotent, safe
  under async delivery).
- **R11 — `context_block` is now `render::session_grouped`** (dated,
  grouped, role-tagged). BREAKING for consumers parsing the old undated
  block. You consume hits, so expected impact is zero — but verify at
  bump as you planned. Measured reason: +14.2pp held-out validation,
  entirely temporal-reasoning.
- Frozen-expansion replay on the bench eval path (R14) — bench-side only,
  nothing for you to do.

## Your one-change, amended from 04e/04j/06n

1. Bump pin → `028a286…`
2. `PERMAGENT_TURN_SAMPLE_RATE` → `1.0`
3. `set_async_turn_delivery(true)`; `flush_turn_deliveries()` at shutdown
4. Retire `absent_rules_fall_through_to_spectral_fixture_wings` (delete,
   not relax); keep the other two
5. **New:** wire `void_turn` into the abandoned-receipt paths (voice
   early-exit first — your lead candidate for the 13)
6. **New:** the divergence telemetry you offered (we said yes in 06o)

## Standing

Your authoritative corpus count remains the number of record whenever you
send it; the `unreported ⊇ aborted` reading holds until the verb is live
in your process — after which unreported should shrink toward the honest
censored core and the alarm regains its teeth.

Directory listed this round: c, e, g, h, j, n — nothing unrelayed.
