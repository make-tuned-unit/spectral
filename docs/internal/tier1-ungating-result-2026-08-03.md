# Tier-1 ungating — Phase A — REJECTED as preregistered

Prereg: `tier1-ungating-prereg-2026-08-03.md`. Real Permagent brain (1,979
memories, 17 real wings), 217 real queries, $0, no LLM.

## Verdict

| gate | rule | result |
|---|---|---|
| 1. Reachability | tier 1 fires on ≥ 30% | **FAIL — 12.0%** |
| 2. Non-degradation | ≥ 95% keep result-set size | PASS — 26/26 |
| 3. Latency | ≤ +20% median | **BORDERLINE/FAIL** — +15.4% and +21.9% across two runs |
| 4. Determinism | byte-identical repeats | PASS — 0 non-deterministic |

**Rejected on gate 1.** The default stays gated (`tier1_requires_hall: true`).

## The mechanism works exactly as designed

| | fires on |
|---|---:|
| gated (wing AND hall) | **0.0%** |
| ungated (wing only) | **12.0%** |

And the attribution is clean:

- wing detected on **27 of 217** queries (12.4%)
- of those 27, the constellation index was **empty 0 times**
- tier 1 fired on 26 of 27

So once a wing is detected, the index always has content and the tier fires.
The hall conjunction really was suppressing it to zero. Removing it works.

**The binding constraint is wing detection on the query — 12.4% — not the hall
gate and not the index.**

## I got the headroom estimate wrong, and preregistered a gate on it

The prereg predicted ≥30% reachability from an earlier measurement that "46.5%
of real queries name a real wing". That number was produced by a **loose token
heuristic** I wrote in a throwaway probe — it counted a hit if any
hyphen-separated fragment of a wing name appeared anywhere in the query, so
`atlas-atlantic` matched on the bare word "atlas", `personal` on "personal",
and so on.

The shipped `detect_wing` uses the actual wing regex and gets **12.4%**. My
proxy overestimated by ~3.7x, and I set a preregistered threshold on it rather
than on the classifier that would actually run.

That is the same error as measuring the constellation tier on LongMemEval: I
substituted a convenient measurement for the real one. The gate did its job —
it failed, and it failed for a reason worth knowing.

## What this actually tells us about wings

A wing fires when the query **names the project**. Real agent queries mostly do
not:

```
Hi Henry, can you tell me about some of your new capabilities?
Give me a tour of the app.
Okay, interesting, continue.
```

12.4% of real queries name a project area. That is not a defect in the
classifier — it is what conversation looks like. Wing-scoped retrieval is
therefore inherently a **minority path**, useful precisely when the user is
explicit about context, and structurally unable to be the primary route.

This reframes what wings are for. They are not a general retrieval accelerator;
they are a **scoping mechanism for when scope is stated**. Ambient context
(`RecognitionContext::focus_wing`) is the mechanism for the other 87.6% — the
agent knows which project it is in even when the user does not say so. That
path already exists and is not exercised here.

## What is kept

`TactConfig::tier1_requires_hall`, **default `true`**. The ungating is
implemented, tested and measured; it is not enabled. It becomes interesting
only if wing detection is driven by ambient context rather than by query text,
which is a different experiment.

## Phase B is still blocked, and that is the real bottleneck

Phase A can only show the mechanism is safe. Whether the 12% of queries that
now reach the constellation tier get *better* results is unanswerable here: the
real brain has no ground-truth answer keys, and the labelled corpora
(LongMemEval, LoCoMo) have no wing structure at all.

**Every verdict ever recorded about TACT tier 1 — including the
0-wins/2-losses/9-ties that nearly justified deleting the fingerprint table —
was measured on corpora with no wings.** That has been the blocker the whole
time, and no amount of $0 work removes it.

The unblock is Permagent emitting real outcomes through the turn ledger
(`turn_events` / `turn_members`): real queries, real wings, recorded use. That
is the dataset this question needs, it does not exist yet, and building it is a
product decision rather than a library one.
