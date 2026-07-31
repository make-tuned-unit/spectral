# Ingest cost profile — corrected, warm — 2026-07-31

**This supersedes the ingest cost figures in commit `9ec8422`'s message and in
PR #234's body. Those numbers were inflated ~2.8x and should not be cited.**

Tool: `crates/spectral/examples/ingest_profile.rs`, release, n=400, M-series mac.

## What went wrong

The original figures were taken **immediately after a cold ~8-minute compile**,
as a **single run with no stability check**. Disk contention and thermal state
dominated the measurement.

| quantity | reported 2026-07-30 | actual (warm) | inflation |
|---|---|---|---|
| full `Brain::remember` | 7.66 ms/write | **2.73 ms/write** | 2.8x |
| fingerprint cost | 4.09 ms/write | **1.01 ms/write** | 4.0x |
| store floor | 5.14 ms/write (194 ev/s) | **1.21 ms/write (~824 ev/s)** | 4.2x |
| store, no fingerprints | 1.05 ms/write (949 ev/s) | **0.20 ms/write (~4,900 ev/s)** | 5.2x |

Storage figures were **not** affected (they are not timing-sensitive):
11.6 / 26.4 / 45.5 KB per event for no-fingerprint / store floor / Brain.

## Warm A/B across PR #233

PR #233 ("recency index removes full-wing sort on the write path") claimed
**-11%**. Measured warm, same session, same machine, both commits built and run
back to back:

| layer | pre-#233 (`34ea145`) | post-#233 (`3005186`) | change |
|---|---|---|---|
| store, no fingerprints | 0.203 / 0.192 ms | 0.191 / 0.196 / 0.213 ms | ~flat |
| store floor | 1.214 / 1.249 ms | 1.091 / 1.102 / 1.277 ms | ~-8% |
| fingerprint cost | 1.011 / 1.057 ms | 0.900 / 0.906 / 1.064 ms | ~-7% |
| **full `Brain::remember`** | **2.736 / 2.733 ms** | **2.324 / 2.310 / 2.748 ms** | **~-9%** |

**#233's -11% claim holds.** The runs bracket it. The apparent "3.3x speedup"
seen when comparing post-#233 against the cold pre-#233 baseline was entirely
the measurement artifact above.

## Corrected composition — the headline changed

The 2026-07-30 framing was *"constellation fingerprint generation is 53% of a
write and the dominant ingest cost."* **That is no longer accurate.** Warm, at
`3005186`:

| band | ms/write | share of full remember |
|---|---|---|
| store, no fingerprints | ~0.20 | ~8% |
| fingerprint generation | ~0.90 | ~39% |
| graph-side (density, signing, recurrence, recognition enrollment) | ~1.23 | **~53%** |

The **graph-side band is now the larger share**, not fingerprinting. Two
reasons: #233 reduced the store-side work, and the original cold measurement
disproportionately inflated the fingerprint band (which does the most I/O).

## What survives from the 2026-07-30 analysis

Ratios, which is what the argument rested on:

- Suppressing fingerprint generation is still a large speedup on the store
  layer (~0.20 vs ~1.2 ms/write, ~5-6x).
- Fingerprint rows are still ~57% of store-layer bytes (26.4 -> 11.6 KB/event).
- The constellation-hash selectivity and hall-transition base-rate findings are
  distribution facts, unaffected by timing.

## Method rules going forward

1. **Warm only.** The tool now discards a warm-up pass, but that is not
   sufficient on a loaded machine.
2. **At least two runs.** Disagreement beyond ~15% means the machine is not
   quiet; discard and re-run.
3. **Never measure right after a cold compile.** Build first, then run.
4. **Ratios travel; absolute numbers do not.** Do not compare across machines,
   and do not compare against Phase 0's 43 ev/s figure, which used different
   content and hardware.
