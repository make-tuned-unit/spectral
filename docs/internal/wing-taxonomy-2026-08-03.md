# Why the constellation tier never fires — and it isn't the fingerprints

## Summary

TACT tier 1 — the constellation/fingerprint path — fires on **0.9% of real
Permagent queries**. I previously concluded the fingerprint table should be
retired. That was wrong. The tier is starved by its gate, not by its idea.

The gate is:

```rust
if let (Some(w), Some(h)) = (wing, hall) { ... fingerprint_search ... }
```

It requires **both a wing and a hall to be detected on the query**. Measured on
217 real Permagent queries against the real brain's own taxonomy:

| gate component | fires on |
|---|---:|
| **wing** (real project taxonomy) | **46.5%** |
| **hall** (`decided\|chose\|remember\|recommend\|…`) | **5.5%** |
| **both — tier 1 reachable** | **0.9%** |

**Wings work.** When the taxonomy is real, nearly half of real queries name one.
**Halls are the blocker**, and structurally so.

## Why hall detection cannot work on queries

A hall is a *memory type* — fact, preference, discovery, advice, event. The
hall rules look for a speaker asserting one:

```
decided|chose|switching to|using|will use|agreed|locked in|decision|auth  -> fact
remember|preference|favourit|favorit|likes|prefers                        -> preference
```

Those are the words of someone **stating** a fact or preference. Real queries
are someone **asking**:

```
Hi Henry, can you tell me about some of your new capabilities?
Give me a tour of the app.
Okay, interesting, continue.
```

A question does not announce what kind of memory would answer it. Requiring the
query to declare a hall asks the user to pre-classify the answer they are
looking for. **Hall is a property of the memory, not of the question**, and the
gate treats it as if it were both.

This is why the tier reads as useless: it has been measured (0 wins, 2 losses,
9 ties) only on the ~3% of cases where a query happened to contain assertion
vocabulary — a badly biased sample, and a tiny one.

## The two defects, separated

**1. The shipped wing taxonomy is demo data — and it corrupts real brains.**

`default_wing_rule_pairs()` ships:

```
alice|coffee|anniversary|colou?r|favourit|sons|noah|leo|carol-doe  -> alice
apollo|polymarket|strategy|weather|prediction|wager|trade          -> apollo
acme|widget|bob|recipe|cook|feast                                  -> acme
```

In the **real** Permagent brain these fixtures have captured live memories:
`apollo` 46, `alice` 18, `acme` 17, `polaris` 16. Real content is being filed
into fictional topic areas by keyword collision. The consumer's genuine
taxonomy — `jesse`, `henry-infra`, `permagent`, `getladle`,
`grocery-savings-planner`, `polybot`, `atlas-atlantic`, `wealthie`, `kinrows` —
sits alongside them.

Wings are **consumer-supplied domain knowledge**, and `BrainConfig::wing_rules`
already supports that properly. The defect is shipping fixtures as the default
instead of an empty or genuinely generic set.

**2. The tier-1 gate requires hall on the query.** See above. This is the
higher-impact defect.

## What a corpus-derived taxonomy cannot fix

I tried deriving wings automatically from corpus statistics — salient
mid-frequency stems as topic anchors, Spectral's own landmark thesis applied to
the wing problem. Two attempts on LongMemEval-S:

| anchor band | wing detection | tier-1 reachable |
|---|---:|---:|
| shipped demo fixtures | 11.4% | 3.2% |
| df ∈ [12, 10% of docs], 64 anchors | 59.8% | 6.2% |
| df ∈ [40, 1% of docs], 96 anchors, stopworded | 11.0% | 1.4% |

The permissive band produced anchors like `just`, `want`, `could`, `most` —
high frequency, zero topicality. The strict band produced genuinely topical
anchors (`hiking`, `treatment`, `productivity`, `roasted`) that simply do not
appear in most questions. Coverage and discrimination trade off directly, and
neither end works.

**Conclusion: wings are not auto-derivable from open-domain chat, and should not
be.** They are deployment knowledge. The real brain proves the concept works
when a consumer supplies it — 46.5% of real queries name a real wing.

## The fix that makes the existing design work

**Do not retire the constellation tier. Ungate it from hall.**

Tier 1 should fire on **wing alone**, searching the wing's fingerprints across
halls, with hall as an optional refinement rather than a precondition. On the
real workload that moves tier-1 reachability from **0.9% → 46.5%** — a 50x
change in how often the constellation path is even consulted.

Only then is the "does the constellation tier add value?" question actually
answerable. The existing 0-wins/2-losses/9-ties verdict was measured on 11
cases drawn from a structurally biased 3% slice; it is not evidence about the
design.

## Status

- Wing-taxonomy defect: **recorded, not fixed.** Removing demo fixtures from the
  default changes classification for every existing brain and needs a migration
  plan plus its own prereg.
- Tier-1 gate: **recorded, not fixed.** Ungating is a retrieval behaviour change
  and needs a prereg and an oracle A/B — run on the **real brain**, not on
  LongMemEval, which has no wing structure to exercise.
- Fingerprint retirement: **recommendation withdrawn**, cost measurements stand,
  default unchanged at `true`. See `fingerprint-retirement-2026-08-03.md`.

## Method note

Measuring the constellation tier on LongMemEval was the original error.
LongMemEval is synthetic open-domain conversation with no project structure —
exactly the workload where wings cannot exist. The real Permagent brain has 12
genuine wings over 1,738 memories. **A feature should be measured on the
workload it was designed for.**
