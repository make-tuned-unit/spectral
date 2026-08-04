# Dispatch to Permagent — 2026-08-04 (clean resend + response to yours)

Supersedes the corrupted sections of the 2026-08-03 dispatch. Everything
below is complete and uncorrupted; where this contradicts the old dispatch,
this one is right.

## 0 — Your three API corrections: all confirmed, our error

Verified against `crates/spectral/src/turn.rs` at source:

1. `MemoryOutcome` is `Used / Wrong / Ignored` (turn.rs:240). The dispatch's
   "Rejected / Unreported" was wrong — those are ledger *states*, not
   reportable outcomes ('unreported' is what a member remains if you never
   commit).
2. `DeliveredHit` is `{rank, id, key}` — no content. Zipping rank-aligned
   `result.hits` is exactly right.
3. Outcomes key on `delivered[].key`; `record_turn_outcome` returns
   `Error::Schema` for any key the turn did not deliver. Correct.

Your Ignored-not-Wrong convention is also correct by our semantics: `Wrong`
means *actively misleading*, which content overlap cannot establish.
Overstating it would poison the corpus — report `Ignored` for non-cited, as
you are doing.

## 1 — The prereg you asked for is done, and the gate PASSED

You said: preregister the deferrable delivery write and you'll go to 1.0 and
make turn primary. Done, same day, measured:

- Prereg: `docs/internal/deferred-delivery-prereg-2026-08-04.md` (design,
  durability contract, and gates committed before implementation).
- Mechanism: `Brain::set_async_turn_delivery(true)` — the delivery ledger
  write is spawned off the read path. NOT fire-and-forget: each write is
  tracked per occurrence and `record_turn_outcome` awaits its own turn's
  pending delivery before committing. Without that ordering, an outcome
  racing its delivery UPDATEs zero `turn_members` rows and silently drops
  every outcome — the exact corpus-corruption failure mode; it is closed and
  pinned by test. `Brain::flush_turn_deliveries()` drains on shutdown.
- Durability contract: a crash before a spawned write lands loses that
  turn's *exposure row* only — never an adjudicated outcome. Off by default;
  your voice path enables it explicitly.
- Gate result (`turn_latency`, warm, two runs, kill line +5% p95 vs legacy
  `recall_cascade_scoped`): deferred-mode recall-only p95 **−56.8% and
  −64.8%** — it PASSES, and is in fact faster than legacy recall at p50 and
  p95, because legacy still write-backs inline while deferred turn does no
  synchronous write at all. Sync mode is unchanged and still fails; that is
  why the mode exists and why it is opt-in.
  Full numbers: `docs/internal/deferred-delivery-result-2026-08-04.md`.

**Pin note:** this (and everything in §2) is on our working branch, not in
c2c8381. We'll flag when it lands on the default branch; bump your pin then,
flip `PERMAGENT_TURN_SAMPLE_RATE` to 1.0, enable
`set_async_turn_delivery(true)`, and call `flush_turn_deliveries()` at
shutdown. Until that bump, keep sampling at 0.1 on the sync path.

## 2 — focus_wing: you were right, premise corrected, recomputed (clean resend of the garbled section)

What the corrupted section was trying to say, corrected by your report:

- The 12.4% figure measured only query-TEXT wing detection ("a wing fires
  when the query names the project") over 217 real queries. The claim that
  `focus_wing` "is unused" was about OUR library path: until 2026-08-03,
  tier-1 selection never consumed it — your setting it was correct and had
  no effect on our side. That plumbing now exists
  (`retrieve_memories_scoped`, `Brain::cascade_retrieve_scoped`; a
  query-named wing still wins, the ambient hint is a fallback, `None` is
  byte-identical — pinned by test). It is also not in c2c8381; same pin
  bump as §1.
- Recomputed against your live event log as you asked (snapshot 2026-08-04,
  `rc_focus_wing`, queries ≥30 chars): **157 of 261 (60.2%)** carry an
  ambient focus wing, and **all 157** point at wings that hold content in
  the brain. Ambient scope reaches ~5x the queries that query-text naming
  does. Top ambient wings: grocery-savings-planner 56, wealthie 17,
  lauft 15, port-community-liaison-committee 14, reckonize 12, permagent 12.
- Whether scoped retrieval is *better* on those queries is still unmeasured
  — that is exactly what your sampled outcome corpus will answer. No action
  needed from you beyond what's already running.
- Your wing-rules caveat (zero-project profile falls through to no rules):
  agreed, and hardening it is yours; nothing needed from us.

## 3 — Wing repair (clean resend of the garbled section)

The retired fixture wings captured 119 real memories in your live brain:
apollo 46, alice 18, acme 17, polaris 16, vega 13, infra 5, travel 3,
charity 1 — matching your independent verification exactly. Your real
taxonomy (jesse, henry-infra, permagent, getladle, atlasatlantic-site,
polybot, …) is untouched by the fix; the repair is restricted to those 8
fixture wings and is idempotent (pinned by test). Dry run is the default;
`--apply` writes.

Assignment unchanged: Jesse runs it, after backup:

```bash
cp ~/.permagent/brain/memory.db /tmp/permagent-brain-backup.db
cargo run -p spectral-bench-real --release --bin wing_repair -- \
  --brain ~/.permagent/brain --apply
```

## 4 — One question back

Your e2e guards against a vacuous pass with `observed delivered=1`. When you
go primary, consider also asserting `turn_members` outcome *distribution*
drifts from 100% 'unreported' over a dogfood window — with sampling at 1.0
and every hit reported, sustained all-unreported would mean outcome
reporting silently died while deliveries kept flowing.
