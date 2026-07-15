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

Weak-actor A/B (haiku actor, sonnet-4-6 judge, temp=0, n=30, transport-clean):

- knowledge-update — baseline vs ACR precision (RERANK): _[pending]_
- multi-session — baseline vs ACR completeness (COMBINED): _[pending]_

_(Results appended when the runs complete. A net-positive here — where sonnet was
net ≤ 0 — is the proof that ACR's recovery is real lift for the actors that need
it: the local/cheap models a data-control user actually runs on-device.)_
