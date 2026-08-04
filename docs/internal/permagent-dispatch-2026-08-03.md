# Dispatch to Permagent — 2026-08-03

**Relay this to the Permagent team.** One blocking ask, one data ask, one
heads-up about a live-data repair.

---

## Context in one paragraph

Spectral has spent months measuring retrieval levers and refuting nearly all of
them. We now know why several of those verdicts were unreliable: they were
measured on benchmark corpora (LongMemEval, LoCoMo) that have **no topic
structure** — no wings — while Spectral's wing-scoped retrieval paths are
designed for exactly the structure Permagent's real brain has. We cannot settle
the remaining questions from our side. We need outcome data from yours.

---

## ASK 1 (blocking) — call `Brain::turn` and report outcomes

**This is the single thing that unblocks the most work.**

Today Permagent presumably calls `recall_*`. Those auto-reinforce every hit at
retrieval time, which credits **exposure**, not usefulness. `Brain::turn` was
built to fix that: it retrieves read-only, hands back a receipt, and learns
nothing until you say what was actually used.

```rust
use spectral::{MemoryOutcome, TurnRequest, Visibility};

// 1. Retrieve. Nothing is written except a delivery record.
let turn = brain.turn(
    &TurnRequest::query(user_message, Visibility::Private)
        .with_observations(&[/* things the agent just encountered */]),
)?;

// 2. ... agent answers, using some of what it got back ...

// 3. Report. Only `Used` is reinforced.
brain.record_turn_outcome(
    &turn.receipt,
    &[(hit_key, MemoryOutcome::Used)],   // or Rejected / Unreported
)?;
```

**What this gives us:** `turn_events` / `turn_members` become a labelled corpus
of *real queries against a real wing taxonomy with recorded use*. That is the
dataset that does not currently exist anywhere, and without it the following
questions are permanently unanswerable:

- Does the constellation/fingerprint tier add value? Every verdict to date
  (including "0 wins, 2 losses, 9 ties", which nearly justified deleting a table
  costing 39% of every write) was measured on corpora with no wings.
- Does wing-scoped retrieval beat plain FTS on real project queries?
- Do any of the reranking levers convert to better answers?

**Two caveats, stated honestly:**

1. **`turn` is NOT currently the recommended default recall path.** Its
   preregistered latency gate FAILED — recall-only p95 regressed +87–100%
   against a +5% kill line. The cause is the synchronous delivery-write commit,
   not retrieval (p50 actually *improved* ~19%). If tail latency matters to you,
   adopt `turn` on a sampled fraction of turns rather than all of them, and tell
   us — a deferrable delivery write is the known fix and we will preregister it.
2. **Outcome reporting is the whole point.** A `turn` that is never committed
   leaves memory state completely unchanged and produces no learning signal.
   Calling `turn` without `record_turn_outcome` gives us nothing.

---

## ASK 2 — confirm your wing taxonomy is supplied deliberately

We removed the library's default wing rules. They were example-scenario
fixtures — `alice|coffee|noah|carol-doe`, `apollo|polymarket`,
`acme|widget|recipe` — and they were **capturing real content** in the live
brain by keyword collision.

Please confirm Permagent passes `BrainConfig::wing_rules` explicitly. If it
relied on the library defaults for any wing, that wing will now classify as
`general` for new writes.

Relevant finding for your side: a wing only fires when the query **names the
project** — 12.4% of your real queries. For the other 87.6%, the mechanism is
`RecognitionContext::focus_wing`: if Permagent knows which project the user is
in, pass it. That is scope you have and we don't, and it is currently unused on
the wing-scoped retrieval path.

---

## ASK 3 (heads-up) — a repair is pending on the live brain

`~/.permagent/brain/memory.db` has **119 memories filed into the retired fixture
wings** (`apollo` 46, `alice` 18, `acme` 17, `polaris` 16, `vega` 13, `infra` 5,
`travel` 3, `charity` 1). Real content, fictional topic areas.

The repair is written, targeted and idempotent. It touches only those 8 wings —
your genuine taxonomy (`permagent`, `polybot`, `getladle`, `henry-infra`,
`atlasatlantic-site`, `jesse`, `wealthie`, `kinrows`, …) is untouched. Verified
by dry run: 119 changes, not the 1,053 an unrestricted run would have made.

Jesse runs it (we are not writing to production data from here):

```bash
cp ~/.permagent/brain/memory.db /tmp/permagent-brain-backup.db
cargo run -p spectral-bench-real --release --bin wing_repair -- \
  --brain ~/.permagent/brain --apply
```

---

## What you get back

Once ASK 1 lands, we can finally answer whether Spectral's distinctive
retrieval paths — the constellation tier, wing scoping, the adaptive feedback
loop — earn their cost on a real workload. Right now we are guessing, and we
would rather say so than keep publishing verdicts measured on the wrong corpus.
