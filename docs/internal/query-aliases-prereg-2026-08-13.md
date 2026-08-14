# R32 — query-alias vocabulary bridging · PREREG (2026-08-13)

> **Renumbering note:** registered under the number "R30"/"R31" on a
> parallel branch; renumbered R32/R33 on rebase after main's same-numbered
> rows became visible. Content otherwise unchanged.


**$0, retrieval-only oracle, LoCoMo full N = 1438, `topk_fts`, R19 labels.
Registered and committed before any arm runs.**

R22 closed the composition family and named this "the only remaining $0
lever": `query_aliases` is a shipped deterministic channel
(`SPECTRAL_QUERY_ALIASES` → `expand_aliases` in `fts_query_words`, additive
terms, same shape as number bridging) that has **never been tested**.

## The question

Can a **consumer-curable, generic-English** alias table recover missed
evidence by bridging vocabulary — or is the zero-overlap failure family
inference-shaped, where no word↔word table can reach?

Diagnostic already in hand (`diagnose_lexical_misses.py` on the fresh N=400
baseline): 119 missed evidence turns in zero-evidence questions — **61.3%
share zero content words** with their question, 35.3% share one. Reading the
zero-overlap examples, most look like inference gaps ("problems before
adopting Toby" ↔ "contacting landlords"), not synonym gaps. That is the
shape this experiment measures against.

## Design — split-half by conversation

LoCoMo has 10 conversations. **Derivation half: conversations 0,2,4,6,8
(721 questions). Test half: 1,3,5,7,9 (717 questions).** The split is by
conversation, not by question, because vocabulary is conversation-specific
and the honest claim is about a table written without seeing the questions
it is scored on.

1. **A0 (baseline):** current defaults, full N, `topk_fts`, binary
   `fa5763d`. Precondition: its first-400 rows must reproduce the
   2026-08-13 tiebreak-verification `base_topk` rows (context_hash
   identical) — same binary, same dataset, so any diff voids the run.
2. **Table authoring (the fitted step, done openly):** I read the
   derivation-half misses and write `aliases.json` under these constraints,
   fixed now:
   - keys and expansions are **common English words only — no proper nouns,
     no numbers** (numbers are the shipped `number_normalize` lever);
   - every pair must be a defensible synonym/near-synonym/hypernym out of
     context ("pup"↔"dog", "home"↔"apartment"), the kind a consumer could
     ship blind;
   - every entry is listed in the result doc with the derivation example
     that motivated it;
   - **nothing from test-half questions or turns is consulted.** The
     derivation-half rows are the only miss data read during authoring.
3. **B (alias arm):** identical to A0 plus `SPECTRAL_QUERY_ALIASES=
   aliases.json`. Single variable.

## Gates, fixed before any arm

**PRIMARY (the honest number): test-half paired evidence-turn recall,
B vs A0.** PASS requires **both**: two-sided Wilcoxon on per-question
evidence-turn counts p < 0.05 **and** ≥ +2.0pp micro evidence-turn recall
on the test half. (Same two-clause form as R22–R24; the effect-size clause
is what kept A3′ honest in R26.)

**SECONDARY (reported, no verdict): derivation-half delta** — a
fitted-table number, closer to a ceiling than a forecast, and it will be
labelled as such. If the derivation half itself moves < +2.0pp, that is the
stronger statement: even fitting cannot make the lever work.

Regression accounting: per-question negative movements are reported with the
same prominence as positive (aliases are global; polluting unrelated queries
is the known risk — R23's dilution concern, which R24 showed can go either
way).

## Prediction, recorded before running

Test half **fails the gate, under +1.0pp**. Reasoning: (a) the zero-overlap
bucket looks inference-dominated on inspection; (b) bridging converts a
0-shared turn into a 1-shared turn, and the 1-shared bucket is already 35.3%
of misses — admission at one term is mostly not retrieval; (c) six lexical
levers and the composition all failed on this corpus. The register's
prediction record is mixed (R25 right, same-day mechanism wrong), which is
why this is written down now.

## What does NOT follow, regardless of outcome

- No accuracy claim (retrieval only, no end-to-end arm).
- No cascade claim (`recall_cascade` unmeasured here; transfer would need
  its own run, only justified on a PASS).
- No default change: `query_aliases_path` stays `None` everywhere; the env
  lever is bench-scoped opt-in.
- A FAIL does not condemn consumer-curated aliases for *consumer* corpora
  (project jargon, product names) — it measures generic-English bridging on
  LoCoMo's failure population only.

## Environment

Same host and apparatus as `tiebreak-paired-verification-result-2026-08-13.md`
(Intel Mac, regenerated dataset matching R19 membership 1438/2140, binary
`fa5763d`). Arms archived under `~/spectral-local-bench/r30-aliases-2026-08-13/`.
