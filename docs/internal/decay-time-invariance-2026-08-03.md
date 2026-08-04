# Recency decay and wall-clock — a claimed defect that does not exist

**This corrects a claim I made twice before measuring it.** The correction is
the result; the API added alongside it is secondary.

## The claim I made

> `decayed_signal_score` uses `(now - last_touch).num_days()` from `Utc::now()`,
> so the same brain, same query, same content returns a **different ranking**
> tomorrow with no new information — which quietly undercuts the
> "byte-reproducible / deterministic" headline claim.

I asserted this from reading the code, cited DMF (arXiv 2606.03463) in support,
and recommended it as the highest-value remaining item. It is wrong.

## What is actually true

Recall **ordering is time-invariant on both retrieval paths**, for two
different reasons.

### 1. Top-k FTS and cascade — invariant by construction

`ranking::apply_recency_weight` is **multiplicative**:

```rust
let recency_factor = 0.5_f64.powf(age_days / half_life_days);
hit.signal_score *= recency_factor;
```

Advancing `now` by `D` increases every candidate's `age_days` by exactly `D`,
multiplying every factor by the common constant `0.5^(D/half_life)`. Scaling
all scores by one positive constant cannot reorder them. The ordering is
provably invariant under a clock shift, up to float precision.

### 2. `recall` / `recall_local` / `recall_at` — invariant by omission

`decayed_signal_score` **is** order-sensitive in principle (linear with a
floor: `raw * max(1 - days/700, 0.5)`, so memories saturate at different
times). But `recall_at` applies it in a `map` and **never re-sorts**:

```rust
let memory_hits: Vec<_> = tact.memories.iter()
    .filter(|m| ...visibility...)
    .map(|mut hit| { hit.signal_score = decayed_signal_score(...); hit })
    .collect();
```

The order stays TACT's retrieval order. The decayed value is written into the
hit and never used for ranking.

## Measurement

`crates/spectral/examples/drift_probe.rs`, 24 memories over multiple time
spreads, comparing recall output at several anchors:

| corpus | anchors compared | ordering changed? |
|---|---|---|
| 3000 days back, 120-day spacing | anchor vs +5y vs +30y | **no** |
| 500 days back, 20-day spacing (straddles the 350-day floor) | anchor vs +200d vs +700d | **no** |

Also pinned as tests in `crates/spectral/tests/deterministic_anchor.rs`:
`recency_decay_is_order_invariant_in_the_topk_path` asserts the equality
directly, so if the decay function ever changes shape the property fails
loudly.

## What IS time-dependent

The decayed `signal_score` **values** callers read. Those shrink as the clock
advances, on both paths. That matters for anything that thresholds or reports
on the score, and for historical replay where recency should be measured from
the query's own date rather than today's.

## A real latent defect, found on the way

`recall_at` computes a decayed score and never uses it for ordering. That is
either dead computation or a missing sort. It is **not fixed here**: adding a
sort would change the ordering of every `recall`/`recall_local` result, which
is a behaviour change on the default path and needs its own preregistration and
oracle run. Recorded so it is not mistaken for intent.

## What was kept

- `MemoryStore::latest_created_at` (default impl returns `None`) and
  `Brain::latest_interaction_time` — a deterministic corpus anchor.
- `RetrievePlan::reproducible(brain, question, visibility)` — the published
  plan with that anchor applied to **both** routes.

These are worth keeping for replay, audit and regression use, and because they
pin an ordering property that currently holds by construction rather than by
design. They are **not** a fix for a determinism bug, because there was no
determinism bug in ranking.

## The lesson, again

The repo's own measurement-discipline note says: *"I repeatedly proposed before
measuring... Always: pre-specify the hard baseline, run warm, run twice."* This
is the same failure in a new place — a code-reading inference presented as a
defect, twice, before a probe existed. The probe took ten minutes and refuted
it.

DMF's argument for interaction-count decay remains sound *for DMF*, whose decay
drives pruning and eviction. Spectral's decay does not drive either, so the
argument does not transfer. Borrowing a paper's conclusion without checking
whether its premise holds locally is how the wrong claim got made.
