# G3 — outcome-ledger statistics · SHIPPED, but **not yet usable on real data**

**2026-08-08. $0, offline, no model calls.** Estimators implemented and
unit-tested. The position-bias correction they exist for **cannot be estimated
on the ledger we have**, and this document says how much data it would take.

## The defect being repaired

`sqlite_store.rs::reinforce_memory`:

```sql
UPDATE memories SET signal_score = MIN(signal_score + ?1, 1.0) ...
```

A flat additive nudge. Three compounding defects:

1. **No denominator.** One use out of one delivery earns the same increment as
   ninety out of a hundred.
2. **No exposure correction.** A memory delivered at rank 1 is used more often
   *because it was seen first*. Reinforcing the raw rate teaches the ranker
   that its own top results are good — circular.
3. **Unbounded compounding.** Reinforced → ranked higher → delivered higher →
   used more → reinforced. Rich-get-richer with no saturation.

## What shipped — `spectral_graph::ledger_stats`

All pure functions, deterministic, no I/O. 10 unit tests.

- `wilson_lower_bound(used, deliveries)` — encodes sample size in the score.
  1/1 → **0.207**, 90/100 → **0.825**. Monotone in sample size, never reaches
  1.0, and `deliveries == 0` yields 0.0 because *no* evidence is not weak
  evidence.
- `saturating_volume(used)` = `log10(1 + used)` — caps rich-get-richer. The
  first ten uses are worth more than the next thousand, which a linear term
  gets exactly backwards.
- `exposure_curve(adjudicated)` — empirical use-rate per delivered rank. Ranks
  with no adjudicated deliveries return **`None`, not 0.0**. An unobserved rank
  is undefined; conflating "never observed" with "never used" is the same class
  of error as R15's diluted denominator, and it would bias every correction
  computed from it.
- `position_corrected_rate(...)` — divides observed uses by the uses the
  memory's rank exposure alone predicts. **Returns `None` when the curve has no
  estimate for the ranks in question**, rather than silently falling back to
  the uncorrected rate.
- `ledger_score(...)` — Wilson × lift × saturation, degrading to Wilson alone
  when no lift is computable.
- `deliveries_needed_per_rank(p_high, p_low)` — so "can we do G3 yet?" has a
  number for an answer.

## The blocker, measured

Live Permagent ledger:

| | |
|---|---:|
| adjudicated deliveries (`used` + `ignored`) | **160** |
| of which `used` | **11** |
| ranks observed | 40 |
| **mean adjudicated deliveries per rank** | **4.0** |

Four observations per rank. The per-rank use-rate is 0 at most ranks and 0.25
at the nine ranks where a single use happened. That is not an exposure curve;
it is noise with an index.

**What it would take**, at 95% confidence and 80% power across 40 ranks:

| effect to detect | per rank | total adjudicated | vs today |
|---|---:|---:|---:|
| strong bias (25% vs 10%) | 100 | **4,000** | **25×** |
| moderate (25% vs 15%) | 250 | 10,000 | 62× |
| subtle (20% vs 15%) | 905 | 36,200 | 226× |

So the position-bias correction — the most valuable of the three repairs,
because it is the one that breaks the circularity — needs roughly **25× the
adjudicated volume we currently hold**, and that is for the coarsest effect
worth acting on. `unreported` stands at 600 against 160 adjudicated, so the
nearest source of that data is adjudicating what is already collected, not
collecting more.

## What Wilson alone would change today

Every memory the flat nudge is currently promoting rests on a **single** use:

| memory | used/delivered | raw rate | Wilson |
|---|---:|---:|---:|
| `chat-20260805_455-…` | 1/1 | 1.00 | **0.207** |
| `chat-20260429_4-26` | 1/2 | 0.50 | 0.095 |
| `chat-20260501_10-28` | 1/2 | 0.50 | 0.095 |
| `chat-20260729_13-…` | 1/3 | 0.33 | 0.061 |

The flat nudge gives each of these the same increment a 90/100 memory would
earn. Wilson gives them 0.06–0.21. **This repair is usable now** — it needs no
exposure curve, only the counts we already have — but shipping it changes
default ranking behaviour on the consumer's live brain, so it is a
`NEEDS-PREREG` change and is **not defaulted on** here.

## On the co-retrieval regression hypothesis

`landscape-research-2026-08-07.md` §G3 proposed that the measured co-retrieval
regression (728/744 events returning the same ~40 memories) is "textbook
position bias plus rich-get-richer". **That remains untested.** It predicts a
declining use-rate by rank, which is exactly the curve the data cannot
estimate. Recording it as a live hypothesis with a known test rather than as an
explanation would overstate what is known — the same failure the baseline
document made yesterday by reasoning from a metric that could not carry it.

## Status

- Estimators: **shipped, tested, unused by any default path.**
- Position-bias correction: **blocked on data**, 25× short, with the
  requirement now quantified rather than guessed.
- Wilson-only reinforcement: **implementable today, needs a prereg** — it is a
  default-ranking change on a live consumer brain.
- Rocchio PRF over the ledger (the untested expansion-side lever): **not
  attempted here.** With 11 `used` documents there is no relevance set to
  expand from. It needs the same data.

**Refs:** `landscape-research-2026-08-07.md` §G3,
`sqlite_store.rs::reinforce_memory`, `REPAIR_REGISTER.md`.
