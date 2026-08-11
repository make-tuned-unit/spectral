# Track C — the 73% really is classification. My hypothesis was wrong.

**$0. `SPECTRAL_INGEST_PROFILE=1`, LoCoMo ingest, 14,900 memory writes.
No model calls.**

**Read the limitation section before using these numbers.** The run violated its
own stated precondition.

## Result

| stage | ms/event | share |
|---|---:|---:|
| **`ingest_call`** (classify + signal score + hash + episode) | **1.6568** | **85.6%** |
| `sig_write` | 0.1260 | 6.5% |
| `density_write` | 0.1039 | 5.4% |
| `readback` | 0.0337 | 1.7% |
| `sign` (Ed25519) | 0.0139 | 0.7% |
| `density_compute` | 0.0013 | 0.1% |
| TOTAL (measured stages) | 1.9357 | — |

## The hypotheses, and how they did

`ingest-per-event-hypotheses-2026-08-09.md` argued from code reading that the
2026-08-03 decomposition's attribution was "largely wrong", because
`remember_with` performs four extra round trips and an asymmetric-crypto
operation *after* the call containing classify/score/hash.

- **H1 — signing dominates: REFUTED.** Ed25519 signing is **0.7%**; with its
  write, 7.2%. The claim that "the auditability edge shows up as a throughput
  cost" is **false**. Crypto is nearly free here.
- **H2 — the density UPDATE is a meaningful second write: weakly supported.**
  Real, and only 5.4%.
- **H3 — the read-back is avoidable: true but negligible.** 1.7%.
- **H4 — it really is classify/score/hash: CONFIRMED at 85.6%.**

**The 2026-08-03 decomposition called this a black box and guessed its contents
correctly. I read the code, found the round trips, and inferred the guess was
wrong. The measurement says the guess was right and the inference was wrong.**

Removing *every* extra round trip — folding density into the insert, returning
the hash from `ingest`, batching the signature write — buys **~13%**, not the
majority the hypotheses predicted. **The ingest gap is inside the classifier and
scorer, where the original decomposition said it was.**

## A finding that does survive

**`session_assoc` never fired — zero calls across 14,900 writes.** The bench
sets `episode_id` but not `session_id` in `RememberOpts`, so
`associate_memory_session` is never exercised by *any* benchmark we have run.
That is a **coverage blind spot**, not a cost: a production write path that
consumers do use has never appeared in a single measurement here.

## Limitation — the run violated its own precondition

The hypotheses doc stated this "**must run on an otherwise-idle machine**".
It ran on a volume at **99% capacity with ~14GB of active swap**. CPU was idle
(no competing arms), but **four of six stages are I/O-bound** and inflate under
that pressure.

Consequently:

- **The absolute numbers are unreliable.** Total measured is **1.94 ms/event**
  against the 2026-08-03 decomposition's **0.233 ms/event** for the same work —
  an **8× discrepancy this run cannot reconcile.**
- **The shares are more robust**, and the 85.6%-vs-7.2% gap is wide enough that
  the qualitative verdict (H4 confirmed, H1 refuted) survives plausible
  distortion.
- **These numbers are NOT entered in `MEASURED_RECORD.md` as measured.** A clean
  re-run on a healthy machine is required first. `scripts/run_ingest_profile.sh`
  reproduces it.

## What this changes

The "we lose the systems axes" claim now has a *located* cause rather than a
black box: the gap to MinHash+BM25 is **classification and signal scoring**, not
durability, not crypto, not round trips. Whether that is worth optimising is a
separate question — ingest throughput does not bind Permagent, who operate where
428 and 3,148 ev/s are indistinguishable.

**Refs:** `ingest-per-event-hypotheses-2026-08-09.md` (the refuted hypotheses),
`ingest-gap-decomposition-2026-08-03.md` (vindicated),
`phase0-rerun-2026-08-03.md`.
