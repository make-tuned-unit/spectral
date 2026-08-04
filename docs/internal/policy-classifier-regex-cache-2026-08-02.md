# Policy classifier regex cache — measured — 2026-08-02

## What was wrong

`spectral::policy::QuestionShape::classify` compiled **11 regexes on every
call** via `Regex::new(...).unwrap()`, with no caching.

This is the same defect measured and fixed for `tact::classifier`,
`tact::extractor` and `spectrogram::dimensions` on 2026-07-25
(`read-path-regex-cache-2026-07-25.md`). It reappeared because `policy.rs` was
migrated in from `spectral-bench-accuracy` on 2026-07-31 (PR #237) and the
harness code did not carry the library's hot-path discipline. The migration was
behaviour-preserving, which is what it was checked for; it was not checked for
cost.

`classify` is called **twice per turn** under `TurnPolicyVersion::V2Shaped`
(`turn.rs:120` for the cascade profile, `turn.rs:136` for the route), and once
or more per question on every bench retrieval path.

## The fix

A `classifier_pattern!` macro defining one `OnceLock<Regex>` accessor per
pattern. `unwrap` is retained — these are static literals, so a compile failure
is an authoring bug — but it now runs at most once per process.

The two near-duplicate recency sub-gates are kept as **separate** statics and
documented as deliberately different: the `where` arm admits bare `recent`, the
general factual arm admits `most recently`. Collapsing them would change
routing.

## Behaviour preservation

A 25-question classification corpus covering every arm of the two-level
classifier was written **against the pre-fix implementation** and passes
unchanged after it. Plus a case-insensitivity test and two known-gap pins
(below). `cargo test -p spectral --lib policy`: 7/7.

## Measurement

Tool: `crates/spectral/examples/policy_classify_bench.rs`, release, warm,
warm-up pass discarded, 3 runs. Both arms in one binary over the same question
mix, so the comparison isolates compilation.

| run | uncached (µs/call) | cached (µs/call) | speedup |
|---|---:|---:|---:|
| 1 | 550.700 | 0.148 | 3728x |
| 2 | 530.558 | 0.146 | 3632x |
| 3 | 523.396 | 0.142 | 3676x |

End-to-end, via `crates/spectral/examples/turn_latency.rs` (corpus=400,
iters=300, release, warm), measured by stashing only `policy.rs`:

| arm | p50 (ms) | p95 (ms) |
|---|---:|---:|
| legacy `recall_cascade_scoped` | 1.027 → 0.971 | 1.416 → 1.359 |
| `turn` V1 (never classifies) | 0.846 → 0.796 | 2.954 → 2.543 |
| **`turn` V2Shaped** | **2.806 → 0.847** | **5.245 → 3.287** |

**V2Shaped p50 −70%, p95 −37%.**

The headline finding: before this fix, the path that executes the **published
retrieval policy** (V2Shaped) was **2.7x slower than the legacy recall path it
is meant to supersede** (2.806 vs 1.027 ms p50), and roughly 70% of that time
was regex compilation. V1 — the arm the 2026-07-31 latency gate measured —
never classifies, so the gate never saw this.

## What this does NOT change

**The turn latency gate still FAILS and `turn` is still not the default recall
path.** The gate's kill line is on the V1 recall-only p95 (`+87.1%` this run vs
a `+5%` line); its diagnosis stands unchanged — the regression is the
synchronous delivery-write commit, not retrieval, and not classification. This
fix removes a cost the gate was not measuring. It is not an attempt to move the
gate, and the gate's verdict is unchanged.

## Two known gaps found by the pinning corpus — recorded, NOT fixed

Both are pinned by tests so they cannot be closed accidentally. Both change
routing on the published benchmark, so both need a prereg and an oracle run
like any other retrieval lever.

1. **`FactualCurrentState` misses bare "current".** The variant is documented as
   *"What is my current X" — most-recent-wins factual*, but the sub-gate pattern
   lists `currently`, not `current`. The exact phrasing in its own doc comment
   falls through to plain `Factual` and loses recency priority.
   Test: `bare_current_misses_the_recency_sub_gate`.

2. **`what should i` in the GeneralPreference gate is dead code.** It is checked
   *after* the Factual branch `^(?:what|where|who|which)\b`, so any question
   beginning with "what" routes to `Factual` first. Given
   `single-session-preference` is the weakest measured category (56.0%,
   `docs/RESULTS.md`), this is a strong Phase 3 candidate.
   Test: `what_should_i_is_shadowed_by_the_factual_branch`.

## Method note

Timings follow `ingest-cost-profile-2026-07-31.md`: release, warm, warm-up
discarded, more than one run, and the A/B taken back-to-back in the same session
on the same machine (M-series mac).
