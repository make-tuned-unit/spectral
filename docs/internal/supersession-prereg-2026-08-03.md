# Preregistration — read-time supersession suppression — 2026-08-03

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

**Written before the measurement.** Binding.

## This is a SAFETY gate, not an efficacy test

State that first because it determines how the result may be read.

`knowledge-update` has **99.4% session-recall** and **87.2% end-to-end
accuracy**. Retrieval already finds the right session almost always. The
hypothesised failure is that the actor receives *every* version of a changed
fact — "my note-taking app is Notion", and later "my note-taking app is
Obsidian" — and has to decide which is current.

Suppressing the stale version can only **remove** things from a result set. So
on every metric the $0 oracle computes — session-recall, key-recall,
zero-recall — this lever can lose and cannot win. Its hypothesised benefit is
entirely actor-side.

**Therefore: no oracle result can support shipping this.** The most a pass
establishes is *"it does not damage retrieval"*, which is a precondition for a
paid A/B, not evidence of value. A fail closes it at $0.

## Decision rules (binding)

1. **Primary (safety).** On `knowledge-update`: session-recall must not drop
   more than **0.5pp**, key-recall not more than **1.0pp**, and zero-recall must
   not increase.
2. **Control.** `temporal-reasoning` must move by **≤ 0.3pp** on session-recall.
   Few temporal questions should carry first-person state assertions, so a
   larger move means extraction is firing where it should not.
3. **Suppression must actually happen.** At least **5** questions must show a
   changed context hash. A "safe" result achieved by never firing is not a
   pass — it is an untested lever, and will be recorded as such.
4. **Cost.** Mean context tokens must not rise (widening backfills suppressed
   slots; it should be roughly neutral, and a rise means backfill is admitting
   junk).
5. **Default stays OFF regardless.** Only a paid, powered actor A/B on
   knowledge-update could justify enabling it.
6. **One shot.** No pattern loosening after seeing the result.

## Design constraints taken from this repo's record

- **Read-time only, never deletion.** `partition` returns both halves and the
  stored memories are untouched. The turn-contract debate settled that the write
  path must not erase evidence of a read-path defect.
- **Conservative extraction.** A topic key is assigned only by narrow
  first-person state-assertion patterns (`my <attr> is …`, `I switched … for
  <attr>`). Anything unmatched is `Unclassified` and unsuppressable. Recall is
  sacrificed for precision on purpose: a missed supersession costs nothing, a
  wrong one destroys answer evidence.
- **Cross-session only, by default.** Within a session a restatement is usually
  elaboration ("my laptop is old" → "my laptop is a Framework 13"), not
  replacement. `SPECTRAL_SUPERSESSION_ANY_SESSION=1` ablates this guard.
- **Undated memories never suppress dated ones**, and ties break on memory key,
  so the outcome cannot depend on input order.
- **Pool widening is enabled with the lever**, so suppressed slots are
  backfilled and the output size is preserved. Without that, suppression is a
  guaranteed loss on any set-recall metric even when it improves what the actor
  reads.

## Method

LongMemEval-S (**in-sample**), categories `knowledge-update` (78) and
`temporal-reasoning` (133, control). $0 oracle, zero LLM calls,
`--fresh-brains --no-keep-brains`. Single lever `SPECTRAL_SUPERSESSION=1`.

Baseline is the V1 arm already measured today
(`policy-v2-result-2026-08-03`-adjacent run): knowledge-update 99.4% /
58.1% / 0 zero-recall; temporal-reasoning 96.0% / 49.2% / 1.

## Prior

That the safety gate passes: moderate-to-high — extraction is deliberately
narrow, so it should rarely fire.

That it fires at all on ≥5 questions (gate 3): **genuinely uncertain**, and the
most likely way this run fails. LongMemEval's knowledge-update questions are
built from natural dialogue, and the narrow `my <attr> is <value>` frame may
simply not be how those updates are phrased. If gate 3 fails, the honest
conclusion is that deterministic supersession extraction needs a broader
pattern set than can be written conservatively — which is an argument that this
belongs in the graph/triple layer (where Spectral already has real
supersession) rather than in a regex over free text.
