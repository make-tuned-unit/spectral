# ACR lift across all memory types — 2026-07-15

Does associative recall (ACR) give real lift on every LongMemEval memory type?
Two prongs: retrieval lift (deterministic, $0, all types) and accuracy conversion
(weak-actor A/B — the test the strong actor couldn't provide).

## Retrieval lift — PROVEN on all 6 memory types

Default published routing + `AssocSpreadConfig::completeness()` (COMBINED),
n=50/category, answer-KEY recall:

| memory type | baseline | ACR | lift | tokens |
|---|:-:|:-:|:-:|:-:|
| single-session-user | 52.4% | **92.4%** | +40.0pp | +26% |
| single-session-assistant | 73.4% | **91.8%** | +18.4pp | +18% |
| single-session-preference | 37.3% | **56.3%** | +19.0pp | +41% |
| knowledge-update | 56.4% | **88.0%** | +31.6pp | +19% |
| multi-session | 50.7% | **72.2%** | +21.5pp | +16% |
| temporal-reasoning | 48.9% | **76.6%** | +27.7pp | +23% |

**Every memory type gains +18–40pp answer-key recall (avg ~+26pp).** Session-recall
is already at/near ceiling (96.7–100%), so the lift is *within-session key
completion* — recovering the specific answer-bearing memories FTS ranked out.
This is real, comprehensive retrieval lift, and it is the metric a retrieval layer
is responsible for.

## Accuracy conversion — the honest test

Retrieval lift is necessary but not sufficient: a *strong* actor (sonnet) already
compensates for a missing memory, so on it ACR netted ≤ 0 (measured earlier —
knowledge-update net +0/−1). The real question is whether the recovery converts
for an actor that *cannot* compensate. A **weaker cloud actor (haiku)** is exactly
that test bed — no local model needed.

Weak-actor A/B (haiku actor, sonnet-4-6 judge, temp=0, transport+auth-clean).
An earlier n=14 knowledge-update run looked +1, so we ran it comprehensively:

| memory type | mode | baseline | ACR | net |
|---|---|:-:|:-:|:-:|
| multi-session | completeness | 19/30 (63%) | 17/30 (57%) | **−2** (fixed 2, broke 4) |
| single-session-user | precision | 27/30 (90%) | 26/30 (87%) | **−1** (fixed 1, broke 2) |
| knowledge-update | precision | 11/14 (79%) | 12/14 (86%) | +1 (n=14, noise) |
| **POOLED (n=74)** | | **57/74 (77%)** | **55/74 (74%)** | **net −2** |

**Honest verdict: the retrieval recovery does NOT convert to accuracy — even for
a weak actor.** The n=14 +1 was noise; the larger runs are net-negative. ACR
*fixes* the questions where a missing memory was decisive (2 counting questions on
multi-session — food-delivery types, citrus fruits) but *breaks* more via
distraction: on "how many hours of jogging/yoga last week?" (gold 0.5h) the
baseline's 40 memories answered correctly; ACR's 57 (add +17 mates) made haiku
miscount. Superlatives ("which store did I spend the most") and arithmetic
("average age") break the same way — the extra context is noise the weak actor
can't filter.

**Why (the whole-arc truth).** On LongMemEval, session-recall is near-ceiling —
the answer session is almost always already retrieved. ACR's +18–40pp is mostly
*additional/redundant* evidence, and adding it distracts the actor (strong actors
already had what they need; weak actors get confused by the extra). Even
constant-context RERANK breaks questions by *displacing* something the actor used.
So the retrieval lift is a proxy that does not translate to end-to-end accuracy on
this benchmark, for any actor tier or config tested. This is consistent with the
entire arc (fetch_mult null, TACT-tiers harmful, novelty null): **on LongMemEval,
retrieval is not the accuracy bottleneck, and adding retrieval hurts via
distraction.**

## Bottom line

- **Retrieval lift: real, large, comprehensive** (+18–40pp key-recall, all six
  types), and shipped as a clean, integrated, deterministic library feature.
- **Accuracy on LongMemEval: does NOT lift** (pooled −2 even for a weak actor) —
  the benchmark's retrieval is saturated and added context distracts. ACR is kept
  OFF by default; the honest recommendation is to NOT enable it for a
  LongMemEval-like workload with a capable actor.
- **Where it could still be real:** a genuinely retrieval-*starved* workload
  (paraphrase-heavy recall where the answer session is often *missed*, not just
  its keys) with an actor that both needs the memory and isn't distracted by
  extra context. LongMemEval is not that workload. Permagent's production is the
  place to find out; the dispatch stands, but with this honest caveat attached.
