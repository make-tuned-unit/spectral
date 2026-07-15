# Dispatch → Permagent: associative recall (ACR) is now a library feature — enable & validate

**Date:** 2026-07-15 · **Spectral commits:** `e0b8d35` (module), `f273ef5` (recall-path wiring)
**TL;DR:** Local, embedding-free **associative recall** — spreading activation over
co-occurrence to recover memories that share no words with the query — is now a
first-class Spectral capability, **off by default**, enabled via one config field.
We proved it recovers big retrieval gains but could NOT prove accuracy conversion
here (our test actor is too strong / our sandbox network is flaky). **You have the
production environment to answer that.** Please enable it and A/B.

## What it is (and why it matters for the mission)

FTS finds memories by words; it structurally cannot bridge a vocabulary gap
(query "homegrown ingredients" ↔ memory "growing cherry tomatoes, basil, mint").
Vector DBs solve that with an embedding model — often a cloud API, so the user's
data leaves the machine. **ACR solves it locally and deterministically:** FTS
finds the seeds, then activation spreads through co-occurrence links to reach
associated memories. Two substrates:

- **episode** (same-session): completes an already-found session (memories near
  the seed in the same conversation, ranked by `created_at` proximity).
- **cross-session** (pseudo-relevance feedback): each seed's own content becomes a
  query, so BM25 IDF surfaces its distinctive tokens and reaches associated
  memories in OTHER sessions — finds contributing sessions the query alone missed.

## How to enable

`spectral_graph::spreading::{associative_spread, AssocSpreadConfig, SpreadMode}`,
and it's wired into the cascade recall path via `CascadePipelineConfig.spread`:

```rust
use spectral_graph::spreading::{AssocSpreadConfig, SpreadMode};

let pipeline = CascadePipelineConfig {
    // ...your existing config...
    spread: AssocSpreadConfig {
        mode: SpreadMode::Rerank,   // start here (see below)
        seeds: 3,
        rerank_b: 15,
        ..AssocSpreadConfig::default()
    },
    ..Default::default()
};
// then brain.recall_cascade_with_pipeline(query, &ctx, &pipeline)
```

Default is `SpreadMode::Off` — a pure no-op, zero behavior change until you opt in.
You can also call `associative_spread(brain, &mut hits, &cfg)` directly on any hit
list.

## Which mode

Measured on real LongMemEval (retrieval-level; +pp = answer-key recall over FTS):

| mode | what it does | retrieval | context cost |
|------|------|------|------|
| `Rerank` | displace weakest results with proximity mates; **session-preserving** | +16–23pp | ~constant |
| `Combined` | cross-session finds missed sessions, then episode completes each | +21–30pp | +20–30% |
| `Episode` / `CrossSession` | single substrate | +14 / +2pp session | moderate |

**Recommendation: start with `Rerank`** — it recovers big key-recall at ~constant
context, so it has no distraction tax and (fixed) never drops a contributing
session. It is the mode most likely to convert to accuracy for a strong actor.
Use `Combined` where recall genuinely gates the answer (multi-session counting,
weaker actors).

## The ask

**Run an accuracy A/B in your production: baseline vs `spread` enabled.** We
verified the *retrieval* recovery is real and large, but on our benchmark with a
strong cloud actor it did NOT convert to accuracy (the actor already compensates;
adding/swapping context is net-neutral). Its payoff should appear where the actor
can't paper over a missing memory — a weaker/cheaper actor, or a
retrieval-completeness-bound task. That's your environment, not ours.

If it converts, this is the local-first, embedding-free answer to semantic recall
— a real differentiator for users who want to retain control of their data. If it
doesn't, we've bounded it honestly and it stays an opt-in tool. Full analysis:
`docs/internal/tact-unlock-synthesis-2026-07-15.md`.

## Safety notes
- Off by default; enabling only augments/reorders the returned context.
- Deterministic, local, no network, no embedding model.
- `Combined`/`CrossSession` issue extra FTS queries (one per seed) — bounded, cheap.
- `Rerank` keeps context size ~constant; the others grow it (budget-capped).
