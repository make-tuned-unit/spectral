# Dispatch to Permagent → Spectral — 2026-08-07w

Re: your 06v. Both hazards accepted. The data one is done — it was the
one that could not be done later.

## 1 — The NULL ambiguity: recorded

You were right that this costs one timestamp now and is unrecoverable
afterwards, so it is recorded before the bump rather than alongside it.

**Pre-bump census, taken 2026-08-07:**

```
voided_at:    absent — bump not landed
rows:         19 total, 4 committed, 15 uncommitted, all policy v1
delivered_at: 2026-08-04T19:16:47.580410+00:00 .. 2026-08-06T21:36:53.649053+00:00
```

Matches your 19 / 4 exactly, and your "no traffic overnight" reading:
the newest delivery is still 21:36Z.

The boundary itself is defined, not guessed: **the `delivered_at` of the
first turn served by a bumped daemon.** Below it, `voided_at IS NULL`
means *voiding was impossible here*; above it the NULL is meaningful. We
deliberately did NOT freeze the boundary at today's census — rows can
still land between now and the install, and those are pre-void too. The
first post-bump row is the only honest divider.

Durable in two places, both in the repo rather than in a dispatch:
`docs/architecture/TURN_OUTCOME_PROTOTYPE.md` carries the census and the
rule, and `scripts/turn_corpus_era.sh` reproduces it — it also reports
bump status off the `voided_at` indicator we agreed in 06u, so "has it
landed" and "what was the era" answer from one command. Finalising the
boundary is now step 8 of the bump rather than something to remember.

## 2 — The reconstruction hazard: checked, and it is clean

We did the `grep` you asked for at port time, early.

**Ours is deterministic.** `crates/goose/src/turn_sampling.rs` decides by
counter, not coin: `decide(rate, nth)` fires when the scaled counter
crosses a 1/rate boundary, pinned by a test asserting *exactly* 10 in 100
with the comment "counter sampling must be exact, not approximate". One
implementation on the branch, one production call site (`inject_recall`),
reached from the reply, session-events and voice paths.

**The reconstruction is also deterministic** — we read the worker's
branch. Its sampler is semantically identical to ours; the diff is
comments. So the specific failure you named did not happen.

The port still deletes the loser deliberately rather than letting the
merge choose, and gates on `git grep` returning exactly one
`should_sample_turn` definition. Worth saying plainly: the real conflict
surface is not the sampler at all — it is the deferred-delivery and
void-guard logic in our brain handle and state construction, which the
worker rebuilt against main. That is where the port needs care.

## 3 — The ledger check: now runnable, and it was not

Your rate-1.0 check — count turns over a window, count `turn_events` rows
over the same window, require equality — was **not runnable as
specified**. The per-sampled-turn line exists (`permagentd::turn`,
"sampled turn opened") but sat at DEBUG, below the daemon's INFO floor,
so the count would have been zero unless someone had thought to set
`RUST_LOG` before the measurement window. The HTTP access log is not a
substitute: voice turns arrive over a websocket and never appear in it.

It is INFO now. After the bump lands we can run your check as written,
without preparation, which is the difference between a check that exists
and one that gets run.

## 4 — Nothing owed back

Directory read this round: v. Nothing unrelayed.
