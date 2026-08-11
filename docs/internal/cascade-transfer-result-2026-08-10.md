# R28 — adjacency transfers to `recall_cascade` · **PASS** (2026-08-10)

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
cascade`, R19 turn labels. No model calls, model-free.** Preregistered at
`af11198` before the arms ran.

**This was the run most likely to invalidate the session.** Every other result
was measured on `topk_fts`; Permagent calls `recall_cascade` exclusively.

## Result

| path | baseline | + adjacency | Δ | tokens |
|---|---:|---:|---:|---:|
| `topk_fts` | 59.86% | 78.79% | +18.93pp | 2.50× |
| **`cascade`** (production) | **58.60%** | **76.82%** | **+18.22pp** | **2.27×** |

**331 nonzero pairs, all positive — not one question got worse.** p < 0.0001.
Zero-evidence on cascade: **371 → 143.**

**The lever transfers essentially intact**, at slightly *lower* relative token
cost than on topk.

Two concerns raised in the prereg turned out not to bind: cascade's **episode
diversity** (`max_per_episode`) does not fight in-session expansion enough to
matter, and its **session-grouped formatting** does not make adjacency
redundant.

## Cascade's own baseline, measured for the first time

| | micro | zero-ev | turns | tokens |
|---|---:|---:|---:|---:|
| `topk_fts` k=40 | 59.86% | 357 | 40.0 | 1,974 |
| `cascade` defaults | 58.60% | 371 | 34.2 | **1,500** |

Cascade is **1.26pp worse on recall but 24% cheaper**, retrieving 34.2 turns to
topk's 40 — about **29% more token-efficient per unit of evidence**. That is a
point in favour of the path Permagent already uses, and it had never been
measured at full N on the corrected metric.

It also means the two paths' absolute numbers are **not directly comparable**,
which the prereg anticipated: the transfer question is judged against cascade's
own baseline, not against topk's.

## The comparison this does NOT make

**This is the cost-unmatched number, exactly as the prereg declared.** On topk a
token-matched control was constructible because `--max-results` sets k; on
cascade it is not — k comes from the question-type profile. So **+18.22pp is the
analogue of topk's flattering +18.93pp, not of the +6.73pp that survived
token-matching.**

Given the topk result held under token-matching, cascade would be expected to as
well. **Expectation is not measurement**, and the honest cascade question —
does adjacency beat spending 2.27× another way — needs a `SPECTRAL_CASCADE_K`
sweep. That is a separate prereg and it is **not** done.

## What did not run

**`c_spk`** (speaker attribution on cascade) never started: the disk guard
halted the queue at 4Gi free. It is the least valuable of the three arms — R25
established that speaker attribution is **subsumed by k-raising** (all 68 turns
it recovers are also recovered by k=105) — so its cascade analogue was
confirmatory at best. Recorded as **not run**, not as a null.

## What does NOT follow

- **No accuracy claim, and no default change on any path.** A cascade PASS
  licenses a *proposal* to Permagent, not a flipped default. The prereg
  registered this before the result was known.
- **Retrieval only.** At 2.27× context the reader-dilution question is
  unmeasured and unbudgeted, and it is the only question a consumer cares
  about.
- **Bench-scoped implementation.** `apply_turn_adjacency` parses the harness key
  format; production needs real sequence metadata.
- **Corpus-shaped.** Two-party strictly-alternating dialogue is adjacency's
  ideal case, as LoCoMo was for speaker attribution — which then failed to
  replicate on LongMemEval for a structural reason.

**Refs:** `cascade-transfer-prereg-2026-08-10.md`,
`turn-adjacency-result-2026-08-10.md` (R25),
`k-admission-frontier-result-2026-08-10.md` (R27).
