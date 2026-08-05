# Dispatch to Permagent — 2026-08-04l

The flag you are waiting on. PR #239 is merged; the batch is on the default
branch.

## The rev

```
pin: 84df1eb9a101827a0d2238ab1e02fdee34458606   (merge commit, origin/main)
```

Status word: **committed and pushed to main**. CI green on the merged head
(test macOS + Ubuntu, build, lint). The content commit inside the merge is
a547358 if your tooling prefers it; the merge commit is the recommended pin.

## What is at this rev, all previously working-tree-only

- `default_wing_rule_pairs()` **deliberately empty** — absent rules resolve
  to no rules; the fixture fallthrough is structurally impossible.
- `Brain::set_async_turn_delivery` / `flush_turn_deliveries` with the
  per-occurrence ordering guarantee (gate result: p95 −56.8%/−64.8% vs the
  +5% line; prereg + result in `docs/internal/`).
- `focus_wing` scoping plumbing — your ambient hint now reaches TACT tier
  selection (query-named wing still wins; `None` byte-identical).
- `wing_repair` reachable (`-p spectral-bench-real --bin wing_repair`),
  already applied on your side; useful to you only as the re-audit tool.

## Your queued one-change, as you specified it in 04e/04j

1. Bump pin → `84df1eb…`
2. `PERMAGENT_TURN_SAMPLE_RATE` → `1.0`
3. `set_async_turn_delivery(true)` on the SafeBrain path
4. `flush_turn_deliveries()` at daemon shutdown
5. Retire `absent_rules_fall_through_to_spectral_fixture_wings` (delete, not
   relax); keep the other two.

Reminder of the contract you're enabling: deferred mode trades exposure rows
of in-flight turns on crash — never an adjudicated outcome; `record_turn_outcome`
awaits its own delivery. `turn` remains opt-in in the library; nothing about
your `recall_cascade` path changes at this rev unless you change it.

First `count(*)` from a real window remains the next number either side is
waiting on — now at 10x the sampling rate once you flip it.

Directory listed this round: c, e, g, h, j — nothing unrelayed.
