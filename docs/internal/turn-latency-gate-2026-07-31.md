# Turn contract latency gate — FAIL — 2026-07-31

Preregistered systems kill line
(`docs/internal/turn-contract-prereg-2026-07-30.md`, decision rule 3):

> Recall-only p95 may regress by at most 5%; combined recall+recognition p95
> must not exceed today's two sequential calls. If exceeded, keep the APIs typed
> but do **not** fuse execution.

**Result: FAIL.** The rule is honoured — `Brain::turn` is **not** recommended as
the default recall path. The API stays; execution is not fused.

Tool: `crates/spectral/examples/turn_latency.rs`, release, corpus=400,
iters=300, warm, M-series mac.

## Measurement

| arm | p50 (ms) | p95 (ms) |
|---|---:|---:|
| legacy `recall_cascade_scoped` | 0.980 | **1.363** |
| `turn` (uncommitted) | **0.792** | **2.728** |
| `turn` + outcome commit | 1.057 | 5.929 |

Recall-only p95 delta: **+100.1%** and **+101.6%** across two runs. Kill line is
+5%. Not marginal — 20x over.

## Diagnosis: it is not retrieval

**p50 improved** (0.792 vs 0.980, ~-19%). Removing the inline write-back from
the read path made the median *faster*, exactly as predicted. The regression is
entirely in the **tail**, and it comes from the synchronous delivery write:
`record_turn_delivery` opens a transaction and commits 1 event row plus up to
`k` member rows on every turn, on the response-critical path.

Attempted fix, kept because it is strictly better but **did not rescue the
gate**: hoisting the per-member `tx.execute` to a `prepare_cached` statement
reused across the loop. At k=40 that removes 40 statement re-parses per turn and
moved p95 from +104.6% to +100.1% — i.e. statement preparation was not the cost.
The cost is the transaction commit (WAL write / fsync) itself.

## What would have to change

The obvious candidate is to make the delivery write **deferrable**, mirroring the
`async_writeback` path the legacy write-back already has
(`spectral-graph/src/brain.rs`). That moves the commit off the response-critical
path.

It is deliberately **not** done here, for two reasons:

1. It trades durability for latency. A deferred delivery row can be lost on
   crash, which weakens exactly the property the ledger exists to provide —
   that exposure survives the call. That is a design decision, not a tuning
   knob, and it needs its own preregistration and its own measurement.
2. Optimising until a preregistered gate passes is the failure mode the gate
   exists to prevent. The honest outcome of a failed gate is to record the
   failure and stop, not to iterate on the implementation until the number
   cooperates.

## Consequence

- `Brain::turn` remains available and correct. Its semantics — read-only
  retrieval, outcome-gated reinforcement, durable exposure — are unchanged and
  fully tested.
- It must **not** be presented to Permagent as a drop-in replacement for
  `recall_*` on a latency-sensitive path until this gate passes.
- Legacy `recall_*` is untouched and remains the default.
- Any future claim that the turn path is latency-neutral must cite a re-run of
  this gate, not this document.

## Method note

Warm, two runs, otherwise-idle machine — per the rules in
`docs/internal/ingest-cost-profile-2026-07-31.md`. Ratios travel between
machines; absolute numbers do not.
