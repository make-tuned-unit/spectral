# R42 prereg — the lead-margin gate, and evidence that only one candidate has

**Written before the change was implemented or measured. Published with the
result regardless of outcome.** Date: 2026-08-19.

## What the diagnostic found

`recognition_e2e` reported that 16.3% of probes made of a memory's **own
content** return `Familiar` rather than `Recognized` on the real brain. A new
instrument, `recognition_gate_diagnostic`, attributes every one of those
misses (n = 48 of 300 probed, 2,808 enrolled):

| condition | failures |
|---|---:|
| coverage >= 0.35 | 0 (0.0%) |
| score >= 3.0 | 0 (0.0%) |
| familiarity >= 0.60 | 0 (0.0%) |
| **lead >= 1.5x the runner-up** | **48 (100.0%)** |
| lead-margin **only** (all other gates pass) | **48 (100.0%)** |
| runner-up is a near-duplicate (containment >= 0.50) | 48 (100.0%) |
| runner-up content is **byte-identical** | 27 (56.2%) |

Median lead ratio 1.00x, median runner-up containment 1.00. So the miss is
never a weak-evidence problem; it is always two candidates whose *total*
evidence ties, because they share nearly all of their text.

The corpus explains it: 2,808 memories hold 2,636 distinct contents. 59
duplicate-content groups cover 231 memories (8.2%), the largest being
`Started working in project Grocery Savers (…)` stored **25 times**.

## The two populations, and why only one is a bug

1. **Byte-identical (56%).** Two enrolled memories with the same text. No
   content-based engine can prefer one, and it must not pretend to.
   `Familiar` — "I have seen this; I cannot say which instance" — is the
   correct verdict, and it matches the dual-process account (familiarity
   without recollection). **Not addressed here.** The remedy is upstream: such
   repeats should reinforce one memory rather than write another (Spectral
   already models recurrence), which is a Permagent write-path question.
2. **Near-duplicate but distinguishable (44%).** e.g. two
   `Navigated to <different URL>` memories, containment 0.81. Distinguishing
   evidence exists — the URL, the id, the number — but it is a small share of
   each candidate's *total* score, which is dominated by the shared template.
   The margin rule compares totals, so it cannot see it. **This is the bug.**

## The change

When the plain margin fails, compare the candidates on **exclusive evidence**
only: for the top two traces, the rarity-weighted score of features
(pair/gram hashes) matched by one candidate and **not** the other.

Promote to `Recognized` iff

```
exclusive(best)  >= exclusive(runner_up) * recognize_margin   (1.5x)
AND exclusive(best) >= recognize_min_score                    (3.0)
```

Deliberately **no new tunable constants**: the exclusive evidence must clear
the same relative bar (`recognize_margin`) and the same absolute bar
(`recognize_min_score`) that total evidence must already clear for identity.
Byte-identical candidates have `exclusive(best) = 0` and can never be
promoted, which keeps population 1 honest by construction.

Behind `ScoreConfig::discriminative_margin` (default **off** in this PR, so
production behaviour is unchanged until the gate is read).

## Preregistered gate

Measured with `recognition_e2e` on the real brain (2,807 enrolled, 300 probed)
and `recognition_gate_diagnostic`. PASS requires **all**:

1. Exact-content probes: `Recognized` **>= 90%** (baseline 84.0%).
2. Exact-content probes: `Recognized(WRONG)` stays **0.0%** — no promotion
   ever names the wrong memory.
3. Foreign LoCoMo probes: false `Recognized` stays **0/300**.
4. Degraded probes (`head50`, `drop30`): `Recognized` **not below** baseline
   (54.0% / 59.3%).
5. Byte-identical duplicate pairs are **still not** `Recognized` — verified by
   the diagnostic's identical-runner-up count remaining a lead-margin failure.

Failing any clause is reported as FAIL and the flag stays off.

## Known metric artefact, stated in advance

`recognition_e2e` scores top-1 against a specific memory **id**. When two
memories are byte-identical, whichever is returned is equally correct, so the
published "top-1 95.3% / Recognized(WRONG)" figures understate accuracy by
counting an identical-content twin as wrong. The e2e metric is left unchanged
so this run stays comparable with R37/R38; the diagnostic reports the twin
count separately.
