# Result — R8: deferrable turn-delivery write (2026-08-04)

Prereg: `deferred-delivery-prereg-2026-08-04.md` (committed before
implementation and measurement).

## Verdict: GATE PASSED — both runs, decisively

`cargo run -p spectral --release --example turn_latency`, warm, two
consecutive runs, corpus=400, iters=300, M1 mini:

| arm | run 1 p50 / p95 (ms) | run 2 p50 / p95 (ms) |
|---|---|---|
| legacy `recall_cascade_scoped` | 0.945 / 1.332 | 1.016 / 1.698 |
| turn, sync delivery (V1) | 0.770 / 5.156 | 0.788 / 3.038 |
| **turn, deferred delivery** | **0.505 / 0.576** | **0.506 / 0.598** |
| turn + outcome commit | 1.058 / 8.769 | 1.074 / 6.657 |

Recall-only p95 delta vs legacy, deferred mode: **−56.8%** (run 1),
**−64.8%** (run 2) against a +5.0% kill line. Both runs individually pass.
The sync arm still fails (+287.1% / +79.0% — the tail is run-to-run noisy,
consistent with the +87–100% measured 2026-07-31), which is why the mode
exists.

Deferred `turn` is *faster* than legacy recall at both p50 and p95. This is
coherent, not surprising: legacy recall performs its auto-reinforce +
event-log write-back inline; deferred `turn` performs **no synchronous write
at all** — it is the read path at its floor, with the ledger write riding the
runtime behind it.

## What shipped

- `Brain::set_async_turn_delivery(bool)` (facade + graph), default **off**;
  the sync path is untouched.
- Per-occurrence ordering: `commit_turn_outcomes` awaits its own pending
  delivery before committing, surfacing a failed delivery write as a commit
  error. The race it closes is silent outcome loss (UPDATE matching zero
  rows) — pinned by test.
- `Brain::flush_turn_deliveries()` for shutdown.
- 3 preregistered correctness gates in
  `crates/spectral/tests/deferred_delivery.rs` — race closed, off-mode
  unchanged, flush drains durably (proven across reopen). All pass; full
  `spectral` + `spectral-graph` suites green.

## Durability contract (as preregistered)

Traded: exposure rows of turns in flight if the process dies before the
spawned write lands. Not traded: adjudicated outcomes — the commit awaits its
own delivery. Callers needing every exposure durable before proceeding leave
the mode off; it is opt-in and off by default.

## Consequence

Permagent's condition for going to sample rate 1.0 and making `turn` primary
("preregister the deferrable delivery write") is met, with the gate passed
rather than merely preregistered. Their integration enables it with
`set_async_turn_delivery(true)` + `flush_turn_deliveries()` on shutdown, once
their pin advances past the merge of this branch.

`turn` remains non-default in the library: the *sync* configuration still
fails the gate, and the mode is opt-in by design.
