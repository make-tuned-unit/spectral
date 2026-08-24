# R42 result — discriminative margin: FAIL on the preregistered gate

Measured 2026-08-19 against `r42-discriminative-margin-prereg-2026-08-19.md`.
Real brain `~/.permagent/brain`, 2,808 enrolled, **2,712 probed**, in-memory
index, $0, seconds. Flag `ScoreConfig::discriminative_margin` stays **off**.

## Gate

| # | clause | baseline | treatment | verdict |
|---|---|---:|---:|---|
| 1 | exact-content `Recognized` >= 90% | 83.8% | **90.6%** | **PASS** |
| 2 | exact-content `Recognized(WRONG)` stays 0.0% | 0.0% | **0.1%** | **FAIL** |
| 3 | foreign false `Recognized` = 0/300 | 0.0% | 0.0% | PASS |
| 4 | degraded probes not below baseline | 55.1 / 60.7 | 59.5 / 65.5 | PASS |
| 5 | byte-identical ties still not `Recognized` | 27 | 27 | PASS |

**Overall: FAIL.** One clause failed, so the flag stays off, as preregistered.

The prereg warned that an id-based metric miscounts a byte-identical twin as
wrong, and predicted that as the likely source of any `Recognized(WRONG)`.
**That prediction was wrong.** The probe now separates the two cases, and
**zero** of the wrong identities named a byte-identical twin — they named
genuinely different memories. The failure is real, not an artefact.

## What the change did and did not do

It works exactly as designed on the population it was designed for. Of the 48
lead-margin misses in the 300-memory diagnostic, 13 of the 21 *distinguishable*
near-duplicates were promoted correctly, while all 27 byte-identical ties
stayed `Familiar` — the safety property holds by construction, since identical
candidates have zero exclusive evidence.

The damage is on **degraded** stimuli:

| probe | Recognized gain | Recognized(WRONG) gain | ratio |
|---|---:|---:|---:|
| exact | +6.8pp | +0.1pp | 68 : 1 |
| drop30 (30% tokens removed) | +4.8pp | +0.7pp | 6.9 : 1 |
| head50 (first 50% of tokens) | +4.4pp | +1.7pp | 2.6 : 1 |

Mechanism: a fragment's surviving features are a **biased sample** of the
memory's features. Between two near-duplicates, which exclusive features
survive truncation is close to arbitrary, and the absolute bar
(`recognize_min_score` = 3.0) is roughly one or two rare features — so a
couple of chance-exclusive matches can decide identity. On an intact stimulus
both candidates' exclusive evidence is fully observed and the comparison is
sound; on a fragment it is not.

## Decisions

1. **`discriminative_margin` ships off.** The code and both instruments land
   because the diagnostic is worth having and the flag makes the follow-up a
   one-line change, but production behaviour is byte-identical to today's.
2. **The 16% is now explained, and most of it is not a defect.** 100% of
   exact-re-encounter misses are lead-margin failures against a near-duplicate;
   56% of those are byte-identical memories, where `Familiar` — "I have seen
   this, I cannot say which instance" — is the only honest verdict a
   content-based engine can give. Recognition is healthier than the 83.8%
   headline suggested.
3. **Proposed R42b** (to be preregistered, and decided on a *different*
   corpus — the LoCoMo brains — so it is not fitted to this sample): fire the
   exclusive-evidence rule only when the stimulus is substantially intact, and
   require the exclusive evidence to rest on at least `familiar_min_features`
   (2) independent features rather than a single rare match. Both reuse
   existing constants. Not implemented here; reading a result and then tuning
   against the same sample is how a gate stops meaning anything.
4. **Upstream finding for Permagent** (separate from recognition): the brain
   holds 2,808 memories with **2,636 distinct contents** — 59 duplicate-content
   groups covering 231 memories (8.2%), the largest being
   `Started working in project Grocery Savers (…)` stored **25 times**. These
   are ambient `project_selected` events with no session, the exact class R45
   leaves on the gap heuristic. They inflate the index, split evidence across
   twins, and are the direct cause of half the lead-margin ties. Spectral
   already models recurrence (`last_reinforced_at`, `RememberResult.recurrence`);
   keying such repeats so they reinforce one memory instead of writing another
   would remove the ties at source. Sent to the runtime session.

## Instruments added

```
target/release/examples/recognition_gate_diagnostic <memory.db> [n]
R42_DISCRIMINATIVE=1 target/release/examples/recognition_e2e <db> content <locomo.json> 300
```
The diagnostic attributes every non-`Recognized` verdict to the specific gate
that failed, and reports near-duplicate / byte-identical runner-ups. `$0`,
read-only, seconds. It is the right first stop for any future verdict question:
the aggregate rate says *how often*, this says *why*.
